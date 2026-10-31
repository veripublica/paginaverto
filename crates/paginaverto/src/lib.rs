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

fn pipeline(_input: Input<'_>, _opts: &Options) -> Result<Book, Error> {
    // Step 2 of PLAN-0.1.0: the text path and structure.
    Err(Error::Internal("not implemented yet".into()))
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
