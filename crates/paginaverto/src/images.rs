//! Page images as input: a PNG, JPEG or TIFF file (every page of a multi-page
//! TIFF), or a folder of them, read in natural name order. Each image is one
//! page, read through OCR.
//!
//! The format is found from the bytes, never from the name, so `read` takes an
//! image wherever it takes a PDF.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use crate::decode::Gray;
use crate::{Error, Input, Limits};

/// A page image's resolution when the file states none.
pub(crate) const DEFAULT_DPI: f32 = 300.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Png,
    Jpeg,
    Tiff,
}

impl Kind {
    pub(crate) fn of(bytes: &[u8]) -> Option<Kind> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Kind::Png)
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            Some(Kind::Jpeg)
        } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
            Some(Kind::Tiff)
        } else {
            None
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Png => "PNG",
            Kind::Jpeg => "JPEG",
            Kind::Tiff => "TIFF",
        }
    }
}

/// The input's image files, whole; a page is one of them, or one frame of a TIFF.
pub(crate) struct Images {
    files: Vec<Vec<u8>>,
    /// Each page: its file, and its frame in that file.
    pub pages: Vec<(usize, usize)>,
}

/// One decoded page.
pub(crate) struct Picture {
    pub gray: Gray,
    /// Dots per inch, across and down.
    pub dpi: (f32, f32),
}

impl Picture {
    /// The page's size in points, as a PDF page's.
    pub(crate) fn points(&self) -> (f32, f32) {
        (
            self.gray.width as f32 * 72.0 / self.dpi.0,
            self.gray.height as f32 * 72.0 / self.dpi.1,
        )
    }
}

/// The input as page images, or `None` if it is not one (it is then a PDF, or
/// nothing paginaverto reads). A folder is always images, and an error if it
/// holds none.
pub(crate) fn open(input: Input<'_>, limits: &Limits) -> Result<Option<Images>, Error> {
    let files = match input {
        Input::Bytes(bytes) => match Kind::of(bytes) {
            Some(_) => vec![bytes.to_vec()],
            None => return Ok(None),
        },
        Input::Path(path) if path.is_dir() => {
            let names = image_files(path)?;
            if names.is_empty() {
                return Err(Error::Open(format!(
                    "{} holds no PNG, JPEG or TIFF file",
                    path.display()
                )));
            }
            let mut total = 0u64;
            let mut files = Vec::with_capacity(names.len());
            for name in names {
                total += std::fs::metadata(&name).map_or(0, |m| m.len());
                if total > limits.max_file_bytes {
                    return Err(Limits::exceeded(
                        crate::LimitKind::FileBytes,
                        limits.max_file_bytes,
                    ));
                }
                let bytes = std::fs::read(&name)
                    .map_err(|e| Error::Open(format!("{}: {e}", name.display())))?;
                if Kind::of(&bytes).is_none() {
                    return Err(Error::Open(format!(
                        "{} is not a PNG, JPEG or TIFF image",
                        name.display()
                    )));
                }
                files.push(bytes);
            }
            files
        }
        Input::Path(path) => {
            if !starts_as_image(path) {
                return Ok(None);
            }
            vec![std::fs::read(path).map_err(|e| Error::Open(e.to_string()))?]
        }
    };
    let mut pages = Vec::new();
    for (i, bytes) in files.iter().enumerate() {
        let frames = match Kind::of(bytes) {
            Some(Kind::Tiff) => tiff_frames(bytes),
            _ => 1,
        };
        pages.extend((0..frames).map(|frame| (i, frame)));
        if pages.len() as u64 > limits.max_pages {
            return Err(Limits::exceeded(crate::LimitKind::Pages, limits.max_pages));
        }
    }
    Ok(Some(Images { files, pages }))
}

fn starts_as_image(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 8];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && Kind::of(&head).is_some()
}

