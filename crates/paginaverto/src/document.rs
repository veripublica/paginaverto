//! What a PDF turns into: metadata, pages, and one flat list of blocks in
//! reading order.

use std::collections::BTreeMap;
use std::fmt;

use lopdf::Object;

use crate::outline::{self, OutlineEntry, text_string};
use crate::text;

/// The version of this document's shape, as the CLI's file states it. New
/// kinds and fields do not raise it; only a breaking change does.
pub const SCHEMA_VERSION: u32 = 1;

/// A PDF read into a document.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Document {
    /// The content's language: [`Options::language`](crate::Options), else the
    /// catalog's `/Lang`, else `None`. Never guessed.
    pub language: Option<String>,
    /// What the file says about itself.
    pub metadata: Metadata,
    /// Every page of the PDF, in order.
    pub pages: Vec<Page>,
    /// Everything read, in reading order. Never empty: an empty result is an
    /// [`Error`](crate::Error). A page's figures come after its text.
    pub blocks: Vec<Block>,
}

impl Document {
    /// The document as chapters: a new one at each heading of level 1, and one
    /// without a title for what comes before the first.
    pub fn chapters(&self) -> Vec<Chapter<'_>> {
        let mut out = Vec::new();
        let mut title = None;
        let mut start = 0;
        for (i, block) in self.blocks.iter().enumerate() {
            if block.level() == Some(1) {
                if title.is_some() || i > start {
                    out.push(Chapter {
                        title,
                        blocks: &self.blocks[start..i],
                    });
                }
                title = Some(block.text());
                start = i + 1;
            }
        }
        if title.is_some() || start < self.blocks.len() {
            out.push(Chapter {
                title,
                blocks: &self.blocks[start..],
            });
        }
        out
    }
}

/// A title and what follows it, up to the next heading of level 1.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Chapter<'a> {
    /// The heading that starts the chapter; `None` for the content before the
    /// first heading.
    pub title: Option<String>,
    /// The chapter's content, without its heading. Headings of deeper levels
    /// are among them.
    pub blocks: &'a [Block],
}

/// What the file says about itself, given raw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Metadata {
    /// The text entries of the file's `/Info` dictionary, such as `Title`.
    pub info: BTreeMap<String, String>,
    /// The XMP packet, if the catalog has one.
    pub xmp: Option<String>,
    /// The catalog's `/Lang`.
    pub catalog_lang: Option<String>,
    /// The PDF's outline (bookmarks), whatever it is worth: some give every
    /// page an entry named after its number.
    pub outline: Vec<OutlineEntry>,
}

/// One page of the PDF.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Page {
    /// The page number, from 1.
    pub number: u32,
    /// The visible width in points, after the page's rotation. Zero if the
    /// page declares no size.
    pub width: f32,
    /// The visible height in points, after the page's rotation.
    pub height: f32,
    /// Where the page's text came from.
    pub text_source: TextSource,
    /// What went wrong on this page. The page is then missing some or all of
    /// its text, and the document is still returned.
    pub problems: Vec<PageProblem>,
}

/// Where a page's text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextSource {
    /// Text the PDF holds, on a page with no whole-page image.
    TextLayer,
    /// A recognised text layer stored in the PDF over a whole-page image. It
    /// did not go through this crate's OCR, or its alphabet.
    EmbeddedOcr,
    /// This crate's OCR.
    Ocr,
    /// No text was read from the page.
    None,
}

impl TextSource {
    /// The name used in the CLI's file.
    pub fn name(self) -> &'static str {
        match self {
            TextSource::TextLayer => "text_layer",
            TextSource::EmbeddedOcr => "embedded_ocr",
            TextSource::Ocr => "ocr",
            TextSource::None => "none",
        }
    }
}

/// What went wrong on a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PageProblem {
    /// OCR ran on the page's image and failed.
    OcrFailed,
    /// The page's image is in a form this build cannot decode, named when it
    /// is a format of its own (`JBIG2`, for one).
    UndecodableImage(Option<&'static str>),
    /// A page with only a whole-page image, in a document read from its text
    /// layer: OCR does not run on such a document, so the image was not read.
    ImageNotRead,
}

impl PageProblem {
    /// The name used in the CLI's file.
    pub fn name(self) -> &'static str {
        match self {
            PageProblem::OcrFailed => "ocr_failed",
            PageProblem::UndecodableImage(_) => "undecodable_images",
            PageProblem::ImageNotRead => "image_not_read",
        }
    }
}

/// One piece of the document.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Block {
    /// Its name in this document: `b1`, `b2`, … in reading order.
    pub id: String,
    /// What it is.
    pub kind: BlockKind,
    /// Where it sits, one box per page it touches. Empty when the position is
    /// not known.
    pub boxes: Vec<BlockBox>,
    /// How sure the reading is, from 0 to 1. Always `None` so far: the OCR
    /// engine gives no figure for it, and a text layer has none.
    pub confidence: Option<f32>,
}

