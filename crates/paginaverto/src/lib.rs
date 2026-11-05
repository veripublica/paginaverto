//! paginaverto reads PDFs and turns them into a book: chapters of paragraphs
//! and figures.
//!
//! **0.1.0 is a parity release.** It does exactly what epublift's PDF import
//! did in cli-v4.5.1 (same chapters, paragraphs, figures and refusals), so
//! that epublift can switch to it. Its known faults are kept on purpose and
//! fixed from 0.2.0 on, each one measured.
//!
//! Three kinds of PDF:
//! 1. born-digital, with a text layer: the text is extracted directly;
//! 2. a scan with an embedded OCR text layer: that layer is reused;
//! 3. a pure scan: the page images go through character recognition (the
//!    `ocr` feature, with models the caller supplies).
//!
//! ```no_run
//! use paginaverto::{Input, Options, read};
//!
//! let book = read(Input::Path("book.pdf".as_ref()), &Options::default())?;
//! println!("{} chapters", book.chapters.len());
//! # Ok::<(), paginaverto::Error>(())
//! ```

#![warn(missing_docs)]

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use lopdf::ObjectId;
use parallel::Pool;

mod inspect;
mod limits;
mod parallel;
mod structure;
mod text;

pub use inspect::{FontInfo, Inspection, PageInfo, PageSource};
pub use limits::{LimitKind, Limits};
pub use parallel::{Cancel, PageDone, Progress};

#[cfg(feature = "ocr")]
pub use paginaverto_ocr::{MODEL_BASE_URL, MODELS, ModelFile, ONNX_MODELS};

/// Where the PDF comes from.
#[derive(Debug, Clone, Copy)]
pub enum Input<'a> {
    /// A file on disk.
    Path(&'a Path),
    /// The whole file in memory.
    Bytes(&'a [u8]),
}

/// How to read the PDF.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Options {
    /// Content language, a BCP-47 tag such as `tr`. Required for a scan
    /// (one that needs OCR); otherwise unused in 0.1.0.
    pub language: Option<String>,
    /// Whether scans may be recognised, and with which models.
    pub ocr: Ocr,
    /// What a document may cost before it is refused.
    pub limits: Limits,
    /// How many threads read pages. `None`: every core but two, and at least
    /// one, and no more than four for OCR, where more does not help. The output
    /// does not depend on it. OCR's models also run on a pool of their own
    /// (rten's: the machine's performance cores, or `RTEN_NUM_THREADS`), which
    /// this does not set.
    pub threads: Option<usize>,
    /// Stops a read from another thread.
    pub cancel: Cancel,
    /// Told about each page as it finishes.
    pub progress: Option<Progress>,
}

/// Character recognition for scans with no text layer.
#[derive(Clone, Default)]
pub enum Ocr {
    /// No recognition: a scan is refused with [`Error::ScanNeedsOcr`].
    #[default]
    Off,
    /// Recognise with these ocrs models (the bytes of
    /// `text-detection.rten` and `text-recognition.rten`). paginaverto never
    /// downloads them; the caller does. Needs the `ocr` feature.
    Models {
        /// The bytes of `text-detection.rten`.
        detection: Vec<u8>,
        /// The bytes of `text-recognition.rten`.
        recognition: Vec<u8>,
    },
}

impl fmt::Debug for Ocr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ocr::Off => f.write_str("Off"),
            Ocr::Models {
                detection,
                recognition,
            } => f
                .debug_struct("Models")
                .field("detection", &format_args!("{} bytes", detection.len()))
                .field("recognition", &format_args!("{} bytes", recognition.len()))
                .finish(),
        }
    }
}

/// What a PDF turned into.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Book {
    /// In reading order. Never empty: an empty result is an [`Error`].
    pub chapters: Vec<Chapter>,
}

/// An optional heading and its content, in reading order.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Chapter {
    /// The heading that starts the chapter. `None` for the content before
    /// the first heading.
    pub title: Option<String>,
    /// Paragraphs and figures, in reading order. A page's figures come after
    /// its text.
    pub blocks: Vec<Block>,
}

/// One piece of chapter content.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Block {
    /// A paragraph's text, whitespace collapsed to single spaces.
    Paragraph(String),
    /// An image.
    Figure(Figure),
}

/// An image from the PDF, as bytes ready to embed: a JPEG verbatim, or raw
/// pixels re-encoded to PNG.
#[derive(Clone)]
#[non_exhaustive]
pub struct Figure {
    /// The image file's bytes.
    pub data: Vec<u8>,
    /// `image/jpeg` or `image/png`.
    pub media_type: &'static str,
    /// `jpg` or `png`.
    pub ext: &'static str,
}

