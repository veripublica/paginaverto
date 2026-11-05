//! End-to-end tests on PDFs generated here from known text. Public by
//! construction: no fixture comes from anyone's documents.

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
#[cfg(feature = "ocr")]
use paginaverto::Ocr;
use paginaverto::{Block, BlockKind, Error, Input, Options, read};

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
    book.blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Paragraph { .. }))
        .map(Block::text)
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
    let chapters = book.chapters();
    let titles: Vec<_> = chapters.iter().map(|c| c.title.as_deref()).collect();
    assert_eq!(titles, vec![None, Some("CHAPTER TWO")]);
    let all: String = paragraphs(&bytes).join("\n");
    assert!(!all.contains("RUNNING HEAD"), "running head kept");
}

/// A page-sized gray image and no text: a scan.
fn scan() -> Vec<u8> {
    scan_of(None)
}

/// A one-page scan whose image claims to be stored under `filter`.
fn scan_of(filter: Option<&str>) -> Vec<u8> {
    let (w, h) = (800i64, 600i64);
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => w,
        "Height" => h,
        "ColorSpace" => "DeviceGray",
        "BitsPerComponent" => 8,
    };
    if let Some(filter) = filter {
        dict.set("Filter", filter);
    }
    let image = Stream::new(dict, vec![200u8; (w * h) as usize]);
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

#[test]
fn undecodable_scan_names_its_format_before_asking_for_ocr() {
    for (filter, name) in [
        ("JBIG2Decode", "JBIG2"),
        ("JPXDecode", "JPEG 2000"),
        ("CCITTFaxDecode", "CCITT fax"),
    ] {
        let err = read(Input::Bytes(&scan_of(Some(filter))), &Options::default()).unwrap_err();
        assert_eq!(err, Error::UndecodableImages(vec![name.to_string()]));
        assert!(err.to_string().contains(name), "{err}");
    }
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
fn an_image_inherited_from_the_page_tree_counts_as_a_full_page() {
    // Text over a page-sized image found only in the parent's /Resources: a
    // scan with a text layer, so its image is the page and no figure.
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = simple_font(&mut doc).unwrap();
    let font = doc.add_object(font);
    let image = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 800,
            "Height" => 600,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
        },
        vec![200u8; 480_000],
    ));
    let ops = five_lines(|ops| {
        ops.push(op(
            "Tj",
            vec![Object::string_literal("Extraordinary words here")],
        ));
    });
    let content = Content { operations: ops }.encode().unwrap();
    let c = doc.add_object(Stream::new(dictionary! {}, content));
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        "Contents" => c,
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page.into()],
            "Count" => 1,
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => font },
                "XObject" => dictionary! { "Im1" => image },
            },
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    let book = read(Input::Bytes(&bytes), &Options::default()).unwrap();
    let mut blocks = book.blocks.iter();
    assert!(!blocks.any(|b| matches!(b.kind, BlockKind::Figure(_))));
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
    book.blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::Figure(f) => {
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

/// One text page per title, with an outline to them, and the fonts named.
fn outlined(entries: &[(&str, Option<u32>)], font_name: &str) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => font_name,
        "Encoding" => "WinAnsiEncoding",
    });
    let mut kids: Vec<Object> = Vec::new();
    for (title, _) in entries {
        let body = format!("{title} ").repeat(30);
        let ops = vec![
            op("BT", vec![]),
            op("Tf", vec!["F1".into(), 24.into()]),
            op("Td", vec![72.into(), 700.into()]),
            op("Tj", vec![Object::string_literal(title.to_string())]),
            op("Tf", vec!["F1".into(), 12.into()]),
            op("Td", vec![0.into(), Object::Integer(-40)]),
            op("Tj", vec![Object::string_literal(body)]),
            op("ET", vec![]),
        ];
        let content = doc.add_object(Stream::new(
            dictionary! {},
            Content { operations: ops }.encode().unwrap(),
        ));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Contents" => content,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        });
        kids.push(page.into());
    }
    let count = kids.len() as i64;
    let pages: Vec<Object> = kids.clone();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );

    let root = doc.new_object_id();
    let ids: Vec<_> = entries.iter().map(|_| doc.new_object_id()).collect();
    for (i, (title, target)) in entries.iter().enumerate() {
        let mut node = dictionary! {
            "Title" => Object::string_literal(title.to_string()),
            "Parent" => root,
        };
        if let Some(t) = target {
            node.set("Dest", vec![pages[*t as usize - 1].clone(), "Fit".into()]);
        }
        if i > 0 {
            node.set("Prev", ids[i - 1]);
        }
        if let Some(next) = ids.get(i + 1) {
            node.set("Next", *next);
        }
        doc.objects.insert(ids[i], Object::Dictionary(node));
    }
    doc.objects.insert(
        root,
        Object::Dictionary(dictionary! {
            "First" => ids[0], "Last" => ids[ids.len() - 1],
            "Count" => ids.len() as i64,
        }),
    );
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages_id, "Outlines" => root, "Lang" => Object::string_literal("tr"),
    });
    doc.trailer.set("Root", catalog);
    let info = doc.add_object(dictionary! { "Title" => Object::string_literal("The Book") });
    doc.trailer.set("Info", info);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn the_outline_sets_the_chapters_and_the_metadata_comes_through() {
    let bytes = outlined(
        &[
            ("First Part", Some(1)),
            ("Second Part", Some(2)),
            ("Third Part", Some(3)),
        ],
        "Helvetica",
    );
    let doc = read(Input::Bytes(&bytes), &Options::default()).unwrap();
    let chapters = doc.chapters();
    let titles: Vec<_> = chapters.iter().map(|c| c.title.as_deref()).collect();
    assert_eq!(
        titles,
        [Some("First Part"), Some("Second Part"), Some("Third Part")]
    );
    assert_eq!(doc.language.as_deref(), Some("tr"));
    assert_eq!(doc.metadata.catalog_lang.as_deref(), Some("tr"));
    assert_eq!(doc.metadata.info["Title"], "The Book");
    let pages: Vec<_> = doc.metadata.outline.iter().map(|e| e.page).collect();
    assert_eq!(pages, [Some(1), Some(2), Some(3)]);
    let first = &doc.blocks[0];
    assert_eq!(first.boxes[0].page, 1);
    assert!(
        first.boxes[0].y < 150.0,
        "from the top: {:?}",
        first.boxes[0]
    );
    assert_eq!(doc.pages[0].height, 842.0);
}

