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

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

mod structure;
mod text;

#[cfg(feature = "ocr")]
pub use paginaverto_ocr::{MODEL_BASE_URL, MODELS, ModelFile};

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
        detection: Vec<u8>,
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
    pub title: Option<String>,
    pub blocks: Vec<Block>,
}

/// One piece of chapter content.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Block {
    Paragraph(String),
    Figure(Figure),
}

/// An image from the PDF, as bytes ready to embed: a JPEG verbatim, or raw
/// pixels re-encoded to PNG.
#[derive(Clone)]
#[non_exhaustive]
pub struct Figure {
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
    /// OCR ran and found no readable text.
    OcrFoundNothing,
    /// A text layer is present but no page decoded (fonts it cannot read).
    UndecodableFonts,
    /// A text layer is present but yielded no text.
    NoText,
    /// The decoded text is mostly non-letters: the fonts did not decode.
    QualityGate,
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
            Error::OcrFoundNothing => f.write_str(
                "OCR found no readable text in this scan — the page images may be \
                 too low-quality, or in a format we can't decode (e.g. JPEG2000)",
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
    match catch_unwind(AssertUnwindSafe(|| pipeline(input, opts))) {
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
    let doc = match input {
        Input::Path(p) => lopdf::Document::load(p),
        Input::Bytes(b) => lopdf::Document::load_mem(b),
    }
    .map_err(|e| Error::Open(e.to_string()))?;

    let kind = text::classify(&doc);
    let pages: Vec<text::Page> = match kind {
        text::Kind::Scan => scan_pages(&doc, opts)?,
        text::Kind::TextLayer => {
            let with_figures = text::is_born_digital(&doc);
            doc.get_pages()
                .into_iter()
                .map(|(num, id)| text::page(&doc, id, num, with_figures))
                .collect()
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

/// Real prose is about 80% letters. When at least 50 non-space characters
/// decoded and fewer than 45% of them are letters, the fonts did not decode.
fn fails_quality_gate(chapters: &[Chapter]) -> bool {
    let (mut nonspace, mut letters) = (0usize, 0usize);
    let paragraphs = chapters
        .iter()
        .flat_map(|c| &c.blocks)
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            Block::Figure(_) => None,
        });
    for ch in paragraphs.flat_map(|p| p.chars()) {
        if !ch.is_whitespace() {
            nonspace += 1;
            if ch.is_alphabetic() {
                letters += 1;
            }
        }
    }
    // PARITY: chapter titles are not counted.
    nonspace >= 50 && (letters as f32) < nonspace as f32 * 0.45
}

/// Recognise every page of a scan. Checks run in epublift's order: OCR
/// available, then the language, then the models.
#[cfg(feature = "ocr")]
fn scan_pages(doc: &lopdf::Document, opts: &Options) -> Result<Vec<text::Page>, Error> {
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
    Ok(doc
        .get_pages()
        .into_values()
        .map(|id| {
            // PARITY: a page whose image is missing, undecodable, or fails
            // recognition becomes an empty page, silently.
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
        .collect())
}

/// Built without OCR: a scan is refused.
#[cfg(not(feature = "ocr"))]
fn scan_pages(_doc: &lopdf::Document, _opts: &Options) -> Result<Vec<text::Page>, Error> {
    Err(Error::ScanNeedsOcr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn public_types_are_send() {
        assert_send::<Book>();
        assert_send::<Options>();
        assert_send::<Error>();
    }
}
