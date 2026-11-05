//! The public accuracy set, OCR path: known text rendered to page images, wrapped
//! in a PDF as a scan, read back through OCR, and compared.
//!
//!     cargo run --release -- <models dir> <font.ttf> [<font.ttf> ...]
//!
//! The models dir holds `text-detection.{onnx,rten}` and
//! `text-recognition.{onnx,rten}`. Prints the character error rate per language,
//! font and page quality, exact and with the Turkish letters folded to ASCII,
//! which shows what the ASCII-only model loses to its alphabet alone. Also the
//! paragraphs found against the corpus's own.

use std::path::Path;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use image::{GrayImage, Luma};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use paginaverto::{Input, Ocr, Options, read};

const EN: &str = include_str!("../../../crates/paginaverto/tests/corpus/en.txt");
const TR: &str = include_str!("../../../crates/paginaverto/tests/corpus/tr.txt");

/// A4 at 200 dpi, 11 pt text.
const PAGE: (u32, u32) = (1654, 2339);
const MARGIN: f32 = 200.0;
const PX: f32 = 31.0;
const LEADING: f32 = 44.0;

#[derive(Clone, Copy, Debug)]
enum Quality {
    Clean,
    /// A grey page with sensor noise, as a phone or a cheap scanner gives.
    Noisy,
}

/// The corpus as page images.
fn render(corpus: &str, font: &FontVec, quality: Quality) -> Vec<GrayImage> {
    let scaled = font.as_scaled(PxScale::from(PX));
    let width = |s: &str| {
        s.chars()
            .map(|c| scaled.h_advance(scaled.glyph_id(c)))
            .sum::<f32>()
    };
    let room = PAGE.0 as f32 - 2.0 * MARGIN;
    let mut lines: Vec<(String, bool)> = Vec::new();
    for paragraph in corpus
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let mut line = String::new();
        let mut first = true;
        for word in paragraph.split_whitespace() {
            let next = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if width(&next) > room {
                lines.push((std::mem::take(&mut line), first));
                first = false;
                line = word.to_string();
            } else {
                line = next;
            }
        }
        lines.push((line, first));
    }
    let per_page = ((PAGE.1 as f32 - 2.0 * MARGIN) / LEADING) as usize;
    let (paper, ink) = match quality {
        Quality::Clean => (255.0, 0.0),
        Quality::Noisy => (225.0, 40.0),
    };
    let mut seed = 12345u32;
    lines
        .chunks(per_page)
        .map(|chunk| {
            let mut coverage = vec![0f32; (PAGE.0 * PAGE.1) as usize];
            for (i, (line, first)) in chunk.iter().enumerate() {
                let mut x = MARGIN + if *first { PX * 2.0 } else { 0.0 };
                let y = MARGIN + i as f32 * LEADING;
                for c in line.chars() {
                    let id = scaled.glyph_id(c);
                    let glyph = id.with_scale_and_position(PxScale::from(PX), point(x, y));
                    if let Some(outline) = font.outline_glyph(glyph) {
                        let b = outline.px_bounds();
                        outline.draw(|gx, gy, v| {
                            let (px, py) = (b.min.x as u32 + gx, b.min.y as u32 + gy);
                            if px < PAGE.0 && py < PAGE.1 {
                                let cell = &mut coverage[(py * PAGE.0 + px) as usize];
                                *cell = cell.max(v);
                            }
                        });
                    }
                    x += scaled.h_advance(id);
                }
            }
            GrayImage::from_fn(PAGE.0, PAGE.1, |x, y| {
                let v = coverage[(y * PAGE.0 + x) as usize];
                let mut level = paper + (ink - paper) * v;
                if matches!(quality, Quality::Noisy) {
                    // A fixed sequence, so that every run sees the same page.
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    level += ((seed >> 24) as f32 / 255.0 - 0.5) * 30.0;
                }
                Luma([level.clamp(0.0, 255.0) as u8])
            })
        })
        .collect()
}

/// Page images as a PDF with no text layer.
fn scan_pdf(pages: &[GrayImage]) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut kids: Vec<Object> = Vec::new();
    for image in pages {
        let xobject = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => i64::from(image.width()),
                "Height" => i64::from(image.height()),
                "ColorSpace" => "DeviceGray",
                "BitsPerComponent" => 8,
            },
            image.as_raw().clone(),
        ));
        let ops = vec![
            Operation::new("q", vec![]),
            Operation::new(
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
            Operation::new("Do", vec!["Im1".into()]),
            Operation::new("Q", vec![]),
        ];
        let content = Content { operations: ops }.encode().expect("encodes");
        let c = doc.add_object(Stream::new(dictionary! {}, content));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Contents" => c,
            "Resources" => dictionary! { "XObject" => dictionary! { "Im1" => xobject } },
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

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fold_turkish(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ş' => 's',
            'Ş' => 'S',
            'ğ' => 'g',
            'Ğ' => 'G',
            'ı' => 'i',
            'İ' => 'I',
            'ç' => 'c',
            'Ç' => 'C',
            'ö' => 'o',
            'Ö' => 'O',
            'ü' => 'u',
            'Ü' => 'U',
            'â' => 'a',
            c => c,
        })
        .collect()
}

