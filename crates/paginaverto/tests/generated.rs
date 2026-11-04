//! End-to-end tests on PDFs generated here from known text. Public by
//! construction: no fixture comes from anyone's documents.

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
#[cfg(feature = "ocr")]
use paginaverto::Ocr;
use paginaverto::{Block, Error, Input, Options, read};

/// A one-page-per-content PDF whose pages share `font` (as `/F1`) and, when
/// given, an image XObject (as `/Im1`).
fn pdf(
    font: impl FnOnce(&mut Document) -> Option<Object>,
    image: Option<Stream>,
    pages: &[Vec<Operation>],
) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut res = dictionary! {};
    if let Some(font) = font(&mut doc) {
        let font_id = doc.add_object(font);
        res.set("Font", dictionary! { "F1" => font_id });
    }
    if let Some(image) = image {
        let im = doc.add_object(image);
        res.set("XObject", dictionary! { "Im1" => im });
    }
    let res_id = doc.add_object(res);
    let mut kids = Vec::new();
    for ops in pages {
        let content = Content {
            operations: ops.clone(),
        };
        let c = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Contents" => c,
            "Resources" => res_id,
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
    doc.save_to(&mut out).unwrap();
    out
}

fn op(name: &str, operands: Vec<Object>) -> Operation {
    Operation::new(name, operands)
}

fn hex(bytes: Vec<u8>) -> Object {
    Object::String(bytes, lopdf::StringFormat::Hexadecimal)
}

/// A Type0 / Identity-H font whose CID `i + 1` is `chars[i]`, through a
/// `/ToUnicode` CMap, every glyph half an em wide.
fn type0_font(doc: &mut Document, doc_chars: &[char]) -> Object {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
    );
    cmap.push_str(&format!("{} beginbfchar\n", doc_chars.len()));
    for (i, c) in doc_chars.iter().enumerate() {
        let mut units = [0u16; 2];
        let u = c.encode_utf16(&mut units);
        let dst: String = u.iter().map(|x| format!("{x:04X}")).collect();
        cmap.push_str(&format!("<{:04X}> <{dst}>\n", i + 1));
    }
    cmap.push_str("endbfchar\nendcmap\nend\nend\n");
    let to_unicode = doc.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
    Object::Dictionary(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => "Test-Identity",
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "CIDFontType2",
            "BaseFont" => "Test-Identity",
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
            "DW" => 500,
        })],
        "ToUnicode" => to_unicode,
    })
}

fn paragraphs(bytes: &[u8]) -> Vec<String> {
    let book = read(Input::Bytes(bytes), &Options::default()).expect("read");
    book.chapters
        .iter()
        .flat_map(|c| &c.blocks)
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn turkish_letters_through_a_type0_cmap() {
    let text = "Çağdaş Türkçe öğretmeni İzmir'de ışığı gördü; ŞÜPHE ÖĞÜT IĞDIR.";
    let mut alphabet: Vec<char> = text.chars().collect();
    alphabet.sort();
    alphabet.dedup();
    let codes: Vec<u8> = text
        .chars()
        .flat_map(|c| {
            let cid = alphabet.iter().position(|&a| a == c).unwrap() as u16 + 1;
            cid.to_be_bytes()
        })
        .collect();
    let bytes = pdf(
        |d| Some(type0_font(d, &alphabet)),
        None,
        &[vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![72.into(), 700.into()]),
            op("Tj", vec![hex(codes)]),
            op("ET", vec![]),
        ]],
    );
    assert_eq!(paragraphs(&bytes), vec![text.to_string()]);
}

/// Helvetica with explicit widths: 0.5 em for every code.
fn simple_font(_: &mut Document) -> Option<Object> {
    Some(Object::Dictionary(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
        "FirstChar" => 32,
        "LastChar" => 126,
        "Widths" => (32..=126).map(|_| Object::Integer(500)).collect::<Vec<_>>(),
    }))
}