#[test]
fn an_outline_entry_without_a_target_costs_only_itself() {
    let bytes = outlined(
        &[
            ("First Part", Some(1)),
            ("Second Part", None),
            ("Third Part", Some(3)),
        ],
        "Helvetica",
    );
    let doc = read(Input::Bytes(&bytes), &Options::default()).unwrap();
    let pages: Vec<_> = doc.metadata.outline.iter().map(|e| e.page).collect();
    assert_eq!(pages, [Some(1), None, Some(3)]);
}

#[test]
fn a_circular_outline_ends() {
    let bytes = outlined(
        &[("First Part", Some(1)), ("Second Part", Some(2))],
        "Helvetica",
    );
    let mut doc = Document::load_mem(&bytes).unwrap();
    let last = doc
        .objects
        .iter()
        .find(|(_, o)| {
            o.as_dict()
                .is_ok_and(|d| d.get(b"Title").is_ok() && d.get(b"Prev").is_ok())
        })
        .map(|(id, _)| *id)
        .unwrap();
    let first = doc
        .objects
        .iter()
        .find(|(_, o)| {
            o.as_dict()
                .is_ok_and(|d| d.get(b"Title").is_ok() && d.get(b"Next").is_ok())
        })
        .map(|(id, _)| *id)
        .unwrap();
    doc.get_object_mut(last)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Next", first);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let got = read(Input::Bytes(&out), &Options::default()).unwrap();
    assert_eq!(got.metadata.outline.len(), 2);
}

#[test]
fn italic_runs_come_from_the_font() {
    for (name, italic) in [("ABCDEF+Garamond-Italic", true), ("Helvetica", false)] {
        let bytes = outlined(&[("First Part", Some(1)), ("Second Part", Some(2))], name);
        let doc = read(Input::Bytes(&bytes), &Options::default()).unwrap();
        let body = doc.blocks.iter().find(|b| b.level().is_none()).unwrap();
        assert!(body.runs().iter().all(|r| r.italic == italic), "{name}");
    }
}

#[test]
fn outline_titles_do_not_hide_a_pdf_with_no_text() {
    // Two pages of blank text from a font with no letters would be a refusal;
    // outline titles must not turn it into a document.
    let mut doc = Document::load_mem(&outlined(
        &[("Long Enough", Some(1)), ("And Another", Some(2))],
        "Helvetica",
    ))
    .unwrap();
    for id in doc.get_pages().into_values() {
        let content = doc.add_object(Stream::new(dictionary! {}, b"BT ET".to_vec()));
        doc.get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Contents", content);
    }
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    assert!(read(Input::Bytes(&out), &Options::default()).is_err());
}

// ---------------------------------------------------------------------------
// Image codecs, against fixtures made by tests/fixtures/make.py from a fixed
// pattern rebuilt here.