fn cer(got: &str, want: &str) -> f64 {
    let (g, w): (Vec<char>, Vec<char>) = (got.chars().collect(), want.chars().collect());
    100.0 * distance(&g, &w) as f64 / w.len() as f64
}

fn model(dir: &Path, stem: &str) -> Vec<u8> {
    ["onnx", "rten"]
        .iter()
        .find_map(|ext| std::fs::read(dir.join(format!("{stem}.{ext}"))).ok())
        .unwrap_or_else(|| panic!("no {stem} model in {}", dir.display()))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [models, fonts @ ..] = args.as_slice() else {
        panic!("usage: ocr-accuracy <models dir> <font.ttf> [<font.ttf> ...]");
    };
    let models = Path::new(models);
    let mut opts = Options::default();
    opts.language = Some("en".into());
    opts.ocr = Ocr::Models {
        detection: model(models, "text-detection"),
        recognition: model(models, "text-recognition"),
    };
    for font_path in fonts {
        let font = FontVec::try_from_vec(std::fs::read(font_path).expect("reads the font"))
            .expect("a font");
        let name = Path::new(font_path).file_stem().unwrap().to_string_lossy();
        for (lang, corpus) in [("en", EN), ("tr", TR)] {
            for quality in [Quality::Clean, Quality::Noisy] {
                let pages = render(corpus, &font, quality);
                let pdf = scan_pdf(&pages);
                // The same pages as page images: a folder of PNGs and one
                // multi-page TIFF must read as the PDF does.
                let folder =
                    std::env::temp_dir().join(format!("ocr-accuracy-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&folder);
                std::fs::create_dir_all(&folder).expect("a temporary folder");
                for (i, page) in pages.iter().enumerate() {
                    page.save(folder.join(format!("page{}.png", i + 1)))
                        .expect("writes a PNG");
                }
                let tiff = folder.join("all.tif");
                {
                    let mut out = std::fs::File::create(&tiff).expect("writes a TIFF");
                    let mut encoder = tiff::encoder::TiffEncoder::new(&mut out).expect("a TIFF");
                    for page in &pages {
                        encoder
                            .write_image::<tiff::encoder::colortype::Gray8>(
                                page.width(),
                                page.height(),
                                page.as_raw(),
                            )
                            .expect("a TIFF page");
                    }
                }
                let text_of = |doc: paginaverto::Document| {
                    squash(
                        &doc.blocks
                            .iter()
                            .map(|b| b.text())
                            .collect::<Vec<_>>()
                            .join(" "),
                    )
                };
                let tiff_text = read(Input::Path(&tiff), &opts).map(text_of);
                std::fs::remove_file(&tiff).expect("removes the TIFF");
                let png_text = read(Input::Path(&folder), &opts).map(text_of);
                let _ = std::fs::remove_dir_all(&folder);
                let pdf_text = read(Input::Bytes(&pdf), &opts).map(text_of);
                let same = |r: &Result<String, paginaverto::Error>| match (r, &pdf_text) {
                    (Ok(a), Ok(b)) if a == b => "same",
                    _ => "DIFFERENT",
                };
                let images = format!("PNG folder {}, TIFF {}", same(&png_text), same(&tiff_text));
                let (got, found) = match read(Input::Bytes(&pdf), &opts) {
                    Ok(doc) => {
                        let mut text = String::new();
                        let mut found = 0;
                        for b in doc.blocks.iter().filter(|b| !b.runs().is_empty()) {
                            text.push_str(&b.text());
                            text.push(' ');
                            found += 1;
                        }
                        (squash(&text), found)
                    }
                    Err(e) => {
                        println!("{lang} {name} {quality:?}: refused: {e}");
                        continue;
                    }
                };
                let want = squash(corpus);
                println!(
                    "{lang} {name} {quality:?}: CER {:.2}%, folded {:.2}%, paragraphs {found} of {}; {images}",
                    cer(&got, &want),
                    cer(&fold_turkish(&got), &fold_turkish(&want)),
                    corpus
                        .split("\n\n")
                        .filter(|p| !p.trim().is_empty())
                        .count(),
                );
            }
        }
    }
}