/// A folder's image files by their extension, in natural order: `page2`
/// before `page10`. Hidden files are left out.
fn image_files(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::Open(e.to_string()))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            name.is_some_and(|n| !n.starts_with('.'))
        })
        .filter(|p| {
            p.extension().is_some_and(|e| {
                let e = e.to_string_lossy().to_ascii_lowercase();
                matches!(e.as_str(), "png" | "jpg" | "jpeg" | "tif" | "tiff")
            })
        })
        .collect();
    files.sort_by(|a, b| natural(&a.to_string_lossy(), &b.to_string_lossy()));
    Ok(files)
}

/// Names compared with their runs of digits as numbers.
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.chars() {
            let digit = c.is_ascii_digit();
            match out.last_mut() {
                Some((d, run)) if *d == digit => run.push(c),
                _ => out.push((digit, c.to_string())),
            }
        }
        out
    }
    let (pa, pb) = (parts(a), parts(b));
    for (x, y) in pa.iter().zip(&pb) {
        let order = match (x, y) {
            ((true, x), (true, y)) => {
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                x.len().cmp(&y.len()).then_with(|| x.cmp(y))
            }
            ((_, x), (_, y)) => x.cmp(y),
        };
        if order.is_ne() {
            return order;
        }
    }
    pa.len().cmp(&pb.len()).then_with(|| a.cmp(b))
}

fn tiff_frames(bytes: &[u8]) -> usize {
    let Ok(mut decoder) = tiff::decoder::Decoder::new(Cursor::new(bytes)) else {
        // Counted as one page, which will be reported as undecodable.
        return 1;
    };
    let mut frames = 1;
    while decoder.more_images() && decoder.next_image().is_ok() {
        frames += 1;
    }
    frames
}

impl Images {
    /// The format of a page's file, to report.
    pub(crate) fn kind(&self, page: usize) -> Option<Kind> {
        let (file, _) = *self.pages.get(page)?;
        Kind::of(&self.files[file])
    }

    /// A page's pixels, within `max_pixels`; `None` if it cannot be decoded.
    pub(crate) fn picture(&self, page: usize, max_pixels: u64) -> Option<Picture> {
        let (file, frame) = *self.pages.get(page)?;
        let bytes = &self.files[file];
        match Kind::of(bytes)? {
            Kind::Tiff => tiff_page(bytes, frame, max_pixels),
            kind => {
                let format = match kind {
                    Kind::Png => image::ImageFormat::Png,
                    _ => image::ImageFormat::Jpeg,
                };
                let reader = image::ImageReader::with_format(Cursor::new(bytes), format);
                let (w, h) = reader.into_dimensions().ok()?;
                if u64::from(w) * u64::from(h) > max_pixels {
                    return None;
                }
                let image = image::load_from_memory_with_format(bytes, format).ok()?;
                let gray = image.into_luma8();
                let (w, h) = gray.dimensions();
                let dpi = match kind {
                    Kind::Png => png_dpi(bytes),
                    _ => jpeg_dpi(bytes),
                };
                Some(Picture {
                    gray: Gray {
                        width: w,
                        height: h,
                        pixels: gray.into_raw(),
                    },
                    dpi: dpi.unwrap_or((DEFAULT_DPI, DEFAULT_DPI)),
                })
            }
        }
    }
}

/// `pHYs`: pixels per unit, across and down; unit 1 is the metre.
fn png_dpi(bytes: &[u8]) -> Option<(f32, f32)> {
    let mut at = 8;
    while at + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().ok()?) as usize;
        let kind = &bytes[at + 4..at + 8];
        let data = bytes.get(at + 8..at + 8 + len)?;
        if kind == b"pHYs" && len == 9 && data[8] == 1 {
            let x = u32::from_be_bytes(data[0..4].try_into().ok()?) as f32;
            let y = u32::from_be_bytes(data[4..8].try_into().ok()?) as f32;
            return sane((x * 0.0254, y * 0.0254));
        }
        if kind == b"IDAT" {
            return None;
        }
        at += 12 + len;
    }
    None
}

