//! The text layer: classify the document, then pull each page's text with
//! font size and position, and its images.
//!
//! Parity note (0.1.0): every rule here reproduces epublift cli-v4.5.1's
//! `src/pdf/extract.rs`, quirks included. The ones to fix later are marked
//! `PARITY:`; do not change them before 0.2.0, and measure when you do.

use std::collections::HashMap;

use lopdf::content::Content;
use lopdf::{Dictionary, Document, Encoding, Object, ObjectId, Stream};

use crate::Figure;

// ---------------------------------------------------------------------------
// The document's kind

/// Which path a document takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A real text layer: born-digital, or a scan with embedded OCR text.
    TextLayer,
    /// Image pages with (almost) no text layer: needs OCR.
    Scan,
}

/// A page counts as having text when its text-showing operators carry at
/// least this many string bytes.
const TEXT_FLOOR: usize = 100;

/// A text layer on at least half the pages makes the document `TextLayer`.
/// A document with no pages is a `Scan`.
pub(crate) fn classify(doc: &Document) -> Kind {
    let pages = doc.get_pages();
    if pages.is_empty() {
        return Kind::Scan;
    }
    let with_text = pages
        .values()
        .filter(|&&id| shown_bytes(doc, id) >= TEXT_FLOOR)
        .count();
    if with_text * 2 >= pages.len() {
        Kind::TextLayer
    } else {
        Kind::Scan
    }
}

/// Bytes of string operands to `Tj`, `'`, `"` and `TJ`, undecoded. Robust
/// where decoding fails (composite fonts), so it only tells presence.
fn shown_bytes(doc: &Document, page_id: ObjectId) -> usize {
    let Ok(content) = Content::decode(&doc.get_page_content(page_id)) else {
        return 0;
    };
    let mut n = 0;
    for op in &content.operations {
        match op.operator.as_str() {
            "Tj" | "'" | "\"" => {
                n += op
                    .operands
                    .iter()
                    .filter_map(|o| o.as_str().ok())
                    .map(<[u8]>::len)
                    .sum::<usize>();
            }
            "TJ" => {
                if let Some(Ok(arr)) = op.operands.first().map(Object::as_array) {
                    n += arr
                        .iter()
                        .filter_map(|o| o.as_str().ok())
                        .map(<[u8]>::len)
                        .sum::<usize>();
                }
            }
            _ => {}
        }
    }
    n
}

/// A born-digital document: fewer than half its pages carry a full-page
/// image. Its images are real figures; a scan's images are the pages.
pub(crate) fn is_born_digital(doc: &Document) -> bool {
    let pages = doc.get_pages();
    if pages.is_empty() {
        return false;
    }
    let scanned = pages
        .values()
        .filter(|&&id| has_full_page_image(doc, id))
        .count();
    scanned * 2 < pages.len()
}

/// Pixels above which an image counts as a whole page.
const FULL_PAGE_PIXELS: i64 = 400_000;

/// Does the page carry an image of about a page's worth of pixels?
fn has_full_page_image(doc: &Document, page_id: ObjectId) -> bool {
    // PARITY: reads only the page's own /Resources, not one inherited from
    // the page tree (unlike `resources`).
    let own = doc
        .get_object(page_id)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Resources").ok())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok());
    own.is_some_and(|res| images(doc, res).any(|s| pixel_area(s) > FULL_PAGE_PIXELS))
}

// ---------------------------------------------------------------------------
// One page's text

/// A page's text, ready for the structure step.
#[derive(Debug, Clone, Default)]
pub(crate) struct Page {
    /// Text blocks, about one per paragraph, whitespace-normalised.
    pub blocks: Vec<String>,
    /// No full-page image on this page: its typography is clean, so the
    /// font-size heading signal can be trusted.
    pub born_digital: bool,
    /// Letters-only texts shown in a notably large font (heading candidates).
    pub big_font: Vec<String>,
    /// The page's figures, in document order.
    pub figures: Vec<Figure>,
}

