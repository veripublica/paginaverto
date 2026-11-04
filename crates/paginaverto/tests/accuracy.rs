//! The public accuracy set, text-layer path: known text in English and Turkish
//! laid out on generated PDFs the ways real PDFs do it, read back, and compared.
//! Prints the character error rate per language and layout; run with
//! `cargo test -p paginaverto --test accuracy -- --nocapture`. The bar fails the
//! test if a layout gets worse. The OCR path is measured by `tools/ocr-accuracy`.

use std::collections::BTreeSet;

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use paginaverto::{Block, Input, Options, read};

const EN: &str = include_str!("corpus/en.txt");
const TR: &str = include_str!("corpus/tr.txt");

const SIZE: f32 = 11.0;
const GLYPH_EM: f32 = 0.5;
const LEFT: f32 = 72.0;
const TOP: f32 = 760.0;
const LEADING: f32 = 14.0;
const LINE_CHARS: usize = 70;
const LINES_PER_PAGE: usize = 18;
const BOOK_REPEATS: usize = 3;

/// The bar: what this set measures today. Raise it only with a reason.
const MAX_CER: f64 = 0.0;

/// How a line's characters get onto the page.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Layout {
    /// One `Tj` per line, spaces shown as glyphs.
    Plain,
    /// `TJ` with small kerning inside words, spaces shown as glyphs.
    Kerned,
    /// `TJ` with no space glyphs: a large adjustment makes each word space.
    TjGaps,
    /// Every word its own `Tj`, with no positioning in between, spaces shown.
    WordRuns,
    /// Every word placed by `Td`, with no space glyphs.
    Positioned,
    /// Every glyph placed by `Td`.
    GlyphByGlyph,
    /// Letter-spaced with `Tc`, spaces shown.
    LetterSpaced,
    /// Condensed with `Tz`, spaces shown.
    Condensed,
    /// `Plain`, with a running head, a page number, and words hyphenated at
    /// line ends.
    Book,
}

const LAYOUTS: [Layout; 9] = [
    Layout::Plain,
    Layout::Kerned,
    Layout::TjGaps,
    Layout::WordRuns,
    Layout::Positioned,
    Layout::GlyphByGlyph,
    Layout::LetterSpaced,
    Layout::Condensed,
    Layout::Book,
];

/// Paragraphs wrapped to lines; the first line of each is indented by the
/// caller. In `hyphenate`, a word that overflows is split with a hyphen.
fn wrap(paragraph: &str, hyphenate: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in paragraph.split_whitespace() {
        let used = line.chars().count();
        let len = word.chars().count();
        if used + 1 + len <= LINE_CHARS || line.is_empty() {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
            continue;
        }
        let room = LINE_CHARS.saturating_sub(used + 2);
        let lower_tail = word.chars().nth(room).is_some_and(char::is_lowercase);
        if hyphenate && room >= 3 && len >= room + 3 && lower_tail {
            let (a, b): (String, String) = (
                word.chars().take(room).collect(),
                word.chars().skip(room).collect(),
            );
            line.push(' ');
            line.push_str(&a);
            line.push('-');
            lines.push(std::mem::take(&mut line));
            line = b;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn op(name: &str, operands: Vec<Object>) -> Operation {
    Operation::new(name, operands)
}

/// A Type0 font over `chars`, so every letter has a two-byte code and a
/// `/ToUnicode` entry, as modern PDFs do it.
struct Font {
    chars: Vec<char>,
}

impl Font {
    fn code(&self, c: char) -> u16 {
        self.chars
            .iter()
            .position(|&x| x == c)
            .expect("in the font") as u16
            + 1
    }

    fn hex(&self, s: &str) -> Object {
        let bytes: Vec<u8> = s.chars().flat_map(|c| self.code(c).to_be_bytes()).collect();
        Object::String(bytes, lopdf::StringFormat::Hexadecimal)
    }

    fn object(&self, doc: &mut Document) -> Object {
        let mut bf = String::new();
        for (i, c) in self.chars.iter().enumerate() {
            let mut utf16 = [0u16; 2];
            let u = c.encode_utf16(&mut utf16);
            bf.push_str(&format!("<{:04X}> <{:04X}>\n", i + 1, u[0]));
        }
        let cmap = format!(
            "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
             /CMapName /Generated def /CMapType 2 def\n\
             1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
             {} beginbfchar\n{bf}endbfchar\nendcmap\nend end\n",
            self.chars.len()
        );
        let to_unicode = doc.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
        let width = (GLYPH_EM * 1000.0) as i64;
        let descendant = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "CIDFontType2",
            "BaseFont" => "Generated",
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
            "DW" => width,
        });
        Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => "Generated",
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![descendant.into()],
            "ToUnicode" => to_unicode,
        })
    }
}