impl fmt::Debug for Figure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Figure")
            .field("data", &format_args!("{} bytes", self.data.len()))
            .field("media_type", &self.media_type)
            .finish()
    }
}

/// Why a PDF could not be read. The caller words the message for its user;
/// `Display` gives a plain English one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The file could not be read or parsed as a PDF.
    Open(String),
    /// A scan with no text layer, and OCR is off or not built in.
    ScanNeedsOcr,
    /// A scan needs OCR, and OCR needs [`Options::language`].
    LanguageRequired,
    /// The OCR models could not be loaded.
    Models(String),
    /// A scan whose page images are all in formats this build cannot decode;
    /// OCR could not read them, so it is not tried. The formats' names, in
    /// the order they were met.
    UndecodableImages(Vec<String>),
    /// OCR ran and found no readable text.
    OcrFoundNothing,
    /// A text layer is present but no page decoded (fonts it cannot read).
    UndecodableFonts,
    /// A text layer is present but yielded no text.
    NoText,
    /// The decoded text is mostly non-letters: the fonts did not decode.
    QualityGate,
    /// The document went over one of the [`Limits`].
    LimitExceeded {
        /// Which limit.
        what: LimitKind,
        /// The limit's value.
        limit: u64,
    },
    /// [`Cancel::cancel`] was called.
    Cancelled,
    /// An internal failure (a panic in a dependency), caught at the boundary.
    Internal(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Open(e) => write!(f, "failed to open PDF: {e}"),
            Error::ScanNeedsOcr => f.write_str(
                "this PDF looks like a scan with no text layer — OCR is needed, \
                 which is not available yet (coming in a later release)",
            ),
            Error::LanguageRequired => {
                f.write_str("--language is required for scanned PDFs (e.g. --language tr)")
            }
            Error::Models(e) => write!(f, "could not load the OCR models: {e}"),
            Error::UndecodableImages(formats) => write!(
                f,
                "this scan's page images are in a format that can't be decoded yet: {}",
                formats.join(", ")
            ),
            Error::OcrFoundNothing => f.write_str(
                "OCR found no readable text in this scan — the page images may be \
                 too low-quality",
            ),
            Error::UndecodableFonts => f.write_str(
                "this PDF has a text layer, but its fonts can't be decoded yet \
                 (CID/Type0 — common in modern, non-Latin PDFs); support is coming \
                 in a later release",
            ),
            Error::NoText => f.write_str("no extractable text found"),
            Error::QualityGate => f.write_str(
                "couldn't decode this PDF's text — it likely uses fonts or a PDF \
                 structure we can't read yet (e.g. object streams); support is \
                 coming in a later release",
            ),
            Error::LimitExceeded { what, limit } => {
                write!(f, "this PDF goes over the limit of {limit} {what}")
            }
            Error::Cancelled => f.write_str("cancelled"),
            Error::Internal(e) => write!(f, "internal error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Read a PDF into a [`Book`].
///
/// Never panics: a panic inside a dependency is caught and returned as
/// [`Error::Internal`].
pub fn read(input: Input<'_>, opts: &Options) -> Result<Book, Error> {
    guarded(|| pipeline(input, opts))
}

/// Report what a PDF is made of, page by page, without reading it into a
/// document. Follows the same [`Limits`] as [`read`], and needs no OCR. Uses
/// `opts` for its limits, threads, cancel and progress.
///
/// Never panics, as [`read`] never does.
pub fn inspect(input: Input<'_>, opts: &Options) -> Result<Inspection, Error> {
    guarded(|| inspect::inspect(input, opts))
}

/// Turns a panic in a dependency into [`Error::Internal`].
fn guarded<T>(run: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(result) => result,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a dependency panicked".into());
            Err(Error::Internal(msg))
        }
    }
}