/// Read one text-layer page. `with_figures` is decided once per document
/// (see [`is_born_digital`]).
pub(crate) fn page(doc: &Document, page_id: ObjectId, page_num: u32, with_figures: bool) -> Page {
    let born_digital = !has_full_page_image(doc, page_id);
    let mut big_font = Vec::new();
    let blocks = if born_digital {
        // Both extractors run and the cleaner one wins: lopdf's runs words
        // together on some fonts, ours on others. Ours runs anyway, for the
        // font-size signal.
        let runs = runs(doc, page_id);
        big_font = big_font_texts(&runs);
        let ours = blocks_from_runs(&runs);
        let theirs = lopdf_blocks(doc, page_id, page_num);
        if glue_score(&theirs) <= glue_score(&ours) {
            theirs
        } else {
            ours
        }
    } else {
        // A scan with an OCR text layer: lopdf reads those layers well; ours
        // is the fallback when it finds nothing.
        let theirs = lopdf_blocks(doc, page_id, page_num);
        if theirs.is_empty() {
            blocks_from_runs(&runs(doc, page_id))
        } else {
            theirs
        }
    };
    let figures = if with_figures {
        figures(doc, page_id)
    } else {
        Vec::new()
    };
    Page {
        blocks,
        born_digital,
        big_font,
        figures,
    }
}

/// How badly text reads: the share (per mille) of letters in implausibly
/// long words (18+ letters: a missed space), then more letters is better.
/// Lower wins; no text at all loses to anything.
fn glue_score(blocks: &[String]) -> (u32, i64) {
    let (mut letters, mut glued) = (0usize, 0usize);
    for word in blocks.iter().flat_map(|b| b.split_whitespace()) {
        let n = word.chars().filter(|c| c.is_alphabetic()).count();
        letters += n;
        if n >= 18 {
            glued += n;
        }
    }
    if letters == 0 {
        return (u32::MAX, 0);
    }
    ((glued * 1000 / letters) as u32, -(letters as i64))
}

/// lopdf's own `extract_text`, one block per non-empty line. Not asked at
/// all on a page with a composite font lopdf cannot map to Unicode: it would
/// return the raw glyph codes as letters.
fn lopdf_blocks(doc: &Document, page_id: ObjectId, page_num: u32) -> Vec<String> {
    let unreadable = fonts(doc, page_id).iter().any(|(_, d)| {
        d.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Type0".as_slice())
            && !matches!(
                d.get_font_encoding(doc),
                Ok(Encoding::UnicodeMapEncoding(_))
            )
    });
    if unreadable {
        return Vec::new();
    }
    doc.extract_text(&[page_num])
        .unwrap_or_default()
        .lines()
        .map(squash)
        .filter(|l| !l.is_empty())
        .collect()
}

