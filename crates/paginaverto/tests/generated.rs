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

#[test]
fn not_a_pdf_is_open_error() {
    let err = read(Input::Bytes(b"this is not a PDF"), &Options::default()).unwrap_err();
    assert!(matches!(err, Error::Open(_)), "{err:?}");
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