fn pipeline(input: Input<'_>, opts: &Options) -> Result<Book, Error> {
    let limits = &opts.limits;
    limits.check_file(input)?;
    let doc = limits.load(input)?;
    limits.check(&doc)?;

    let pool = Pool {
        threads: opts.threads,
        cancel: &opts.cancel,
    };
    let page_list: Vec<_> = doc.get_pages().into_iter().collect();
    let kind = text::classify(&doc, &page_list, &pool)?;
    let pages: Vec<text::Page> = match kind {
        text::Kind::Scan => {
            // Before OCR's checks: nobody should fetch models for this.
            if let Some(formats) = text::undecodable_scan(&doc) {
                return Err(Error::UndecodableImages(formats));
            }
            scan_pages(&doc, &page_list, &pool, opts)?
        }
        text::Kind::TextLayer => {
            let with_figures = text::is_born_digital(&doc);
            pool.map(&page_list, opts.progress.as_ref(), |num, id| {
                text::page(&doc, id, num, with_figures)
            })?
        }
    };

    let chapters = structure::chapters(&pages);
    if chapters.is_empty() {
        return Err(match kind {
            text::Kind::Scan => Error::OcrFoundNothing,
            text::Kind::TextLayer if pages.iter().all(|p| p.blocks.is_empty()) => {
                Error::UndecodableFonts
            }
            text::Kind::TextLayer => Error::NoText,
        });
    }
    if fails_quality_gate(&chapters) {
        return Err(Error::QualityGate);
    }
    Ok(Book { chapters })
}

/// An error's message followed by each of its causes, `: `-separated, as
/// `anyhow`'s `{:#}` prints a chain: "couldn't parse input: invalid file
/// header", not only the outer "couldn't parse input".
fn with_causes(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        let s = c.to_string();
        // Some errors repeat their source in their own message; skip those.
        if !msg.ends_with(&s) {
            msg.push_str(": ");
            msg.push_str(&s);
        }
        cause = c.source();
    }
    msg
}

/// Real prose is about 80% letters. When at least 50 non-space characters
/// decoded and fewer than 45% of them are letters, the fonts did not decode.
fn fails_quality_gate(chapters: &[Chapter]) -> bool {
    let (mut nonspace, mut letters) = (0usize, 0usize);
    let titles = chapters.iter().filter_map(|c| c.title.as_ref());
    let paragraphs = chapters
        .iter()
        .flat_map(|c| &c.blocks)
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            Block::Figure(_) => None,
        });
    for ch in titles.chain(paragraphs).flat_map(|p| p.chars()) {
        if !ch.is_whitespace() {
            nonspace += 1;
            if ch.is_alphabetic() {
                letters += 1;
            }
        }
    }
    nonspace >= 50 && (letters as f32) < nonspace as f32 * 0.45
}

/// Threads recognising pages, unless the caller chose a number. The models
/// run on rten's own pool, so more only costs memory: on a 180-page scan, 8
/// threads took as long as 4 with 44% more memory.
#[cfg(feature = "ocr")]
const OCR_THREADS: usize = 4;

/// Recognise every page of a scan. Checks run in epublift's order: OCR
/// available, then the language, then the models.
#[cfg(feature = "ocr")]
fn scan_pages(
    doc: &lopdf::Document,
    pages: &[(u32, ObjectId)],
    pool: &Pool<'_>,
    opts: &Options,
) -> Result<Vec<text::Page>, Error> {
    let Ocr::Models {
        detection,
        recognition,
    } = &opts.ocr
    else {
        return Err(Error::ScanNeedsOcr);
    };
    // Required, though 0.1.0 does not use it yet.
    if opts.language.as_deref().is_none_or(str::is_empty) {
        return Err(Error::LanguageRequired);
    }
    let engine = paginaverto_ocr::Engine::new(detection.clone(), recognition.clone())
        .map_err(|e| Error::Models(e.to_string()))?;
    let pool = pool.at_most(OCR_THREADS);
    pool.map(pages, opts.progress.as_ref(), |_, id| {
        // A page whose image is missing, undecodable, or fails recognition
        // becomes an empty page. The document format will report it.
        let lines = text::scan_image(doc, id)
            .and_then(|fig| engine.recognise(&fig.data).ok())
            .unwrap_or_default();
        text::Page {
            blocks: lines
                .iter()
                .map(|l| text::squash(&l.text))
                .filter(|l| !l.is_empty())
                .collect(),
            ..text::Page::default()
        }
    })
}

/// Built without OCR: a scan is refused.
#[cfg(not(feature = "ocr"))]
fn scan_pages(
    _doc: &lopdf::Document,
    _pages: &[(u32, ObjectId)],
    _pool: &Pool<'_>,
    _opts: &Options,
) -> Result<Vec<text::Page>, Error> {
    Err(Error::ScanNeedsOcr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn public_types_are_send() {
        assert_send::<Book>();
        assert_send::<Inspection>();
        assert_send::<Options>();
        assert_send::<Error>();
        assert_send::<Cancel>();
        assert_send::<Progress>();
    }
}