/// Collapse every run of whitespace to one space and trim.
pub(crate) fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Letters only, ASCII-lowercased, words single-spaced: for matching text
/// across pages and against large-font runs.
pub(crate) fn letters_only(s: &str) -> String {
    // PARITY: `to_ascii_lowercase` leaves non-ASCII capitals (İ, Ç, Ş, Ğ, Ö,
    // Ü, …) as they are, so Turkish running heads and headings match less
    // well. Fix from 0.2.0, measured.
    let mapped: String = s
        .chars()
        .map(|c| {
            if c.is_alphabetic() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    squash(&mapped)
}

// ---------------------------------------------------------------------------
// Positioned runs: our own reading of the content stream

/// One shown string with its rendered geometry.
#[derive(Debug, Clone)]
struct Run {
    text: String,
    /// Rendered size: text-space size × the matrix's vertical scale.
    font_size: f32,
    /// Device-space start x.
    x: f32,
    /// Device-space end x, from the glyphs' real widths.
    end_x: f32,
    y: f32,
}

/// A 2D affine matrix `[a b c d e f]`, PDF's row-vector convention.
type Mat = [f32; 6];
const IDENTITY: Mat = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` then `b`.
fn mul(a: Mat, b: Mat) -> Mat {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn translate(tx: f32, ty: f32) -> Mat {
    [1.0, 0.0, 0.0, 1.0, tx, ty]
}

fn num(o: &Object) -> f32 {
    o.as_float().unwrap_or(0.0)
}

fn mat(a: &[Object]) -> Mat {
    [
        num(&a[0]),
        num(&a[1]),
        num(&a[2]),
        num(&a[3]),
        num(&a[4]),
        num(&a[5]),
    ]
}

/// Walk the page's content stream with the graphics and text state, and
/// record every shown string.
fn runs(doc: &Document, page_id: ObjectId) -> Vec<Run> {
    let mut out = Vec::new();
    let Ok(content) = Content::decode(&doc.get_page_content(page_id)) else {
        return out;
    };
    let fonts = page_fonts(doc, page_id);

    let mut ctm = IDENTITY;
    let mut saved: Vec<Mat> = Vec::new();
    let mut tm = IDENTITY;
    let mut tlm = IDENTITY;
    let mut size = 0.0_f32;
    let mut leading = 0.0_f32;
    let mut font: Option<&Font> = None;

    // PARITY: showing text does not advance the text matrix, so every run
    // starts where its line (or last positioning operator) started.
    let show =
        |out: &mut Vec<Run>, tm: Mat, ctm: Mat, size: f32, font: Option<&Font>, bytes: &[u8]| {
            let (text, adv) = decode(font, bytes);
            push(out, tm, ctm, size, text, adv);
        };

    for op in &content.operations {
        let a = &op.operands;
        match op.operator.as_str() {
            "q" => saved.push(ctm),
            "Q" => {
                if let Some(m) = saved.pop() {
                    ctm = m;
                }
            }
            "cm" if a.len() == 6 => ctm = mul(mat(a), ctm),
            "BT" => {
                tm = IDENTITY;
                tlm = IDENTITY;
            }
            "Tf" if a.len() == 2 => {
                size = num(&a[1]);
                font = a[0].as_name().ok().and_then(|n| fonts.get(n));
            }
            "Td" if a.len() == 2 => {
                tlm = mul(translate(num(&a[0]), num(&a[1])), tlm);
                tm = tlm;
            }
            "TD" if a.len() == 2 => {
                leading = -num(&a[1]);
                tlm = mul(translate(num(&a[0]), num(&a[1])), tlm);
                tm = tlm;
            }
            "Tm" if a.len() == 6 => {
                tm = mat(a);
                tlm = tm;
            }
            "TL" if a.len() == 1 => leading = num(&a[0]),
            "T*" => {
                tlm = mul(translate(0.0, -leading), tlm);
                tm = tlm;
            }
            "Tj" if !a.is_empty() => {
                if let Ok(bytes) = a[0].as_str() {
                    show(&mut out, tm, ctm, size, font, bytes);
                }
            }
            "'" if !a.is_empty() => {
                tlm = mul(translate(0.0, -leading), tlm);
                tm = tlm;
                if let Ok(bytes) = a[0].as_str() {
                    show(&mut out, tm, ctm, size, font, bytes);
                }
            }
            "\"" if a.len() == 3 => {
                tlm = mul(translate(0.0, -leading), tlm);
                tm = tlm;
                if let Ok(bytes) = a[2].as_str() {
                    show(&mut out, tm, ctm, size, font, bytes);
                }
            }
            "TJ" if !a.is_empty() => {
                if let Ok(arr) = a[0].as_array() {
                    let mut text = String::new();
                    let mut adv = 0.0_f32;
                    for el in arr {
                        match el {
                            Object::String(bytes, _) => {
                                let (t, w) = decode(font, bytes);
                                text.push_str(&t);
                                adv += w;
                            }
                            // A positive number moves the next glyph left.
                            Object::Integer(_) | Object::Real(_) => adv -= num(el) / 1000.0,
                            _ => {}
                        }
                    }
                    push(&mut out, tm, ctm, size, text, adv);
                }
            }
            _ => {}
        }
    }
    out
}

/// Record a run at the current text position. `adv_em` is its advance in
/// em units.
fn push(out: &mut Vec<Run>, tm: Mat, ctm: Mat, size: f32, text: String, adv_em: f32) {
    if text.trim().is_empty() {
        return;
    }
    let m = mul(tm, ctm);
    let yscale = (m[2] * m[2] + m[3] * m[3]).sqrt();
    let xscale = (m[0] * m[0] + m[1] * m[1]).sqrt();
    out.push(Run {
        text,
        font_size: size * yscale,
        x: m[4],
        end_x: m[4] + adv_em * size * xscale,
        y: m[5],
    });
}

/// Paragraph blocks from positioned runs: lines clustered by y, runs joined
/// by their x gaps, lines merged into blocks at a large vertical gap or a
/// first-line indent.
fn blocks_from_runs(runs: &[Run]) -> Vec<String> {
    let mut runs: Vec<&Run> = runs.iter().filter(|r| !r.text.trim().is_empty()).collect();
    if runs.is_empty() {
        return Vec::new();
    }
    // Top to bottom. Stable, so equal y keeps stream order.
    runs.sort_by(|a, b| b.y.total_cmp(&a.y));

    struct Line {
        y: f32,
        x: f32,
        font: f32,
        text: String,
    }
    let mut lines: Vec<Line> = Vec::new();
    let mut i = 0;
    while i < runs.len() {
        let y0 = runs[i].y;
        let tol = runs[i].font_size.max(1.0) * 0.6;
        let mut group: Vec<&Run> = Vec::new();
        while i < runs.len() && (y0 - runs[i].y).abs() <= tol {
            group.push(runs[i]);
            i += 1;
        }
        group.sort_by(|a, b| a.x.total_cmp(&b.x));
        // Many PDFs place each glyph or word as its own run; a space goes
        // only where the gap from the previous run's real end exceeds 0.2 em.
        let mut text = String::new();
        let mut prev_end: Option<f32> = None;
        for r in &group {
            let t = r.text.trim();
            if t.is_empty() {
                continue;
            }
            if prev_end.is_some_and(|pe| r.x - pe > r.font_size * 0.2) {
                text.push(' ');
            }
            text.push_str(t);
            prev_end = Some(r.end_x);
        }
        let text = squash(&text);
        if text.is_empty() {
            continue;
        }
        lines.push(Line {
            y: y0,
            x: group.iter().map(|r| r.x).fold(f32::INFINITY, f32::min),
            font: group.iter().map(|r| r.font_size).fold(0.0, f32::max),
            text,
        });
    }

    let mut gaps: Vec<f32> = lines
        .windows(2)
        .map(|w| w[0].y - w[1].y)
        .filter(|g| *g > 0.0)
        .collect();
    gaps.sort_by(f32::total_cmp);
    let median_gap = gaps.get(gaps.len() / 2).copied().unwrap_or(0.0);
    let left = lines.iter().map(|l| l.x).fold(f32::INFINITY, f32::min);

    let mut blocks = Vec::new();
    let mut cur = String::new();
    let mut prev_y: Option<f32> = None;
    for l in &lines {
        let new_block = cur.is_empty()
            || prev_y.is_some_and(|py| median_gap > 0.0 && py - l.y > median_gap * 1.6)
            || l.x > left + l.font;
        if new_block {
            if !cur.trim().is_empty() {
                blocks.push(cur.trim().to_string());
            }
            cur = l.text.clone();
        } else {
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(&l.text);
        }
        prev_y = Some(l.y);
    }
    if !cur.trim().is_empty() {
        blocks.push(cur.trim().to_string());
    }
    blocks
}

/// Letters-only texts of runs at least 1.3× the page's median font size.
fn big_font_texts(runs: &[Run]) -> Vec<String> {
    if runs.is_empty() {
        return Vec::new();
    }
    let mut sizes: Vec<f32> = runs.iter().map(|r| r.font_size).collect();
    sizes.sort_by(f32::total_cmp);
    let median = sizes[sizes.len() / 2].max(1.0);
    runs.iter()
        .filter(|r| r.font_size >= median * 1.3)
        .map(|r| letters_only(&r.text))
        .filter(|s| s.len() > 2)
        .collect()
}

// ---------------------------------------------------------------------------
// Fonts

/// How a font's shown bytes become Unicode.
enum Decoder<'a> {
    /// A simple (1-byte) font, through lopdf's encodings.
    Simple(Encoding<'a>),
    /// A font with a `/ToUnicode` CMap, parsed here.
    Cmap {
        map: HashMap<u32, String>,
        code_len: usize,
    },
}

/// A page font: its decoder and per-code glyph widths in em.
struct Font<'a> {
    decoder: Decoder<'a>,
    widths: HashMap<u32, f32>,
    default_width: f32,
}

/// The page's fonts by resource name. A font that is neither decodable by
/// CMap nor by lopdf's encodings is left out (its text then decodes as
/// UTF-8, lossily).
fn page_fonts<'a>(doc: &'a Document, page_id: ObjectId) -> HashMap<Vec<u8>, Font<'a>> {
    let mut out = HashMap::new();
    for (name, dict) in fonts(doc, page_id) {
        let subtype = dict
            .get(b"Subtype")
            .and_then(Object::as_name)
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        // The font's own /ToUnicode is authoritative. A simple font reads one
        // byte per code whatever the CMap declares; Type0 uses its width.
        let decoder = if let Some((map, inferred)) = to_unicode(doc, dict) {
            let code_len = if subtype == "Type0" { inferred } else { 1 };
            Decoder::Cmap { map, code_len }
        } else if let Ok(enc) = dict.get_font_encoding(doc) {
            Decoder::Simple(enc)
        } else {
            continue;
        };
        let (widths, default_width) = widths(doc, dict, &subtype);
        out.insert(
            name,
            Font {
                decoder,
                widths,
                default_width,
            },
        );
    }
    out
}

/// Shown bytes to (text, advance in em).
fn decode(font: Option<&Font>, bytes: &[u8]) -> (String, f32) {
    let Some(font) = font else {
        return (
            String::from_utf8_lossy(bytes).into_owned(),
            bytes.len() as f32 * 0.5,
        );
    };
    let width = |code: u32| {
        font.widths
            .get(&code)
            .copied()
            .unwrap_or(font.default_width)
    };
    match &font.decoder {
        Decoder::Simple(enc) => {
            let text = enc
                .bytes_to_string(bytes)
                .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned());
            (text, bytes.iter().map(|&b| width(u32::from(b))).sum())
        }
        Decoder::Cmap { map, code_len } => {
            let n = (*code_len).max(1);
            let mut text = String::new();
            let mut adv = 0.0_f32;
            // PARITY: a trailing partial code is dropped.
            for chunk in bytes.chunks_exact(n) {
                let code = chunk.iter().fold(0u32, |c, &b| (c << 8) | u32::from(b));
                if let Some(s) = map.get(&code) {
                    text.push_str(s);
                }
                adv += width(code);
            }
            (text, adv)
        }
    }
}

/// Per-code widths (em) and the default width: `/Widths` for a simple font,
/// the descendant's `/W` and `/DW` for Type0.
fn widths(doc: &Document, dict: &Dictionary, subtype: &str) -> (HashMap<u32, f32>, f32) {
    let mut widths = HashMap::new();
    if subtype != "Type0" {
        let first = dict
            .get(b"FirstChar")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0);
        if let Some(arr) = dict
            .get(b"Widths")
            .ok()
            .and_then(|o| deref(doc, o))
            .and_then(|o| o.as_array().ok())
        {
            for (i, w) in arr.iter().enumerate() {
                if let Ok(w) = w.as_float() {
                    widths.insert((first + i as i64) as u32, w / 1000.0);
                }
            }
        }
        return (widths, 0.5);
    }

    let Some(desc) = dict
        .get(b"DescendantFonts")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_array().ok())
        .and_then(|a| a.first())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
    else {
        return (widths, 0.5);
    };
    let dw = desc
        .get(b"DW")
        .ok()
        .and_then(|o| o.as_float().ok())
        .unwrap_or(1000.0)
        / 1000.0;
    if let Some(w) = desc
        .get(b"W")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_array().ok())
    {
        // Two forms: `c [w1 w2 …]` and `c_first c_last w`.
        let mut i = 0;
        while i < w.len() {
            let Ok(c) = w[i].as_i64() else { break };
            if let Some(arr) = w.get(i + 1).and_then(|o| o.as_array().ok()) {
                for (j, wv) in arr.iter().enumerate() {
                    if let Ok(wf) = wv.as_float() {
                        widths.insert(c as u32 + j as u32, wf / 1000.0);
                    }
                }
                i += 2;
            } else if let (Some(last), Some(wv)) = (
                w.get(i + 1).and_then(|o| o.as_i64().ok()),
                w.get(i + 2).and_then(|o| o.as_float().ok()),
            ) {
                for code in c..=last.min(c + 65535) {
                    widths.insert(code as u32, wv / 1000.0);
                }
                i += 3;
            } else {
                break;
            }
        }
    }
    (widths, dw)
}

/// The font's `/ToUnicode` CMap, parsed.
fn to_unicode(doc: &Document, font: &Dictionary) -> Option<(HashMap<u32, String>, usize)> {
    let stream = deref(doc, font.get(b"ToUnicode").ok()?)?.as_stream().ok()?;
    let content = stream
        .decompressed_content()
        .or_else(|_| stream.get_plain_content())
        .ok()?;
    parse_cmap(&content)
}

/// A minimal ToUnicode CMap parser: `bfchar` pairs `<src> <dst>` and
/// `bfrange` triples `<lo> <hi> <dst>` or `<lo> <hi> [<d>…]`, dst as UTF-16BE.
/// Also returns the code length in bytes, from the widest src token (two hex
/// digits a byte); 2 when no src was seen. None for an empty map.
fn parse_cmap(content: &[u8]) -> Option<(HashMap<u32, String>, usize)> {
    #[derive(Debug)]
    enum Tok {
        Hex(String),
        Open,
        Close,
        Word(String),
    }
    let s = String::from_utf8_lossy(content);
    let b = s.as_bytes();
    let mut toks: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'<' => {
                let end = b[i + 1..]
                    .iter()
                    .position(|&c| c == b'>')
                    .map_or(b.len(), |p| i + 1 + p);
                toks.push(Tok::Hex(s[i + 1..end].split_whitespace().collect()));
                i = end + 1;
            }
            b'[' => {
                toks.push(Tok::Open);
                i += 1;
            }
            b']' => {
                toks.push(Tok::Close);
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => {
                let end = b[i..]
                    .iter()
                    .position(|&c| c.is_ascii_whitespace() || matches!(c, b'<' | b'[' | b']'))
                    .map_or(b.len(), |p| i + p);
                toks.push(Tok::Word(s[i..end].to_string()));
                i = end;
            }
        }
    }

    let mut map = HashMap::new();
    let mut src_hex = 0usize;
    let mut k = 0;
    while k < toks.len() {
        match &toks[k] {
            Tok::Word(w) if w == "beginbfchar" => {
                k += 1;
                while let (Some(Tok::Hex(src)), Some(Tok::Hex(dst))) =
                    (toks.get(k), toks.get(k + 1))
                {
                    src_hex = src_hex.max(src.len());
                    if let (Some(code), Some(text)) = (hex_u32(src), hex_string(dst)) {
                        map.insert(code, text);
                    }
                    k += 2;
                }
            }
            Tok::Word(w) if w == "beginbfrange" => {
                k += 1;
                while k + 2 < toks.len() {
                    let (Tok::Hex(lo), Tok::Hex(hi)) = (&toks[k], &toks[k + 1]) else {
                        break;
                    };
                    src_hex = src_hex.max(lo.len());
                    let (Some(lo), Some(hi)) = (hex_u32(lo), hex_u32(hi)) else {
                        break;
                    };
                    k += 2;
                    match &toks[k] {
                        Tok::Hex(dst) => {
                            if let Some(units) = hex_units(dst) {
                                for (off, code) in (lo..=hi.min(lo + 65535)).enumerate() {
                                    let mut u = units.clone();
                                    if let Some(last) = u.last_mut() {
                                        *last = last.wrapping_add(off as u16);
                                    }
                                    map.insert(code, String::from_utf16_lossy(&u));
                                }
                            }
                            k += 1;
                        }
                        Tok::Open => {
                            k += 1;
                            let mut off = 0u32;
                            while k < toks.len() {
                                k += 1;
                                let Tok::Hex(d) = &toks[k - 1] else { break };
                                if let Some(t) = hex_string(d) {
                                    map.insert(lo + off, t);
                                }
                                off += 1;
                            }
                        }
                        _ => break,
                    }
                }
            }
            _ => k += 1,
        }
    }
    if map.is_empty() {
        return None;
    }
    let code_len = if src_hex == 0 { 2 } else { src_hex.div_ceil(2) };
    Some((map, code_len))
}

fn hex_u32(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim(), 16).ok()
}

/// A hex string as UTF-16 code units, four digits each.
fn hex_units(s: &str) -> Option<Vec<u16>> {
    let s = s.trim();
    if s.is_empty() || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    s.as_bytes()
        .chunks(4)
        .map(|c| u16::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok())
        .collect()
}

fn hex_string(s: &str) -> Option<String> {
    hex_units(s).map(|u| String::from_utf16_lossy(&u))
}

// ---------------------------------------------------------------------------
// Resources and images

fn deref<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
    match o {
        Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    }
}

/// The page's /Resources, inherited from the page tree if need be (at most
/// 16 levels up).
fn resources(doc: &Document, page_id: ObjectId) -> Option<&Dictionary> {
    let mut cur = doc.get_object(page_id).ok().and_then(|o| o.as_dict().ok());
    for _ in 0..16 {
        let dict = cur?;
        if let Some(res) = dict
            .get(b"Resources")
            .ok()
            .and_then(|o| deref(doc, o))
            .and_then(|o| o.as_dict().ok())
        {
            return Some(res);
        }
        cur = dict
            .get(b"Parent")
            .ok()
            .and_then(|p| deref(doc, p))
            .and_then(|o| o.as_dict().ok());
    }
    None
}

/// The page's font dictionaries by resource name.
fn fonts(doc: &Document, page_id: ObjectId) -> Vec<(Vec<u8>, &Dictionary)> {
    let Some(fonts) = resources(doc, page_id)
        .and_then(|r| r.get(b"Font").ok())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
    else {
        return Vec::new();
    };
    fonts
        .iter()
        .filter_map(|(name, v)| Some((name.clone(), deref(doc, v)?.as_dict().ok()?)))
        .collect()
}

/// The image XObjects of a resource dictionary, in its order. Only those
/// given by reference.
fn images<'a>(doc: &'a Document, res: &'a Dictionary) -> impl Iterator<Item = &'a Stream> {
    res.get(b"XObject")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
        .into_iter()
        .flat_map(|x| x.iter())
        .filter_map(move |(_, v)| match v {
            Object::Reference(id) => doc.get_object(*id).and_then(Object::as_stream).ok(),
            _ => None,
        })
        .filter(|s| {
            s.dict
                .get(b"Subtype")
                .ok()
                .and_then(|o| o.as_name().ok())
                .is_some_and(|n| n == b"Image")
        })
}

fn pixel_area(s: &Stream) -> i64 {
    let dim = |k: &[u8]| {
        s.dict
            .get(k)
            .ok()
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0)
    };
    dim(b"Width") * dim(b"Height")
}

/// The page's images as figures. JPEG2000, CCITT, JBIG2, CMYK and other
/// images without an EPUB-core, pure-Rust path are skipped.
fn figures(doc: &Document, page_id: ObjectId) -> Vec<Figure> {
    resources(doc, page_id)
        .map(|res| images(doc, res).filter_map(|s| figure(doc, s)).collect())
        .unwrap_or_default()
}

/// The page's largest image, as bytes OCR can decode: for a scan, where the
/// page is one big image. The first of equal largest wins.
#[cfg_attr(not(feature = "ocr"), allow(dead_code))]
pub(crate) fn scan_image(doc: &Document, page_id: ObjectId) -> Option<Figure> {
    let res = resources(doc, page_id)?;
    let mut best: Option<(&Stream, i64)> = None;
    for s in images(doc, res) {
        let area = pixel_area(s);
        if best.is_none_or(|(_, a)| area > a) {
            best = Some((s, area));
        }
    }
    figure(doc, best?.0)
}

/// An image stream as embeddable bytes: a DCT stream verbatim as JPEG; raw
/// 8-bit gray or RGB pixels as PNG; anything else none.
fn figure(doc: &Document, stream: &Stream) -> Option<Figure> {
    // With a filter chain, the last filter names the format.
    let filter = stream.dict.get(b"Filter").ok().and_then(|o| match o {
        Object::Name(n) => Some(n.as_slice()),
        Object::Array(a) => a.iter().filter_map(|x| x.as_name().ok()).next_back(),
        _ => None,
    });
    match filter {
        Some(b"DCTDecode") => Some(Figure {
            data: jpeg_bytes(doc, stream)?,
            media_type: "image/jpeg",
            ext: "jpg",
        }),
        Some(b"FlateDecode") | Some(b"LZWDecode") | None => raw_to_png(doc, stream),
        _ => None,
    }
}

/// The JPEG inside a stream whose last filter is DCTDecode. With DCTDecode
/// alone, the stream's bytes as stored. With filters in front of it, those are
/// undone one by one, each with its own /DecodeParms entry; if that fails or
/// does not end in a JPEG, none: wrong bytes under a JPEG label are worse than
/// no figure.
fn jpeg_bytes(doc: &Document, stream: &Stream) -> Option<Vec<u8>> {
    let chain = match stream.dict.get(b"Filter").ok()? {
        Object::Array(chain) if chain.len() > 1 => chain,
        _ => return Some(stream.content.clone()),
    };
    let parms = stream
        .dict
        .get(b"DecodeParms")
        .ok()
        .and_then(|o| o.as_array().ok());
    let mut data = stream.content.clone();
    for (i, filter) in chain[..chain.len() - 1].iter().enumerate() {
        let mut dict = Dictionary::new();
        dict.set("Filter", filter.clone());
        if let Some(p) = parms
            .and_then(|p| p.get(i))
            .and_then(|p| deref(doc, p))
            .and_then(|p| p.as_dict().ok())
        {
            dict.set("DecodeParms", p.clone());
        }
        data = Stream::new(dict, data).decompressed_content().ok()?;
    }
    data.starts_with(&[0xff, 0xd8]).then_some(data)
}

/// Re-encode a raw 8-bit DeviceGray/DeviceRGB image as PNG.
fn raw_to_png(doc: &Document, stream: &Stream) -> Option<Figure> {
    let dict = &stream.dict;
    let w = dict.get(b"Width").ok()?.as_i64().ok()? as u32;
    let h = dict.get(b"Height").ok()?.as_i64().ok()? as u32;
    let bpc = dict
        .get(b"BitsPerComponent")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(8);
    if bpc != 8 || w == 0 || h == 0 {
        return None;
    }
    let comps = components(doc, dict)?;
    let data = stream.decompressed_content().ok()?;
    if data.len() < (w as usize) * (h as usize) * comps {
        return None;
    }
    let img = match comps {
        1 => image::GrayImage::from_raw(w, h, data).map(image::DynamicImage::ImageLuma8),
        3 => image::RgbImage::from_raw(w, h, data).map(image::DynamicImage::ImageRgb8),
        _ => None,
    }?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(Figure {
        data: png,
        media_type: "image/png",
        ext: "png",
    })
}

/// Colour components of a raw image: 1 or 3, else none (Indexed, CMYK, Lab…).
fn components(doc: &Document, dict: &Dictionary) -> Option<usize> {
    match deref(doc, dict.get(b"ColorSpace").ok()?)? {
        Object::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" => Some(1),
            b"DeviceRGB" | b"CalRGB" | b"RGB" => Some(3),
            _ => None,
        },
        // [/ICCBased <stream>]: the profile's /N.
        Object::Array(a)
            if a.first().and_then(|o| o.as_name().ok()) == Some(b"ICCBased".as_slice()) =>
        {
            let icc = deref(doc, a.get(1)?)?.as_stream().ok()?;
            match icc.dict.get(b"N").ok()?.as_i64().ok()? {
                1 => Some(1),
                3 => Some(3),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmap_bfchar() {
        let cmap = b"begincmap
1 begincodespacerange <0000> <FFFF> endcodespacerange
2 beginbfchar
<0003> <0041>
<0004> <0042>
endbfchar
endcmap";
        let (map, code_len) = parse_cmap(cmap).unwrap();
        assert_eq!(code_len, 2, "4 hex digits = 2-byte codes");
        assert_eq!(map[&3], "A");
        assert_eq!(map[&4], "B");
    }

    #[test]
    fn cmap_bfrange_single_destination() {
        let (map, _) = parse_cmap(b"beginbfrange <0010> <0012> <0061> endbfrange").unwrap();
        assert_eq!(map[&0x10], "a");
        assert_eq!(map[&0x11], "b");
        assert_eq!(map[&0x12], "c");
    }

    #[test]
    fn cmap_bfrange_array() {
        let (map, _) =
            parse_cmap(b"beginbfrange <0020> <0021> [<0058> <005A>] endbfrange").unwrap();
        assert_eq!(map[&0x20], "X");
        assert_eq!(map[&0x21], "Z");
    }

    #[test]
    fn cmap_turkish_letters() {
        // ç ğ ı İ ö ş ü through bfchar, one-byte codes.
        let cmap = b"beginbfchar <01> <00E7> <02> <011F> <03> <0131> <04> <0130> \
<05> <00F6> <06> <015F> <07> <00FC> endbfchar";
        let (map, code_len) = parse_cmap(cmap).unwrap();
        assert_eq!(code_len, 1);
        let s: String = (1..=7).map(|c| map[&c].as_str()).collect();
        assert_eq!(s, "çğıİöşü");
    }

    #[test]
    fn cmap_range_at_end_of_stream() {
        let (map, code_len) = parse_cmap(b"beginbfrange <01> <02> <0061>").unwrap();
        assert_eq!(
            (map[&1].as_str(), map[&2].as_str(), code_len),
            ("a", "b", 1)
        );
    }

    #[test]
    fn hex_helpers() {
        assert_eq!(hex_units("0041").unwrap(), vec![0x41]);
        assert_eq!(hex_string("00660069").as_deref(), Some("fi"));
        assert_eq!(hex_u32("00ff"), Some(255));
        assert!(hex_units("zz").is_none());
    }

    #[test]
    fn empty_cmap_is_none() {
        assert!(parse_cmap(b"begincmap endcmap").is_none());
    }

    #[test]
    fn glued_text_scores_worse() {
        let spaced = vec!["At year end 2022, Berkshire was the largest owner".to_string()];
        let glued = vec!["Atyearend2022,Berkshirewasthelargestowner".to_string()];
        assert!(glue_score(&spaced) < glue_score(&glued));
        let more = vec!["At year end 2022, Berkshire was the largest owner of".to_string()];
        assert!(glue_score(&more) < glue_score(&spaced));
        assert!(glue_score(&glued) < glue_score(&[]));
    }

    #[test]
    fn letters_only_keeps_non_ascii_capitals() {
        // PARITY: documents the quirk to fix in 0.2.0.
        assert_eq!(letters_only("İSTANBUL, 12 Ekim"), "İstanbul ekim");
        assert_eq!(letters_only("ÇAĞ"), "ÇaĞ");
    }
}
