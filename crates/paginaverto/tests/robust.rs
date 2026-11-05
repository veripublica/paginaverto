//! Limits, cancel, progress and threads, on PDFs generated here.

use std::sync::{Arc, Mutex};

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use paginaverto::{Block, Book, Cancel, Error, Input, LimitKind, Limits, Options, Progress, read};

/// A generated PDF, with `extra` adding to each page's dictionary.
fn pdf(pages: usize, extra: impl Fn(&mut Document, &mut lopdf::Dictionary)) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let mut kids = Vec::new();
    for n in 0..pages {
        // Differs from page to page in its letters, or it reads as a running head.
        let word = |k: usize| char::from(b'a' + ((n / 26usize.pow(k as u32)) % 26) as u8);
        let mark = format!("{}{}{}", word(0), word(1), word(2));
        let text =
            format!("{mark} says something long enough to count as a text layer {mark} ").repeat(4);
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![72.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let c = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Contents" => c,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        };
        extra(&mut doc, &mut page);
        kids.push(doc.add_object(page).into());
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

/// A stream of `bytes` zeros, deflated to almost nothing.
fn bomb(bytes: usize) -> Stream {
    let mut s = Stream::new(dictionary! {}, vec![b' '; bytes]);
    s.compress().unwrap();
    assert!(s.content.len() < bytes / 100);
    s
}

fn with_limits(limits: Limits) -> Options {
    let mut o = Options::default();
    o.limits = limits;
    o
}

fn text(book: &Book) -> Vec<String> {
    book.chapters
        .iter()
        .flat_map(|c| &c.blocks)
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

fn refused(pdf: &[u8], limits: Limits, what: LimitKind) {
    match read(Input::Bytes(pdf), &with_limits(limits)) {
        Err(Error::LimitExceeded { what: got, .. }) => assert_eq!(got, what),
        other => panic!("expected {what:?}, got {other:?}"),
    }
}

#[test]
fn the_defaults_read_an_ordinary_pdf() {
    read(Input::Bytes(&pdf(3, |_, _| {})), &Options::default()).unwrap();
}

#[test]
fn too_large_a_file_is_refused() {
    let bytes = pdf(1, |_, _| {});
    let mut limits = Limits::default();
    limits.max_file_bytes = bytes.len() as u64 - 1;
    refused(&bytes, limits, LimitKind::FileBytes);
    limits.max_file_bytes = bytes.len() as u64;
    read(Input::Bytes(&bytes), &with_limits(limits)).unwrap();
}

#[test]
fn too_many_pages_are_refused() {
    let bytes = pdf(3, |_, _| {});
    let mut limits = Limits::default();
    limits.max_pages = 2;
    refused(&bytes, limits, LimitKind::Pages);
    limits.max_pages = 3;
    read(Input::Bytes(&bytes), &with_limits(limits)).unwrap();
}

#[test]
fn a_decompression_bomb_is_refused_without_decoding_it_all() {
    let bytes = pdf(1, |doc, page| {
        let id = doc.add_object(bomb(4 << 20));
        page.set("Unused", id);
    });
    let mut limits = Limits::default();
    limits.max_stream_bytes = 1 << 20;
    refused(&bytes, limits, LimitKind::StreamBytes);
    limits.max_stream_bytes = 4 << 20;
    read(Input::Bytes(&bytes), &with_limits(limits)).unwrap();
}

#[test]
fn a_stream_listed_many_times_counts_every_time() {
    let bytes = pdf(1, |doc, page| {
        let id = doc.add_object(bomb(1 << 20));
        page.set("Contents", vec![Object::Reference(id); 20]);
    });
    let mut limits = Limits::default();
    limits.max_page_content_bytes = 10 << 20;
    refused(&bytes, limits, LimitKind::PageContentBytes);
}

#[test]
fn a_bomb_in_an_object_stream_is_refused_while_loading() {
    // Object streams are decoded during the load, before any check of ours.
    let mut doc = Document::load_mem(&pdf(1, |_, _| {})).unwrap();
    let padding = doc.add_object(Object::string_literal(" ".repeat(4 << 20)));
    doc.trailer.set("Padding", padding);
    let mut bytes = Vec::new();
    doc.save_modern(&mut bytes).unwrap();
    read(Input::Bytes(&bytes), &Options::default()).unwrap();

    let mut limits = Limits::default();
    limits.max_stream_bytes = 1 << 20;
    match read(Input::Bytes(&bytes), &with_limits(limits)) {
        Err(Error::LimitExceeded { what, .. }) => assert_eq!(what, LimitKind::StreamBytes),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn image(width: i64, height: i64) -> Stream {
    Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => width, "Height" => height,
            "BitsPerComponent" => 8, "ColorSpace" => "DeviceGray",
        },
        vec![0; 16],
    )
}

fn with_image(width: i64, height: i64) -> Vec<u8> {
    pdf(1, |doc, _| {
        doc.add_object(image(width, height));
    })
}

#[test]
fn too_many_pixels_are_refused() {
    let mut limits = Limits::default();
    limits.max_image_pixels = 1000;
    refused(&with_image(1000, 2), limits, LimitKind::ImagePixels);
}

#[test]
fn absurd_image_sizes_do_not_overflow() {
    let huge = 1i64 << 40;
    refused(
        &with_image(huge, huge),
        Limits::default(),
        LimitKind::ImagePixels,
    );
    read(Input::Bytes(&with_image(-huge, huge)), &Options::default()).unwrap();
}

#[test]
fn deep_nesting_does_not_abort() {
    // A stack overflow could not be caught, so reaching the end is the test.
    let deep = format!("{}{}", "[".repeat(200_000), "]".repeat(200_000));
    let bytes = pdf(4, |doc, page| {
        let id = doc.add_object(Stream::new(dictionary! {}, deep.clone().into_bytes()));
        page.set("Contents", id);
    });
    let mut opts = Options::default();
    opts.threads = Some(2);
    let _ = read(Input::Bytes(&bytes), &opts);
}

#[test]
fn deep_nesting_in_an_object_does_not_abort() {
    // The same length as the string it replaces, so the offsets stay right.
    let depth = 100_000;
    let bytes = pdf(1, |doc, page| {
        let id = doc.add_object(Object::string_literal("x".repeat(2 * depth - 2)));
        page.set("Deep", id);
    });
    let at = bytes.windows(9).position(|w| w == b"xxxxxxxxx").unwrap() - 1;
    let mut bytes = bytes;
    let nested = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    bytes[at..at + 2 * depth].copy_from_slice(nested.as_bytes());
    let _ = read(Input::Bytes(&bytes), &Options::default());
}

#[test]
fn cancelled_before_it_starts() {
    let opts = Options::default();
    opts.cancel.cancel();
    assert_eq!(
        read(Input::Bytes(&pdf(3, |_, _| {})), &opts).unwrap_err(),
        Error::Cancelled
    );
}

#[test]
fn cancelled_halfway_stops_reading() {
    for threads in [1, 4] {
        let cancel = Cancel::new();
        let seen = Arc::new(Mutex::new(0));
        let mut opts = Options::default();
        opts.threads = Some(threads);
        opts.cancel = cancel.clone();
        let count = seen.clone();
        opts.progress = Some(Progress::new(move |p| {
            *count.lock().unwrap() = p.done;
            if p.done == 5 {
                cancel.cancel();
            }
        }));
        assert_eq!(
            read(Input::Bytes(&pdf(400, |_, _| {})), &opts).unwrap_err(),
            Error::Cancelled
        );
        assert!(*seen.lock().unwrap() < 400, "threads {threads}");
    }
}

#[test]
fn progress_counts_every_page_once() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut opts = Options::default();
    opts.threads = Some(4);
    let log = seen.clone();
    opts.progress = Some(Progress::new(move |p| log.lock().unwrap().push(p)));
    read(Input::Bytes(&pdf(50, |_, _| {})), &opts).unwrap();

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 50);
    assert!(seen.iter().enumerate().all(|(i, p)| p.done == i + 1));
    assert!(seen.iter().all(|p| p.total == 50));
    let mut pages: Vec<_> = seen.iter().map(|p| p.page).collect();
    pages.sort_unstable();
    assert_eq!(pages, (1..=50).collect::<Vec<u32>>());
}

#[test]
fn a_panicking_reporter_comes_back_as_an_error() {
    let mut opts = Options::default();
    opts.threads = Some(4);
    opts.progress = Some(Progress::new(|_| panic!("reporter failed")));
    let error = read(Input::Bytes(&pdf(50, |_, _| {})), &opts).unwrap_err();
    assert_eq!(error, Error::Internal("reporter failed".into()));
}

#[test]
fn the_thread_count_does_not_change_the_output() {
    let bytes = pdf(120, |_, _| {});
    let run = |threads| {
        let mut opts = Options::default();
        opts.threads = Some(threads);
        text(&read(Input::Bytes(&bytes), &opts).unwrap())
    };
    let one = run(1);
    let words = one.iter().map(|p| p.split(' ').count()).sum::<usize>();
    assert!(words > 120 * 40, "{words} words");
    for threads in [0, 2, 3, 8, 1000] {
        assert_eq!(run(threads), one, "threads {threads}");
    }
}