impl Block {
    /// A paragraph, for a caller's own tests and tools: what `read` gives is
    /// read, never built this way.
    pub fn paragraph(id: impl Into<String>, runs: Vec<Run>) -> Block {
        Block::new(id, BlockKind::Paragraph { runs })
    }

    /// A heading of this level, from 1.
    pub fn heading(id: impl Into<String>, level: u32, runs: Vec<Run>) -> Block {
        Block::new(id, BlockKind::Heading { level, runs })
    }

    /// A figure.
    pub fn figure(id: impl Into<String>, figure: Figure) -> Block {
        Block::new(id, BlockKind::Figure(figure))
    }

    fn new(id: impl Into<String>, kind: BlockKind) -> Block {
        Block {
            id: id.into(),
            kind,
            boxes: Vec::new(),
            confidence: None,
        }
    }

    /// The block's text; empty for a figure.
    pub fn text(&self) -> String {
        self.runs().iter().map(|r| r.text.as_str()).collect()
    }

    /// The block's text, piece by piece; empty for a figure.
    pub fn runs(&self) -> &[Run] {
        match &self.kind {
            BlockKind::Heading { runs, .. } | BlockKind::Paragraph { runs } => runs,
            BlockKind::Figure(_) => &[],
        }
    }

    /// A heading's level; `None` for anything else.
    pub fn level(&self) -> Option<u32> {
        match self.kind {
            BlockKind::Heading { level, .. } => Some(level),
            _ => None,
        }
    }
}

/// What a block is. A reader that does not know a kind can show its
/// [`Block::runs`] as a paragraph.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BlockKind {
    /// A heading. Level 1 starts a chapter.
    Heading {
        /// How deep it sits, from 1.
        level: u32,
        /// Its text.
        runs: Vec<Run>,
    },
    /// A paragraph, whitespace collapsed to single spaces.
    Paragraph {
        /// Its text.
        runs: Vec<Run>,
    },
    /// An image.
    Figure(Figure),
}

/// A stretch of text with one style.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Run {
    /// The text.
    pub text: String,
    /// Shown in an italic font. Text layers only; a reading through lopdf or
    /// OCR has no fonts to tell from.
    pub italic: bool,
}

impl Run {
    /// A run of text, italic or not.
    pub fn new(text: impl Into<String>, italic: bool) -> Run {
        Run {
            text: text.into(),
            italic,
        }
    }
}

/// Where a block sits on a page: in points, with the origin at the top left of
/// the visible page, after its rotation. For text, a box follows the font's
/// size rather than the drawn glyphs.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct BlockBox {
    /// The page number, from 1.
    pub page: u32,
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The width.
    pub width: f32,
    /// The height.
    pub height: f32,
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

impl Figure {
    /// A figure from an image file's bytes, for a caller's own tests and
    /// tools: `media_type` such as `image/png`, `ext` such as `png`.
    pub fn new(data: Vec<u8>, media_type: &'static str, ext: &'static str) -> Figure {
        Figure {
            data,
            media_type,
            ext,
        }
    }
}

impl fmt::Debug for Figure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Figure")
            .field("data", &format_args!("{} bytes", self.data.len()))
            .field("media_type", &self.media_type)
            .finish()
    }
}

/// Reads the file's own metadata.
pub(crate) fn metadata(doc: &lopdf::Document) -> Metadata {
    let mut info = BTreeMap::new();
    let entries = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| text::deref(doc, o))
        .and_then(|o| o.as_dict().ok());
    for (key, value) in entries.into_iter().flatten() {
        if let Some(value) = text::deref(doc, value).and_then(text_string) {
            info.insert(String::from_utf8_lossy(key).into_owned(), value);
        }
    }
    let catalog = doc.catalog().ok();
    let xmp = catalog
        .and_then(|c| c.get(b"Metadata").ok())
        .and_then(|o| text::deref(doc, o))
        .and_then(|o| o.as_stream().ok())
        .map(|s| {
            let bytes = s
                .decompressed_content()
                .unwrap_or_else(|_| s.content.clone());
            String::from_utf8_lossy(&bytes).into_owned()
        });
    let catalog_lang = catalog
        .and_then(|c| c.get(b"Lang").ok())
        .and_then(|o| text::deref(doc, o))
        .filter(|o| matches!(o, Object::String(..)))
        .and_then(text_string)
        .filter(|l| !l.is_empty());
    Metadata {
        info,
        xmp,
        catalog_lang,
        outline: outline::outline(doc),
    }
}
