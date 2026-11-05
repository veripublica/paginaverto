//! What a PDF is made of, page by page, without reading it into a document:
//! the answer to "why did this come out broken?".

use std::collections::BTreeMap;

use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::parallel::Pool;
use crate::text::{self, Kind};
use crate::{Error, Input, Options};

/// What [`inspect`](crate::inspect()) found.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Inspection {
    /// Whether [`read`](crate::read) would treat the document as a scan. It
    /// decides this once for the whole document, so with OCR every page goes
    /// through recognition, even one with a text layer.
    pub reads_as_scan: bool,
    /// Every page, in order.
    pub pages: Vec<PageInfo>,
    /// The fonts the pages show text in that have no usable `/ToUnicode` map.
    pub fonts: Vec<FontInfo>,
}

/// One page.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PageInfo {
    /// The page number, from 1.
    pub number: u32,
    /// The visible width in points, after the page's rotation. Zero if the
    /// page declares no size.
    pub width: f32,
    /// The visible height in points, after the page's rotation.
    pub height: f32,
    /// Where text on this page would come from.
    pub source: PageSource,
    /// On a [`PageSource::Scan`] page: the format of its image, if that is one
    /// this build cannot decode.
    pub undecodable_image: Option<&'static str>,
}

/// Where a page's text would come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PageSource {
    /// Text the PDF holds.
    TextLayer,
    /// A whole-page image with a recognised text layer over it.
    EmbeddedOcr,
    /// An image with no text: only OCR can read it.
    Scan,
    /// Nothing to read.
    Empty,
}

impl PageSource {
    /// The name used in reports.
    pub fn name(self) -> &'static str {
        match self {
            PageSource::TextLayer => "text_layer",
            PageSource::EmbeddedOcr => "embedded_ocr",
            PageSource::Scan => "scan",
            PageSource::Empty => "empty",
        }
    }
}

/// A font with no usable `/ToUnicode` map.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FontInfo {
    /// Its `/BaseFont`.
    pub name: String,
    /// Its `/Subtype`, such as `Type0` or `TrueType`.
    pub subtype: String,
    /// Whether its text can be read anyway, through its encoding. A composite
    /// font's cannot: its codes mean nothing without the map.
    pub readable: bool,
    /// The pages that show text in it.
    pub pages: Vec<u32>,
}

/// What one page contributes.
struct Seen<'a> {
    info: PageInfo,
    fonts: Vec<&'a Dictionary>,
}

pub(crate) fn inspect(input: Input<'_>, opts: &Options) -> Result<Inspection, Error> {
    let limits = &opts.limits;
    limits.check_file(input)?;
    let doc = limits.load(input)?;
    limits.check(&doc)?;

    let pool = Pool {
        threads: opts.threads,
        cancel: &opts.cancel,
    };
    let page_list: Vec<_> = doc.get_pages().into_iter().collect();
    let reads_as_scan = text::classify(&doc, &page_list, &pool)? == Kind::Scan;
    let seen = pool.map(&page_list, opts.progress.as_ref(), |number, id| {
        look_at_page(&doc, id, number)
    })?;

    let mut fonts: BTreeMap<(String, String), FontInfo> = BTreeMap::new();
    let mut pages = Vec::with_capacity(seen.len());
    for page in seen {
        for dict in page.fonts {
            let (name, subtype) = (font_field(dict, b"BaseFont"), font_field(dict, b"Subtype"));
            let readable = subtype != "Type0" && dict.get_font_encoding(&doc).is_ok();
            let font = fonts
                .entry((name.clone(), subtype.clone()))
                .or_insert_with(|| FontInfo {
                    name,
                    subtype,
                    readable,
                    pages: Vec::new(),
                });
            // Two resource names can point at one font.
            if font.pages.last() != Some(&page.info.number) {
                font.pages.push(page.info.number);
            }
        }
        pages.push(page.info);
    }
    Ok(Inspection {
        reads_as_scan,
        pages,
        fonts: fonts.into_values().collect(),
    })
}

fn look_at_page(doc: &Document, id: ObjectId, number: u32) -> Seen<'_> {
    let content = Content::decode(&doc.get_page_content(id)).ok();
    let shown = text::shown_bytes(doc, id);
    let shows_text = shown > 0;
    let full_page_image = text::has_full_page_image(doc, id);
    let source = if shown >= text::TEXT_FLOOR && full_page_image {
        PageSource::EmbeddedOcr
    } else if shows_text && !full_page_image {
        PageSource::TextLayer
    } else if text::largest_image(doc, id).is_some() {
        PageSource::Scan
    } else {
        PageSource::Empty
    };
    let (width, height) = visible_size(doc, id);
    let undecodable_image = (source == PageSource::Scan)
        .then(|| text::largest_image(doc, id).and_then(text::undecodable_format))
        .flatten();

    // Only fonts the content selects: a page's resources often list more.
    let used: Vec<&[u8]> = content
        .as_ref()
        .map(|c| {
            c.operations
                .iter()
                .filter(|op| op.operator == "Tf")
                .filter_map(|op| op.operands.first()?.as_name().ok())
                .collect()
        })
        .unwrap_or_default();
    let fonts = if shows_text {
        text::fonts(doc, id)
            .into_iter()
            .filter(|(name, _)| used.contains(&name.as_slice()))
            .filter(|(_, dict)| !text::has_usable_to_unicode(doc, dict))
            .map(|(_, dict)| dict)
            .collect()
    } else {
        Vec::new()
    };
    Seen {
        info: PageInfo {
            number,
            width,
            height,
            source,
            undecodable_image,
        },
        fonts,
    }
}

fn font_field(dict: &Dictionary, key: &[u8]) -> String {
    dict.get(key).and_then(Object::as_name).map_or_else(
        |_| String::new(),
        |n| String::from_utf8_lossy(n).into_owned(),
    )
}

/// The page's visible size: `/CropBox`, else `/MediaBox`, each inherited,
/// swapped by a quarter turn of `/Rotate`.
fn visible_size(doc: &Document, id: ObjectId) -> (f32, f32) {
    let rect = text::inherited(doc, id, b"CropBox")
        .and_then(|o| rect(doc, o))
        .or_else(|| text::inherited(doc, id, b"MediaBox").and_then(|o| rect(doc, o)));
    let Some([x0, y0, x1, y1]) = rect else {
        return (0.0, 0.0);
    };
    let (w, h) = ((x1 - x0).abs(), (y1 - y0).abs());
    let turned = text::inherited(doc, id, b"Rotate")
        .and_then(|o| o.as_i64().ok())
        .is_some_and(|r| r.rem_euclid(180) == 90);
    if turned { (h, w) } else { (w, h) }
}

fn rect(doc: &Document, o: &Object) -> Option<[f32; 4]> {
    let a = o.as_array().ok()?;
    let mut out = [0.0; 4];
    if a.len() != 4 {
        return None;
    }
    for (slot, v) in out.iter_mut().zip(a) {
        *slot = text::deref(doc, v)?.as_float().ok()?;
    }
    Some(out)
}