#[test]
fn word_space_from_glyph_widths() {
    // Two runs per line with no space glyph between them, the second placed
    // 2 em after the first one's real end: a word space. Five lines, so the
    // page carries the 100 bytes of text that make it a text layer.
    let a = "Extraordinary";
    let b = "circumstances";
    let gap = a.len() as f32 * 0.5 * 12.0 + 24.0;
    let mut ops = Vec::new();
    for line in 0..5 {
        ops.extend([
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![72.into(), (700 - line * 14).into()]),
            op("Tj", vec![Object::string_literal(a)]),
            op("Td", vec![gap.into(), 0.into()]),
            op("Tj", vec![Object::string_literal(b)]),
            op("ET", vec![]),
        ]);
    }
    let text = paragraphs(&pdf(simple_font, None, &[ops])).join(" ");
    assert_eq!(text, vec![format!("{a} {b}"); 5].join(" "));
}

#[test]
fn heading_starts_a_chapter_and_running_head_goes() {
    // Every line differs in its letters, so no body line looks like a
    // running head or foot.
    let body = "This page was generated from known text and is long enough to count";
    let words = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
    ];
    let mut pages = Vec::new();
    for i in 0..6 {
        let mut ops = vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 10.into()]),
            op("Td", vec![72.into(), 800.into()]),
            op("Tj", vec![Object::string_literal("THE RUNNING HEAD")]),
            op("ET", vec![]),
        ];
        if i == 1 {
            ops.extend([
                op("BT", vec![]),
                op("Tf", vec!["F1".into(), 10.into()]),
                op("Td", vec![72.into(), 760.into()]),
                op("Tj", vec![Object::string_literal("CHAPTER TWO")]),
                op("ET", vec![]),
            ]);
        }
        for line in 0..8 {
            ops.extend([
                op("BT", vec![]),
                op("Tf", vec!["F1".into(), 10.into()]),
                op("Td", vec![72.into(), (700 - line * 14).into()]),
                op(
                    "Tj",
                    vec![Object::string_literal(format!(
                        "{body} {} {}.",
                        words[i], words[line as usize]
                    ))],
                ),
                op("ET", vec![]),
            ]);
        }
        pages.push(ops);
    }
    let bytes = pdf(simple_font, None, &pages);
    let book = read(Input::Bytes(&bytes), &Options::default()).unwrap();
    let titles: Vec<_> = book.chapters.iter().map(|c| c.title.as_deref()).collect();
    assert_eq!(titles, vec![None, Some("CHAPTER TWO")]);
    let all: String = paragraphs(&bytes).join("\n");
    assert!(!all.contains("RUNNING HEAD"), "running head kept");
}

/// A page-sized gray image and no text: a scan.
fn scan() -> Vec<u8> {
    let (w, h) = (800i64, 600i64);
    let image = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w,
            "Height" => h,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
        },
        vec![200u8; (w * h) as usize],
    );
    pdf(
        |_| None,
        Some(image),
        &[vec![
            op("q", vec![]),
            op(
                "cm",
                vec![
                    595.into(),
                    0.into(),
                    0.into(),
                    842.into(),
                    0.into(),
                    0.into(),
                ],
            ),
            op("Do", vec!["Im1".into()]),
            op("Q", vec![]),
        ]],
    )
}

#[test]
fn scan_with_ocr_off_is_refused() {
    let err = read(Input::Bytes(&scan()), &Options::default()).unwrap_err();
    assert_eq!(err, Error::ScanNeedsOcr);
}

#[cfg(feature = "ocr")]
#[test]
fn scan_checks_come_in_order() {
    let bytes = scan();
    let mut opts = Options::default();
    opts.ocr = Ocr::Models {
        detection: vec![0; 8],
        recognition: vec![0; 8],
    };
    // Language before models.
    assert_eq!(
        read(Input::Bytes(&bytes), &opts).unwrap_err(),
        Error::LanguageRequired
    );
    opts.language = Some(String::new());
    assert_eq!(
        read(Input::Bytes(&bytes), &opts).unwrap_err(),
        Error::LanguageRequired
    );
    opts.language = Some("tr".into());
    assert!(matches!(
        read(Input::Bytes(&bytes), &opts),
        Err(Error::Models(_))
    ));
}