/// The operators that draw one line at (x, y).
fn draw_line(font: &Font, layout: Layout, line: &str, x: f32, y: f32) -> Vec<Operation> {
    let glyph = GLYPH_EM * SIZE;
    let mut ops = vec![
        op("BT", vec![]),
        op("Tf", vec!["F1".into(), SIZE.into()]),
        op("Td", vec![x.into(), y.into()]),
    ];
    let words: Vec<&str> = line.split(' ').collect();
    match layout {
        Layout::Plain | Layout::Book => ops.push(op("Tj", vec![font.hex(line)])),
        Layout::Kerned => {
            let mut parts = Vec::new();
            for c in line.chars() {
                parts.push(font.hex(&c.to_string()));
                parts.push(Object::Integer(15));
            }
            ops.push(op("TJ", vec![Object::Array(parts)]));
        }
        Layout::TjGaps => {
            let mut parts = Vec::new();
            for w in &words {
                parts.push(font.hex(w));
                parts.push(Object::Integer(-300));
            }
            ops.push(op("TJ", vec![Object::Array(parts)]));
        }
        Layout::WordRuns => {
            for (i, w) in words.iter().enumerate() {
                let s = if i + 1 < words.len() {
                    format!("{w} ")
                } else {
                    (*w).to_string()
                };
                ops.push(op("Tj", vec![font.hex(&s)]));
            }
        }
        Layout::Positioned => {
            let mut prev = 0;
            for (i, w) in words.iter().enumerate() {
                if i > 0 {
                    let dx = (prev + 1) as f32 * glyph;
                    ops.push(op("Td", vec![dx.into(), 0.into()]));
                }
                ops.push(op("Tj", vec![font.hex(w)]));
                prev = w.chars().count();
            }
        }
        Layout::GlyphByGlyph => {
            let mut pending = 0.0;
            let mut first = true;
            for c in line.chars() {
                if c == ' ' {
                    pending += glyph;
                    continue;
                }
                if !first {
                    ops.push(op("Td", vec![(pending + glyph).into(), 0.into()]));
                }
                pending = 0.0;
                first = false;
                ops.push(op("Tj", vec![font.hex(&c.to_string())]));
            }
        }
        Layout::LetterSpaced => {
            ops.push(op("Tc", vec![0.8.into()]));
            ops.push(op("Tj", vec![font.hex(line)]));
            ops.push(op("Tc", vec![0.into()]));
        }
        Layout::Condensed => {
            ops.push(op("Tz", vec![85.into()]));
            ops.push(op("Tj", vec![font.hex(line)]));
            ops.push(op("Tz", vec![100.into()]));
        }
    }
    ops.push(op("ET", vec![]));
    ops
}

/// The corpus as a PDF in this layout, as many pages as it takes.
fn pdf(corpus: &str, layout: Layout) -> Vec<u8> {
    let book = layout == Layout::Book;
    let chars: BTreeSet<char> = corpus
        .chars()
        .chain("0123456789 -".chars())
        .chain("THE RUNNING HEAD".chars())
        .filter(|c| *c != '\n')
        .collect();
    let font = Font {
        chars: chars.into_iter().collect(),
    };

    let mut lines: Vec<(String, bool)> = Vec::new();
    for paragraph in corpus
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        for (i, line) in wrap(paragraph, book).into_iter().enumerate() {
            lines.push((line, i == 0));
        }
    }

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_object = font.object(&mut doc);
    let font_id = doc.add_object(font_object);
    let res = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let mut kids: Vec<Object> = Vec::new();
    for (n, chunk) in lines.chunks(LINES_PER_PAGE).enumerate() {
        let mut ops = Vec::new();
        if book {
            ops.extend(draw_line(
                &font,
                Layout::Plain,
                "THE RUNNING HEAD",
                LEFT,
                TOP + 40.0,
            ));
        }
        for (i, (line, first)) in chunk.iter().enumerate() {
            let x = if *first { LEFT + SIZE * 2.0 } else { LEFT };
            ops.extend(draw_line(&font, layout, line, x, TOP - i as f32 * LEADING));
        }
        if book {
            let number = (n + 1).to_string();
            ops.extend(draw_line(&font, Layout::Plain, &number, LEFT, 40.0));
        }
        let content = Content { operations: ops }.encode().expect("encodes");
        let c = doc.add_object(Stream::new(dictionary! {}, content));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Contents" => c,
            "Resources" => res,
        });
        kids.push(page.into());
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("saves");
    out
}

/// Levenshtein distance over characters.
fn distance(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (diag + usize::from(ca != cb))
                .min(row[j] + 1)
                .min(row[j + 1] + 1);
            diag = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

fn squash(s: &str) -> Vec<char> {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect()
}

/// Character error rate of reading the corpus in this layout (in %), and the
/// paragraphs found.
fn measure(corpus: &str, layout: Layout) -> (f64, usize) {
    // Long enough for a running head to recur.
    let corpus = &if layout == Layout::Book {
        [corpus; BOOK_REPEATS].join("\n\n")
    } else {
        corpus.to_string()
    };
    let book = read(Input::Bytes(&pdf(corpus, layout)), &Options::default()).expect("reads");
    let mut text = String::new();
    let mut paragraphs = 0;
    for chapter in &book.chapters {
        if let Some(t) = &chapter.title {
            text.push_str(t);
            text.push(' ');
            paragraphs += 1;
        }
        for b in &chapter.blocks {
            if let Block::Paragraph(p) = b {
                text.push_str(p);
                text.push(' ');
                paragraphs += 1;
            }
        }
    }
    let (got, want) = (squash(&text), squash(corpus));
    (
        100.0 * distance(&got, &want) as f64 / want.len() as f64,
        paragraphs,
    )
}

#[test]
fn text_layer_accuracy() {
    let paragraphs_in = |corpus: &str| corpus.split("\n\n").count();
    for (name, corpus) in [("en", EN), ("tr", TR)] {
        for layout in LAYOUTS {
            let (cer, paragraphs) = measure(corpus, layout);
            let want = paragraphs_in(corpus)
                * if layout == Layout::Book {
                    BOOK_REPEATS
                } else {
                    1
                };
            println!("{name} {layout:?}: CER {cer:.2}%, {paragraphs} paragraphs of {want}");
            assert!(cer <= MAX_CER, "{name} {layout:?}: CER {cer:.2}%");
            assert_eq!(paragraphs, want, "{name} {layout:?}");
        }
    }
}
