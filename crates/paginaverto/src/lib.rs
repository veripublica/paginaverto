//! paginaverto reads PDFs and turns them into a structured [`Document`]: its
//! metadata, its pages with where their text came from, and one flat list of
//! headings, paragraphs and figures in reading order.
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
//! let doc = read(Input::Path("book.pdf".as_ref()), &Options::default())?;
//! println!("{} chapters", doc.chapters().len());
//! for page in doc.pages.iter().filter(|p| !p.problems.is_empty()) {
//!     println!("page {}: {:?}", page.number, page.problems);
//! }
//! # Ok::<(), paginaverto::Error>(())
//! ```

#![warn(missing_docs)]

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use lopdf::ObjectId;
use parallel::Pool;

mod decode;
mod document;
mod images;
mod inspect;
mod limits;
mod outline;
mod parallel;
mod structure;
mod text;

pub use document::{
    Block, BlockBox, BlockKind, Chapter, Document, Figure, Metadata, Page, PageProblem, Run,
    SCHEMA_VERSION, TextSource,
};
pub use inspect::{FontInfo, Inspection, PageInfo, PageSource};
pub use limits::{LimitKind, Limits};
pub use outline::OutlineEntry;
pub use parallel::{Cancel, PageDone, Progress};

#[cfg(feature = "ocr")]
pub use paginaverto_ocr::{
    LATIN_RECOGNITION, LATIN_RECOGNITION_URL, MODEL_BASE_URL, MODELS, ModelFile, ONNX_MODELS,
};

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
    /// Content language, a BCP-47 tag such as `tr`, recorded as
    /// [`Document::language`]. Required for a scan (one that needs OCR).
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
                "this PDF looks like a scan with no text layer, so it needs OCR, \
                 which is off or not built in",
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