/// The paragraphs of a page of `simple_font` text drawn by `ops`.
fn page_text(ops: Vec<Operation>) -> Vec<String> {
    paragraphs(&pdf(simple_font, None, &[ops]))
}

/// A long enough page: `line` shown five times, one under the other, by what
/// `show` draws at each line.
fn five_lines(show: impl Fn(&mut Vec<Operation>)) -> Vec<Operation> {
    let mut ops = Vec::new();
    for line in 0..5 {
        ops.extend([
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![72.into(), (700 - line * 14).into()]),
        ]);
        show(&mut ops);
        ops.push(op("ET", vec![]));
    }
    ops
}

#[test]
fn large_tj_adjustment_is_a_word_space() {
    let ops = five_lines(|ops| {
        let parts = vec![
            Object::string_literal("Extraordinary"),
            Object::Integer(-300),
            Object::string_literal("circumstances"),
        ];
        ops.push(op("TJ", vec![Object::Array(parts)]));
    });
    let text = page_text(ops).join(" ");
    assert_eq!(text, ["Extraordinary circumstances"; 5].join(" "));
}

#[test]
fn small_tj_adjustment_is_only_kerning() {
    let ops = five_lines(|ops| {
        let parts = vec![
            Object::string_literal("Extraordinary"),
            Object::Integer(-40),
            Object::string_literal("circumstances"),
        ];
        ops.push(op("TJ", vec![Object::Array(parts)]));
    });
    let text = page_text(ops).join(" ");
    assert_eq!(text, ["Extraordinarycircumstances"; 5].join(" "));
}

#[test]
fn showing_text_advances_so_a_later_run_stays_after_it() {
    // Two Tj in a row with no positioning: the second continues the first.
    let ops = five_lines(|ops| {
        ops.push(op("Tj", vec![Object::string_literal("Extraordinary ")]));
        ops.push(op("Tj", vec![Object::string_literal("circumstances")]));
    });
    let text = page_text(ops).join(" ");
    assert_eq!(text, ["Extraordinary circumstances"; 5].join(" "));
}

#[test]
fn character_spacing_counts_in_a_runs_width() {
    // Without Tc in its width, the first run seems to end far before the
    // second starts, which would read as a word space.
    let a = "Extraordinary";
    let tc = 2.0;
    let gap = a.len() as f32 * (0.5 * 12.0 + tc);
    let ops = five_lines(|ops| {
        ops.push(op("Tc", vec![tc.into()]));
        ops.push(op("Tj", vec![Object::string_literal(a)]));
        ops.push(op("Tc", vec![0.into()]));
        ops.push(op("Td", vec![gap.into(), 0.into()]));
        ops.push(op("Tj", vec![Object::string_literal("circumstances")]));
    });
    let text = page_text(ops).join(" ");
    assert_eq!(text, ["Extraordinarycircumstances"; 5].join(" "));
}

#[test]
fn a_capitals_line_is_not_merged_into_the_text_under_it() {
    let mut ops = Vec::new();
    let lines = [
        "THE FIRST PART",
        "Some words that open the part and run for a while longer than this.",
        "Some more words that go on after them, so that the page has text.",
    ];
    for (n, line) in lines.iter().enumerate() {
        ops.extend([
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![72.into(), (700 - n as i64 * 14).into()]),
            op("Tj", vec![Object::string_literal(*line)]),
            op("ET", vec![]),
        ]);
    }
    let text = page_text(ops);
    assert_eq!(text[0], "THE FIRST PART");
    assert_eq!(text.len(), 2);
}

/// A tiny JPEG, with zeros after its end so that it compresses.
fn jpeg() -> Vec<u8> {
    let img = image::GrayImage::from_pixel(8, 8, image::Luma([128]));
    let mut out = Vec::new();
    image::DynamicImage::ImageLuma8(img)
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    out.resize(out.len() + 4096, 0);
    out
}