/// JFIF's `APP0`: density units 1 (per inch) or 2 (per centimetre).
fn jpeg_dpi(bytes: &[u8]) -> Option<(f32, f32)> {
    let app0 = bytes.get(2..20)?;
    if app0[0] != 0xff || app0[1] != 0xe0 || &app0[4..9] != b"JFIF\0" {
        return None;
    }
    let x = f32::from(u16::from_be_bytes([app0[12], app0[13]]));
    let y = f32::from(u16::from_be_bytes([app0[14], app0[15]]));
    match app0[11] {
        1 => sane((x, y)),
        2 => sane((x * 2.54, y * 2.54)),
        _ => None,
    }
}

/// A resolution worth believing: files often carry 1 or 72 by default.
fn sane((x, y): (f32, f32)) -> Option<(f32, f32)> {
    let ok = |d: f32| (75.0..=2400.0).contains(&d);
    (ok(x) && ok(y)).then_some((x, y))
}

fn tiff_page(bytes: &[u8], frame: usize, max_pixels: u64) -> Option<Picture> {
    use tiff::ColorType;
    use tiff::decoder::{Decoder, DecodingResult};
    use tiff::tags::Tag;

    let mut decoder = Decoder::new(Cursor::new(bytes)).ok()?;
    decoder.seek_to_image(frame).ok()?;
    let (width, height) = decoder.dimensions().ok()?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > max_pixels {
        return None;
    }
    let colour = decoder.colortype().ok()?;
    // PhotometricInterpretation 0: WhiteIsZero, as fax pages often are.
    let white_is_zero = decoder
        .find_tag_unsigned::<u16>(Tag::PhotometricInterpretation)
        .ok()
        .flatten()
        == Some(0);
    let dpi = tiff_dpi(&mut decoder);
    let data = decoder.read_image().ok()?;
    let (w, h) = (width as usize, height as usize);
    let samples: Vec<u8> = match data {
        DecodingResult::U8(v) => v,
        DecodingResult::U16(v) => v.into_iter().map(|s| (s >> 8) as u8).collect(),
        _ => return None,
    };
    let (channels, bits) = match colour {
        ColorType::Gray(b) => (1, b),
        ColorType::GrayA(b) => (2, b),
        ColorType::RGB(b) | ColorType::YCbCr(b) => (3, b),
        ColorType::RGBA(b) => (4, b),
        ColorType::CMYK(b) => (4, b),
        _ => return None,
    };
    let cmyk = matches!(colour, ColorType::CMYK(_));
    let mut pixels = Vec::with_capacity(w * h);
    if bits < 8 {
        // Packed samples, gray only, each row starting on a byte.
        if channels != 1 {
            return None;
        }
        let bits = usize::from(bits);
        let row = (w * bits).div_ceil(8);
        let top = (1u32 << bits) - 1;
        for line in samples.chunks(row).take(h) {
            for x in 0..w {
                let at = x * bits;
                let byte = u32::from(*line.get(at / 8)?);
                let v = (byte >> (8 - bits - at % 8)) & top;
                pixels.push((v * 255 / top) as u8);
            }
        }
    } else {
        for p in samples.chunks_exact(channels).take(w * h) {
            let c = |i: usize| u32::from(p[i]);
            let luma = |r: u32, g: u32, b: u32| ((r * 2126 + g * 7152 + b * 722) / 10000) as u8;
            pixels.push(match (channels, cmyk) {
                (1 | 2, _) => p[0],
                (4, true) => {
                    let ink = u32::from(luma(c(0), c(1), c(2))) + c(3);
                    255 - ink.min(255) as u8
                }
                _ => luma(c(0), c(1), c(2)),
            });
        }
    }
    if pixels.len() < w * h {
        return None;
    }
    if white_is_zero {
        pixels.iter_mut().for_each(|p| *p = 255 - *p);
    }
    Some(Picture {
        gray: Gray {
            width,
            height,
            pixels,
        },
        dpi: dpi.unwrap_or((DEFAULT_DPI, DEFAULT_DPI)),
    })
}