/// Read a PDF into a [`Document`].
///
/// A page that cannot be read does not fail the document: it is reported in
/// [`Page::problems`]. Whole documents are refused, as [`Error`] lists.
///
/// Never panics: a panic inside a dependency is caught and returned as
/// [`Error::Internal`].
pub fn read(input: Input<'_>, opts: &Options) -> Result<Document, Error> {
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

fn pipeline(input: Input<'_>, opts: &Options) -> Result<Document, Error> {
    let limits = &opts.limits;
    limits.check_file(input)?;
    let pool = Pool {
        threads: opts.threads,
        cancel: &opts.cancel,
    };
    if let Some(images) = images::open(input, limits)? {
        let pages = image_pages(&images, &pool, opts)?;
        return finish(text::Kind::Scan, pages, Metadata::default(), opts);
    }
    let doc = limits.load(input)?;
    limits.check(&doc)?;

    let page_list: Vec<_> = doc.get_pages().into_iter().collect();
    let kind = text::classify(&doc, &page_list, &pool)?;
    let pages: Vec<text::Page> = match kind {
        text::Kind::Scan => {
            // Before OCR's checks: nobody should fetch models for this.
            if let Some(formats) = text::undecodable_scan(&doc, limits.max_image_pixels) {
                return Err(Error::UndecodableImages(formats));
            }
            scan_pages(&doc, &page_list, &pool, opts)?
        }
        text::Kind::TextLayer => {
            let with_figures = text::is_born_digital(&doc);
            pool.map(&page_list, opts.progress.as_ref(), |num, id| {
                text::page(&doc, id, num, with_figures, limits.max_image_pixels)
            })?
        }
    };

    finish(kind, pages, document::metadata(&doc), opts)
}

/// The pages read, as a document, or the reason there is none.
fn finish(
    kind: text::Kind,
    pages: Vec<text::Page>,
    metadata: Metadata,
    opts: &Options,
) -> Result<Document, Error> {
    let marks = outline::chapter_marks(&metadata.outline);
    let built = structure::build(&pages, marks.as_deref());
    if !built.has_content {
        return Err(match kind {
            text::Kind::Scan => Error::OcrFoundNothing,
            text::Kind::TextLayer if pages.iter().all(|p| p.blocks.is_empty()) => {
                Error::UndecodableFonts
            }
            text::Kind::TextLayer => Error::NoText,
        });
    }
    if built.fails_quality_gate {
        return Err(Error::QualityGate);
    }
    let language = opts
        .language
        .clone()
        .filter(|l| !l.is_empty())
        .or_else(|| metadata.catalog_lang.clone());
    Ok(Document {
        language,
        metadata,
        pages: pages
            .into_iter()
            .map(|p| Page {
                number: p.number,
                width: p.size.0,
                height: p.size.1,
                text_source: p.source,
                problems: p.problems,
            })
            .collect(),
        blocks: built.blocks,
    })
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

/// Threads recognising pages, unless the caller chose a number. The models
/// run on rten's own pool, so more only costs memory: on a 180-page scan, 8
/// threads took as long as 4 with 44% more memory.
#[cfg(feature = "ocr")]
const OCR_THREADS: usize = 4;

/// The OCR engine, once the checks pass, in epublift's order: OCR available,
/// then the language, then the models.
#[cfg(feature = "ocr")]
fn engine(opts: &Options) -> Result<paginaverto_ocr::Engine, Error> {
    let Ocr::Models {
        detection,
        recognition,
    } = &opts.ocr
    else {
        return Err(Error::ScanNeedsOcr);
    };
    // Required, though the recognition does not use it.
    if opts.language.as_deref().is_none_or(str::is_empty) {
        return Err(Error::LanguageRequired);
    }
    paginaverto_ocr::Engine::new(detection.clone(), recognition.clone())
        .map_err(|e| Error::Models(e.to_string()))
}

/// Recognise every page of a scan.
#[cfg(feature = "ocr")]
fn scan_pages(
    doc: &lopdf::Document,
    pages: &[(u32, ObjectId)],
    pool: &Pool<'_>,
    opts: &Options,
) -> Result<Vec<text::Page>, Error> {
    let engine = engine(opts)?;
    let max_pixels = opts.limits.max_image_pixels;
    let pool = pool.at_most(OCR_THREADS);
    pool.map(pages, opts.progress.as_ref(), |number, id| {
        let frame = text::Frame::of(doc, id);
        // A page without an image has nothing to read. One whose image cannot
        // be decoded is reported.
        let Some(image) = text::largest_image(doc, id) else {
            return text::Page {
                number,
                size: frame.size(),
                ..text::Page::default()
            };
        };
        match decode::gray(doc, image, max_pixels) {
            Some(gray) => recognised(&engine, number, frame, &gray),
            None => text::Page {
                number,
                size: frame.size(),
                problems: vec![PageProblem::UndecodableImage(text::format_name(image))],
                ..text::Page::default()
            },
        }
    })
}

/// Recognise every page image.
#[cfg(feature = "ocr")]
fn image_pages(
    images: &images::Images,
    pool: &Pool<'_>,
    opts: &Options,
) -> Result<Vec<text::Page>, Error> {
    let engine = engine(opts)?;
    let max_pixels = opts.limits.max_image_pixels;
    let pool = pool.at_most(OCR_THREADS);
    // The pool walks PDF pages; an image page's index stands in for the object.
    let list: Vec<(u32, ObjectId)> = (0..images.pages.len())
        .map(|i| (i as u32 + 1, (i as u32, 0)))
        .collect();
    pool.map(&list, opts.progress.as_ref(), |number, (i, _)| match images
        .picture(i as usize, max_pixels)
    {
        Some(picture) => {
            let (w, h) = picture.points();
            recognised(&engine, number, text::Frame::sized(w, h), &picture.gray)
        }
        None => text::Page {
            number,
            problems: vec![PageProblem::UndecodableImage(
                images.kind(i as usize).map(images::Kind::name),
            )],
            ..text::Page::default()
        },
    })
}

/// One page image through OCR. A failure is reported on the page.
#[cfg(feature = "ocr")]
fn recognised(
    engine: &paginaverto_ocr::Engine,
    number: u32,
    frame: text::Frame,
    gray: &decode::Gray,
) -> text::Page {
    let mut page = text::Page {
        number,
        size: frame.size(),
        ..text::Page::default()
    };
    {
        let Ok(lines) = engine.recognise_gray(gray.width, gray.height, &gray.pixels) else {
            page.problems.push(PageProblem::OcrFailed);
            return page;
        };
        let boxed: Vec<_> = lines
            .iter()
            .filter_map(|line| {
                let clean = text::squash(&line.text);
                let r = line.bbox?;
                (!clean.is_empty()).then_some((clean, (r.x, r.y, r.width, r.height)))
            })
            .collect();
        page.blocks = text::blocks_from_ocr(
            &boxed,
            number,
            &frame,
            (i64::from(gray.width), i64::from(gray.height)),
        );
        if !page.blocks.is_empty() {
            page.source = TextSource::Ocr;
        }
    }
    page
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

/// Built without OCR: page images are refused, as a scan is.
#[cfg(not(feature = "ocr"))]
fn image_pages(
    _images: &images::Images,
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
        assert_send::<Document>();
        assert_send::<Inspection>();
        assert_send::<Options>();
        assert_send::<Error>();
        assert_send::<Cancel>();
        assert_send::<Progress>();
    }
}