const PATTERN: (u32, u32) = (400, 400);
/// The pattern's blocks, and how many there are across.
const BLOCK: u32 = 16;
const BLOCKS_ACROSS: u32 = PATTERN.0.div_ceil(BLOCK);
const BLOCKS: usize = (BLOCKS_ACROSS * PATTERN.1.div_ceil(BLOCK)) as usize;

/// Each pixel's block, row by row, as make.py lays them.
fn blocks() -> impl Iterator<Item = usize> {
    (0..PATTERN.1).flat_map(|y| {
        (0..PATTERN.0).map(move |x| ((y / BLOCK) * BLOCKS_ACROSS + x / BLOCK) as usize)
    })
}

/// The C library's rand(), as make.py draws it.
fn pattern(n: usize) -> Vec<u32> {
    let mut x: u64 = 1;
    (0..n)
        .map(|_| {
            x = (x * 1_103_515_245 + 12_345) % (1 << 31);
            (x >> 16) as u32
        })
        .collect()
}

/// The one figure a text-layer page with this image gives, decoded, with its
/// media type.
fn the_figure_as(dict: lopdf::Dictionary, stored: Vec<u8>) -> (&'static str, image::DynamicImage) {
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
    let doc = read(Input::Bytes(&bytes), &Options::default()).unwrap();
    let figures: Vec<_> = doc
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::Figure(f) => Some(f.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(figures.len(), 1);
    (
        figures[0].media_type,
        image::load_from_memory(&figures[0].data).unwrap(),
    )
}

/// The same, for a gray figure, which comes as an exact PNG.
fn the_figure(dict: lopdf::Dictionary, stored: Vec<u8>) -> image::DynamicImage {
    let (media_type, image) = the_figure_as(dict, stored);
    assert_eq!(media_type, "image/png");
    image
}

fn image_dict(filter: &str, colour: &str, bits: i64) -> lopdf::Dictionary {
    dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => i64::from(PATTERN.0),
        "Height" => i64::from(PATTERN.1),
        "ColorSpace" => colour,
        "BitsPerComponent" => bits,
        "Filter" => filter,
    }
}

#[test]
fn a_group4_fax_figure_decodes_to_its_exact_pixels() {
    let mut dict = image_dict("CCITTFaxDecode", "DeviceGray", 1);
    dict.set(
        "DecodeParms",
        dictionary! {
            "K" => -1,
            "Columns" => i64::from(PATTERN.0),
            "Rows" => i64::from(PATTERN.1),
            "BlackIs1" => true,
        },
    );
    let got = the_figure(dict, include_bytes!("fixtures/pattern.g4").to_vec()).into_luma8();
    let r = pattern(BLOCKS);
    let want: Vec<u8> = blocks()
        .map(|b| if r[b] & 1 == 1 { 255 } else { 0 })
        .collect();
    assert_eq!(got.dimensions(), PATTERN);
    assert_eq!(got.into_raw(), want);
}

#[test]
fn a_lossless_jpeg_2000_figure_decodes_exactly_in_gray_and_closely_in_colour() {
    let n = BLOCKS;
    let r = pattern(n * 3);
    let gray = the_figure(
        image_dict("JPXDecode", "DeviceGray", 8),
        include_bytes!("fixtures/pattern.jp2").to_vec(),
    );
    let want: Vec<u8> = blocks().map(|b| (r[b] & 255) as u8).collect();
    assert_eq!(gray.into_luma8().into_raw(), want);

    // Colour comes as a JPEG at quality 90: near, not exact.
    let (media_type, rgb) = the_figure_as(
        image_dict("JPXDecode", "DeviceRGB", 8),
        include_bytes!("fixtures/pattern-rgb.jp2").to_vec(),
    );
    assert_eq!(media_type, "image/jpeg");
    let want: Vec<u8> = blocks()
        .flat_map(|b| [r[b], r[b + n], r[b + 2 * n]].map(|v| (v & 255) as u8))
        .collect();
    let got = rgb.into_rgb8().into_raw();
    assert_eq!(got.len(), want.len());
    let mean_error = got
        .iter()
        .zip(&want)
        .map(|(&a, &b)| f64::from(a.abs_diff(b)))
        .sum::<f64>()
        / want.len() as f64;
    assert!(mean_error < 4.0, "mean error {mean_error}");
}