fn tiff_dpi<R: std::io::Read + std::io::Seek>(
    decoder: &mut tiff::decoder::Decoder<R>,
) -> Option<(f32, f32)> {
    use tiff::tags::Tag;
    let x = decoder.get_tag_f32(Tag::XResolution).ok()?;
    let y = decoder.get_tag_f32(Tag::YResolution).ok()?;
    // ResolutionUnit: 2 inch (the default), 3 centimetre, 1 none.
    match decoder
        .find_tag_unsigned::<u16>(Tag::ResolutionUnit)
        .ok()
        .flatten()
    {
        None | Some(2) => sane((x, y)),
        Some(3) => sane((x * 2.54, y * 2.54)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_with_their_numbers() {
        let mut names = vec!["p10.png", "p2.png", "p1.png", "p02b.png", "a.png"];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(names, ["a.png", "p1.png", "p2.png", "p02b.png", "p10.png"]);
    }

    #[test]
    fn formats_are_found_from_the_bytes() {
        assert_eq!(Kind::of(b"\x89PNG\r\n\x1a\n...."), Some(Kind::Png));
        assert_eq!(Kind::of(&[0xff, 0xd8, 0xff, 0xe0]), Some(Kind::Jpeg));
        assert_eq!(Kind::of(b"II*\0...."), Some(Kind::Tiff));
        assert_eq!(Kind::of(b"MM\0*...."), Some(Kind::Tiff));
        assert_eq!(Kind::of(b"%PDF-1.7"), None);
    }

    #[test]
    fn a_resolution_of_72_or_1_is_not_believed() {
        assert_eq!(sane((72.0, 72.0)), None);
        assert_eq!(sane((1.0, 1.0)), None);
        assert_eq!(sane((300.0, 300.0)), Some((300.0, 300.0)));
    }

    fn png(w: u32, h: u32, dpm: Option<u32>) -> Vec<u8> {
        let img = image::GrayImage::from_pixel(w, h, image::Luma([200]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        if let Some(dpm) = dpm {
            // pHYs goes before IDAT: after the 8-byte signature and the 25-byte IHDR.
            let mut chunk = Vec::new();
            chunk.extend_from_slice(&9u32.to_be_bytes());
            chunk.extend_from_slice(b"pHYs");
            chunk.extend_from_slice(&dpm.to_be_bytes());
            chunk.extend_from_slice(&dpm.to_be_bytes());
            chunk.push(1);
            chunk.extend_from_slice(&[0, 0, 0, 0]); // CRC, unchecked here
            out.splice(33..33, chunk);
        }
        out
    }

    #[test]
    fn a_png_page_gets_its_size_from_its_resolution() {
        let bytes = png(600, 300, Some(7874)); // 200 dpi
        let images = open(Input::Bytes(&bytes), &Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(images.pages, [(0, 0)]);
        let picture = images.picture(0, u64::MAX).unwrap();
        let (w, h) = picture.points();
        assert!(
            (w - 216.0).abs() < 0.5 && (h - 108.0).abs() < 0.5,
            "{w} {h}"
        );
    }

    #[test]
    fn without_a_resolution_a_page_is_taken_at_300_dpi() {
        let bytes = png(600, 300, None);
        let images = open(Input::Bytes(&bytes), &Limits::default())
            .unwrap()
            .unwrap();
        let (w, _) = images.picture(0, u64::MAX).unwrap().points();
        assert!((w - 144.0).abs() < 0.5, "{w}");
    }

    #[test]
    fn a_pdf_is_not_an_image() {
        let got = open(Input::Bytes(b"%PDF-1.7\n"), &Limits::default()).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn too_many_pixels_are_not_decoded() {
        let bytes = png(10, 10, None);
        let images = open(Input::Bytes(&bytes), &Limits::default())
            .unwrap()
            .unwrap();
        assert!(images.picture(0, 99).is_none());
    }
}