fn deflate(bytes: &[u8]) -> Vec<u8> {
    let mut s = Stream::new(dictionary! {}, bytes.to_vec());
    s.compress().unwrap();
    assert!(s.dict.has(b"Filter"), "the fixture must compress");
    s.content
}

fn names(names: &[&str]) -> Object {
    Object::Array(names.iter().map(|&n| n.into()).collect())
}

/// The figures read from a text page that shows an image stored as `stored`
/// under `filter`.
fn figures_of(filter: Object, parms: Option<Object>, stored: Vec<u8>) -> Vec<Vec<u8>> {
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => 8,
        "Height" => 8,
        "ColorSpace" => "DeviceGray",
        "BitsPerComponent" => 8,
        "Filter" => filter,
    };
    if let Some(parms) = parms {
        dict.set("DecodeParms", parms);
    }
    // Long enough to count as a text layer.
    let text = "A sentence of ordinary words. ".repeat(4);
    let bytes = pdf(
        simple_font,
        Some(Stream::new(dict, stored)),
        &[vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![72.into(), 700.into()]),
            op("Tj", vec![Object::string_literal(text)]),
            op("ET", vec![]),
        ]],
    );
    let book = read(Input::Bytes(&bytes), &Options::default()).unwrap();
    book.chapters
        .iter()
        .flat_map(|c| &c.blocks)
        .filter_map(|b| match b {
            Block::Figure(f) => {
                assert_eq!(f.media_type, "image/jpeg");
                Some(f.data.clone())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn jpeg_alone_stays_as_stored() {
    let j = jpeg();
    assert_eq!(figures_of("DCTDecode".into(), None, j.clone()), vec![j]);
}

#[test]
fn flate_in_front_of_jpeg_is_undone() {
    let j = jpeg();
    let chain = names(&["FlateDecode", "DCTDecode"]);
    assert_eq!(figures_of(chain, None, deflate(&j)), vec![j]);
}

#[test]
fn two_filters_in_front_of_jpeg_are_undone() {
    let j = jpeg();
    let mut stored: Vec<u8> = deflate(&j)
        .iter()
        .flat_map(|b| format!("{b:02x}").into_bytes())
        .collect();
    stored.push(b'>');
    let chain = names(&["ASCIIHexDecode", "FlateDecode", "DCTDecode"]);
    assert_eq!(figures_of(chain, None, stored), vec![j]);
}

#[test]
fn null_parms_entries_in_a_jpeg_chain() {
    let j = jpeg();
    let chain = names(&["FlateDecode", "DCTDecode"]);
    let parms = Object::Array(vec![Object::Null, Object::Null]);
    assert_eq!(figures_of(chain, Some(parms), deflate(&j)), vec![j]);
}

#[test]
fn a_chain_that_does_not_end_in_a_jpeg_gives_no_figure() {
    let chain = names(&["FlateDecode", "DCTDecode"]);
    // Inflates cleanly, to bytes that are no JPEG.
    let not_jpeg = deflate(&[7u8; 4096]);
    assert!(figures_of(chain.clone(), None, not_jpeg).is_empty());
    // Does not inflate at all.
    assert!(figures_of(chain, None, vec![1, 2, 3, 4]).is_empty());
    // A filter nothing here can undo.
    let chain = names(&["JBIG2Decode", "DCTDecode"]);
    assert!(figures_of(chain, None, jpeg()).is_empty());
}

#[test]
fn not_a_pdf_is_open_error() {
    let err = read(Input::Bytes(b"this is not a PDF"), &Options::default()).unwrap_err();
    // The cause is kept, not only lopdf's outer message.
    assert_eq!(
        err,
        Error::Open("couldn't parse input: invalid file header".into())
    );
}

#[test]
fn missing_file_is_open_error() {
    let err = read(
        Input::Path("/nonexistent/paginaverto/test.pdf".as_ref()),
        &Options::default(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::Open(_)), "{err:?}");
}