#[test]
fn a_damaged_jpeg_2000_figure_is_no_figure() {
    let mut data = include_bytes!("fixtures/pattern.jp2").to_vec();
    data.truncate(40);
    let bytes = {
        let text = "A sentence of ordinary words. ".repeat(4);
        pdf(
            simple_font,
            Some(Stream::new(image_dict("JPXDecode", "DeviceGray", 8), data)),
            &[vec![
                op("BT", vec![]),
                op("Tf", vec!["F1".into(), 12.into()]),
                op("Td", vec![72.into(), 700.into()]),
                op("Tj", vec![Object::string_literal(text)]),
                op("ET", vec![]),
            ]],
        )
    };
    let doc = read(Input::Bytes(&bytes), &Options::default()).unwrap();
    assert!(
        !doc.blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Figure(_)))
    );
}

// ---------------------------------------------------------------------------
// Page images as input.

fn gray_png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::GrayImage::from_pixel(w, h, image::Luma([230]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn tiff_of(pages: &[(u32, u32)]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    let mut encoder = tiff::encoder::TiffEncoder::new(&mut out).unwrap();
    for &(w, h) in pages {
        let pixels = vec![230u8; (w * h) as usize];
        encoder
            .write_image::<tiff::encoder::colortype::Gray8>(w, h, &pixels)
            .unwrap();
    }
    out.into_inner()
}

#[test]
fn every_page_of_a_tiff_is_a_scan_page() {
    let bytes = tiff_of(&[(300, 400), (600, 800), (300, 400)]);
    let found = paginaverto::inspect(Input::Bytes(&bytes), &Options::default()).unwrap();
    assert!(found.reads_as_scan);
    assert_eq!(found.pages.len(), 3);
    assert!(
        found
            .pages
            .iter()
            .all(|p| p.source == paginaverto::PageSource::Scan && p.undecodable_image.is_none())
    );
    // No resolution in the file: 300 dpi, so 600 pixels are 144 points.
    assert!((found.pages[1].width - 144.0).abs() < 0.01);
}

#[test]
fn a_folder_of_images_reads_in_natural_order() {
    let dir = std::env::temp_dir().join(format!("pv-images-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Widths tell the pages apart: page 2 is 600 pixels, page 10 is 900.
    for (name, w) in [("page10.png", 900), ("page2.png", 600), ("page1.png", 300)] {
        std::fs::write(dir.join(name), gray_png(w, 100)).unwrap();
    }
    std::fs::write(dir.join("notes.txt"), "not a page").unwrap();
    std::fs::write(dir.join(".hidden.png"), "not a page either").unwrap();
    let found = paginaverto::inspect(Input::Path(&dir), &Options::default()).unwrap();
    let widths: Vec<f32> = found.pages.iter().map(|p| p.width).collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(widths, [72.0, 144.0, 216.0]);
}

#[test]
fn a_folder_with_no_images_is_an_open_error() {
    let dir = std::env::temp_dir().join(format!("pv-no-images-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let err = read(Input::Path(&dir), &Options::default()).unwrap_err();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(matches!(err, Error::Open(_)), "{err:?}");
}

#[test]
fn a_page_image_with_ocr_off_is_refused_as_a_scan_is() {
    let err = read(Input::Bytes(&gray_png(200, 100)), &Options::default()).unwrap_err();
    assert_eq!(err, Error::ScanNeedsOcr);
}

#[test]
fn a_damaged_page_image_is_reported_by_inspect() {
    let mut bytes = gray_png(200, 100);
    bytes.truncate(60);
    let found = paginaverto::inspect(Input::Bytes(&bytes), &Options::default()).unwrap();
    assert_eq!(found.pages[0].undecodable_image, Some("PNG"));
}

// ---------------------------------------------------------------------------
// Building a document by hand, as a caller's tests do.

#[test]
fn a_document_built_by_hand_splits_into_chapters() {
    use paginaverto::{Document, Figure, Run};
    let mut doc = Document::default();
    doc.blocks = vec![
        Block::paragraph("b1", vec![Run::new("Before the first. ", false)]),
        Block::heading("b2", 1, vec![Run::new("One", false)]),
        Block::paragraph(
            "b3",
            vec![Run::new("Plain, ", false), Run::new("slanted", true)],
        ),
        Block::heading("b4", 2, vec![Run::new("Inside", false)]),
        Block::figure("b5", Figure::new(vec![1, 2, 3], "image/png", "png")),
    ];
    let chapters = doc.chapters();
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[0].title, None);
    assert_eq!(chapters[1].title.as_deref(), Some("One"));
    assert_eq!(chapters[1].blocks.len(), 3);
    assert_eq!(chapters[1].blocks[0].text(), "Plain, slanted");
    assert!(chapters[1].blocks[0].runs()[1].italic);
    assert_eq!(chapters[1].blocks[1].level(), Some(2));
}
