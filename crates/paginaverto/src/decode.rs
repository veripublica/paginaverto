//! A PDF image as gray pixels, whatever its format: what OCR reads.
//!
//! JPEG, raw samples at 1, 2, 4 or 8 bits, JBIG2, JPEG 2000 and CCITT fax. All
//! the decoders are pure Rust. Every failure here is `None`: the caller reports
//! the page, and the rest of the document is still read.

use lopdf::{Dictionary, Document, Object, Stream};

use crate::text::{deref, last_filter};

/// An image in 8-bit gray, row by row, 0 black. Without OCR only whether it
/// decodes matters.
#[cfg_attr(not(feature = "ocr"), allow(dead_code))]
pub(crate) struct Gray {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Gray {
    fn new(width: u32, height: u32, pixels: Vec<u8>, max_pixels: u64) -> Option<Gray> {
        fits(width, height, max_pixels)?;
        (pixels.len() as u64 >= u64::from(width) * u64::from(height)).then_some(Gray {
            width,
            height,
            pixels,
        })
    }

    fn inverted(mut self) -> Gray {
        self.pixels.iter_mut().for_each(|p| *p = 255 - *p);
        self
    }
}

/// A coded image states its own size, which need not be its dictionary's, so
/// `Limits::max_image_pixels` is checked again before anything is allocated.
fn fits(width: u32, height: u32, max_pixels: u64) -> Option<()> {
    (width > 0 && height > 0 && u64::from(width) * u64::from(height) <= max_pixels).then_some(())
}

/// The image's pixels, within `max_pixels`; `None` if it cannot be decoded.
pub(crate) fn gray(doc: &Document, stream: &Stream, max_pixels: u64) -> Option<Gray> {
    let inverted = decode_is_inverted(doc, &stream.dict);
    let image = match last_filter(stream) {
        Some(b"JBIG2Decode") => jbig2(doc, stream, max_pixels),
        Some(b"CCITTFaxDecode") => ccitt(doc, stream, max_pixels),
        Some(b"JPXDecode") => jpx(doc, stream, max_pixels),
        Some(b"DCTDecode") => jpeg(doc, stream, max_pixels),
        Some(b"FlateDecode")
        | Some(b"LZWDecode")
        | Some(b"ASCII85Decode")
        | Some(b"ASCIIHexDecode")
        | Some(b"RunLengthDecode")
        | None => {
            return raw(doc, stream, max_pixels);
        }
        _ => None,
    }?;
    Some(if inverted { image.inverted() } else { image })
}

/// Does `/Decode` turn the samples around? It starts `[1 0`.
fn decode_is_inverted(doc: &Document, dict: &Dictionary) -> bool {
    let number = |o: &Object| o.as_float().ok().or(o.as_i64().ok().map(|i| i as f32));
    dict.get(b"Decode")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_array().ok())
        .is_some_and(|a| {
            matches!(
                (a.first().and_then(number), a.get(1).and_then(number)),
                (Some(lo), Some(hi)) if lo > hi
            )
        })
}

/// The stream's bytes with the filters in front of the last one undone, each with
/// its own `/DecodeParms` entry.
fn before_last_filter(doc: &Document, stream: &Stream) -> Option<Vec<u8>> {
    let chain = match deref(doc, stream.dict.get(b"Filter").ok()?)? {
        Object::Array(chain) if chain.len() > 1 => chain,
        _ => return Some(stream.content.clone()),
    };
    let parms = stream
        .dict
        .get(b"DecodeParms")
        .ok()
        .and_then(|o| deref(doc, o))
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
    Some(data)
}

/// The last filter's `/DecodeParms`, directly or as the last of an array.
fn last_parms<'a>(doc: &'a Document, stream: &'a Stream) -> Option<&'a Dictionary> {
    match deref(doc, stream.dict.get(b"DecodeParms").ok()?)? {
        Object::Array(a) => deref(doc, a.last()?)?.as_dict().ok(),
        o => o.as_dict().ok(),
    }
}

struct Bilevel(Vec<u8>);

impl hayro_jbig2::Decoder for Bilevel {
    fn push_pixel(&mut self, black: bool) {
        self.0.push(if black { 0 } else { 255 });
    }

    fn push_pixel_chunk(&mut self, black: bool, chunks: u32) {
        let value = if black { 0 } else { 255 };
        self.0
            .extend(std::iter::repeat_n(value, chunks as usize * 8));
    }

    fn next_line(&mut self) {}
}

fn jbig2(doc: &Document, stream: &Stream, max_pixels: u64) -> Option<Gray> {
    let settings = hayro_jbig2::DecodeSettings::default();
    let globals = last_parms(doc, stream)
        .and_then(|d| d.get(b"JBIG2Globals").ok())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_stream().ok())
        .map(|s| {
            s.decompressed_content()
                .unwrap_or_else(|_| s.content.clone())
        });
    let globals = match &globals {
        Some(g) => Some(hayro_jbig2::Globals::new(g, &settings).ok()?),
        None => None,
    };
    let data = before_last_filter(doc, stream)?;
    let page = hayro_jbig2::Page::new_embedded(&data, globals.as_ref(), &settings).ok()?;
    let (width, height) = (page.width(), page.height());
    fits(width, height, max_pixels)?;
    let mut out = Bilevel(Vec::with_capacity(width as usize * height as usize));
    page.decode(&mut out).ok()?;
    Gray::new(width, height, out.0, max_pixels)
}

struct FaxRows {
    width: usize,
    black_is_1: bool,
    rows: u32,
    pixels: Vec<u8>,
}

impl hayro_ccitt::Decoder for FaxRows {
    fn push_row(&mut self, changes: &[u32]) {
        // A sample of 1 is white unless /BlackIs1 says otherwise; gray then
        // follows the sample, as DeviceGray does.
        let (white, black) = if self.black_is_1 { (0, 255) } else { (255, 0) };
        let start = self.pixels.len();
        self.pixels.resize(start + self.width, white);
        for span in changes.chunks(2) {
            let from = span[0] as usize;
            let to = span.get(1).map_or(self.width, |&e| e as usize);
            if from < to && to <= self.width {
                self.pixels[start + from..start + to].fill(black);
            }
        }
        self.rows += 1;
    }
}

fn ccitt(doc: &Document, stream: &Stream, max_pixels: u64) -> Option<Gray> {
    let parms = last_parms(doc, stream);
    let int = |key: &[u8]| {
        parms
            .and_then(|d| d.get(key).ok())
            .and_then(|o| o.as_i64().ok())
    };
    let flag = |key: &[u8], default: bool| {
        parms
            .and_then(|d| d.get(key).ok())
            .and_then(|o| o.as_bool().ok())
            .unwrap_or(default)
    };
    let columns = u32::try_from(int(b"Columns").unwrap_or(1728)).ok()?;
    // The decoder takes nearly any bytes for fax, so a row length the image
    // itself contradicts is the one sign that these are not.
    if stream
        .dict
        .get(b"Width")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .is_some_and(|w| w != i64::from(columns))
    {
        return None;
    }
    // /Rows may be absent; then the image's own /Height says.
    let rows = int(b"Rows")
        .filter(|&r| r > 0)
        .or_else(|| stream.dict.get(b"Height").ok()?.as_i64().ok())?;
    let rows = u32::try_from(rows).ok()?;
    fits(columns, rows, max_pixels)?;
    let k = int(b"K").unwrap_or(0);
    let settings = hayro_ccitt::DecodeSettings {
        columns,
        rows,
        end_of_block: flag(b"EndOfBlock", true),
        end_of_line: flag(b"EndOfLine", false),
        rows_are_byte_aligned: flag(b"EncodedByteAlign", false),
        encoding: match k {
            k if k < 0 => hayro_ccitt::EncodingMode::Group4,
            0 => hayro_ccitt::EncodingMode::Group3_1D,
            _ => hayro_ccitt::EncodingMode::Group3_2D,
        },
        strict: false,
    };
    let data = before_last_filter(doc, stream)?;
    let mut out = FaxRows {
        width: columns as usize,
        black_is_1: flag(b"BlackIs1", false),
        rows: 0,
        pixels: Vec::with_capacity(columns as usize * rows as usize),
    };
    let mut ctx = hayro_ccitt::DecoderContext::new(settings);
    // A damaged stream may still have given most of its rows, and a page cut
    // short is better than none. Less than half of them is garbage.
    let decoded = hayro_ccitt::decode(&data, &mut out, &mut ctx);
    if out.rows == 0 || (decoded.is_err() && out.rows < rows / 2) {
        return None;
    }
    let white = if out.black_is_1 { 0 } else { 255 };
    out.pixels.resize(columns as usize * rows as usize, white);
    Gray::new(columns, rows, out.pixels, max_pixels)
}

fn jpx(doc: &Document, stream: &Stream, max_pixels: u64) -> Option<Gray> {
    let data = before_last_filter(doc, stream)?;
    let image =
        hayro_jpeg2000::Image::new(&data, &hayro_jpeg2000::DecodeSettings::default()).ok()?;
    let (width, height) = (image.width(), image.height());
    fits(width, height, max_pixels)?;
    let colors = usize::from(image.color_space().num_channels());
    let step = colors + usize::from(image.has_alpha());
    let mut ctx = hayro_jpeg2000::DecoderContext::default();
    let samples = image.decode(&mut ctx).ok()?.data_u8();
    let pixels = samples
        .chunks_exact(step)
        .map(|p| to_gray(&p[..colors]))
        .collect::<Option<Vec<u8>>>()?;
    Gray::new(width, height, pixels, max_pixels)
}

/// A figure in a format the plain figure path skips, as pixels to embed: the
/// bilevel formats (JBIG2, CCITT, 1-bit and masks) and gray at 2 or 4 bits as
/// gray, exactly; JPEG 2000 in its own colours. Indexed colour and CMYK raw
/// images stay skipped.
pub(crate) fn figure_pixels(
    doc: &Document,
    stream: &Stream,
    max_pixels: u64,
) -> Option<image::DynamicImage> {
    let as_gray = |g: Gray| {
        image::GrayImage::from_raw(g.width, g.height, g.pixels).map(image::DynamicImage::ImageLuma8)
    };
    match last_filter(stream) {
        Some(b"JBIG2Decode") | Some(b"CCITTFaxDecode") => as_gray(gray(doc, stream, max_pixels)?),
        Some(b"JPXDecode") => jpx_colour(doc, stream, max_pixels),
        Some(b"DCTDecode") => None,
        _ => match space(doc, &stream.dict)? {
            Space::Direct(1) => as_gray(gray(doc, stream, max_pixels)?),
            _ => None,
        },
    }
}

/// The image's pixels in colour, within `max_pixels`.
fn colour(doc: &Document, stream: &Stream, max_pixels: u64) -> Option<image::RgbImage> {
    match last_filter(stream) {
        Some(b"JPXDecode") => Some(jpx_colour(doc, stream, max_pixels)?.into_rgb8()),
        Some(b"DCTDecode") => {
            let data = before_last_filter(doc, stream)?;
            let reader = image::ImageReader::with_format(
                std::io::Cursor::new(&data),
                image::ImageFormat::Jpeg,
            );
            let (w, h) = reader.into_dimensions().ok()?;
            fits(w, h, max_pixels)?;
            Some(
                image::load_from_memory_with_format(&data, image::ImageFormat::Jpeg)
                    .ok()?
                    .into_rgb8(),
            )
        }
        _ => {
            let g = gray(doc, stream, max_pixels)?;
            let g = image::GrayImage::from_raw(g.width, g.height, g.pixels)?;
            Some(image::DynamicImage::ImageLuma8(g).into_rgb8())
        }
    }
}

/// A page made of layers, as library scans make them: one image without a mask
/// (the background, often at a lower resolution), and over it one or more
/// images each shown through its soft mask (the text and lines, in their
/// colour). All are assumed to fill the page, as such scans draw them. The page
/// put together at the first layer's size, or `None` if it is not built so.
pub(crate) fn layered_page(
    doc: &Document,
    images: &[&Stream],
    max_pixels: u64,
) -> Option<image::RgbImage> {
    use image::imageops::{self, FilterType};
    let mask_of = |s: &Stream| {
        deref(doc, s.dict.get(b"SMask").ok()?)?
            .as_stream()
            .ok()
            .cloned()
    };
    let backgrounds: Vec<&&Stream> = images.iter().filter(|s| mask_of(s).is_none()).collect();
    let layers: Vec<(&Stream, Stream)> = images
        .iter()
        .filter_map(|s| Some((*s, mask_of(s)?)))
        .collect();
    let ([background], Some((first, _))) = (backgrounds.as_slice(), layers.first()) else {
        return None;
    };
    let (w, h) = (
        u32::try_from(first.dict.get(b"Width").ok()?.as_i64().ok()?).ok()?,
        u32::try_from(first.dict.get(b"Height").ok()?.as_i64().ok()?).ok()?,
    );
    fits(w, h, max_pixels)?;
    let base = colour(doc, background, max_pixels)?;
    let mut page = imageops::resize(&base, w, h, FilterType::Triangle);
    for (layer, mask) in &layers {
        let mut colours = colour(doc, layer, max_pixels)?;
        if colours.dimensions() != (w, h) {
            colours = imageops::resize(&colours, w, h, FilterType::Triangle);
        }
        // A soft mask's sample is how opaque the layer is there, `/Decode` applied.
        let alpha = gray(doc, mask, max_pixels)?;
        let alpha = image::GrayImage::from_raw(alpha.width, alpha.height, alpha.pixels)?;
        let alpha = if alpha.dimensions() == (w, h) {
            alpha
        } else {
            imageops::resize(&alpha, w, h, FilterType::Nearest)
        };
        for (x, y, a) in alpha.enumerate_pixels() {
            if a[0] >= 128 {
                page.put_pixel(x, y, *colours.get_pixel(x, y));
            }
        }
    }
    Some(page)
}

fn jpx_colour(doc: &Document, stream: &Stream, max_pixels: u64) -> Option<image::DynamicImage> {
    let data = before_last_filter(doc, stream)?;
    let image =
        hayro_jpeg2000::Image::new(&data, &hayro_jpeg2000::DecodeSettings::default()).ok()?;
    let (width, height) = (image.width(), image.height());
    fits(width, height, max_pixels)?;
    let colors = usize::from(image.color_space().num_channels());
    let step = colors + usize::from(image.has_alpha());
    let mut ctx = hayro_jpeg2000::DecoderContext::default();
    let samples = image.decode(&mut ctx).ok()?.data_u8();
    let pixels = samples.chunks_exact(step);
    match colors {
        1 => {
            let gray: Vec<u8> = pixels.map(|p| p[0]).collect();
            image::GrayImage::from_raw(width, height, gray).map(image::DynamicImage::ImageLuma8)
        }
        3 | 4 => {
            let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
            for p in pixels {
                if colors == 3 {
                    rgb.extend_from_slice(&p[..3]);
                } else {
                    // CMYK as ink over paper.
                    let k = u32::from(p[3]);
                    rgb.extend((0..3).map(|i| 255 - (u32::from(p[i]) + k).min(255) as u8));
                }
            }
            image::RgbImage::from_raw(width, height, rgb).map(image::DynamicImage::ImageRgb8)
        }
        _ => None,
    }
}

fn jpeg(doc: &Document, stream: &Stream, max_pixels: u64) -> Option<Gray> {
    let data = before_last_filter(doc, stream)?;
    let reader =
        image::ImageReader::with_format(std::io::Cursor::new(&data), image::ImageFormat::Jpeg);
    let (width, height) = reader.into_dimensions().ok()?;
    fits(width, height, max_pixels)?;
    let image = image::load_from_memory_with_format(&data, image::ImageFormat::Jpeg).ok()?;
    let image = image.into_luma8();
    Gray::new(width, height, image.into_raw(), max_pixels)
}

/// One pixel's samples as gray, with the `image` crate's weights for the same
/// result as a JPEG of the same pixels. CMYK as ink over paper.
fn to_gray(p: &[u8]) -> Option<u8> {
    let c = |i: usize| u32::from(p[i]);
    Some(match p.len() {
        1 | 2 => p[0],
        3 => luma(c(0), c(1), c(2)),
        4 => {
            let ink = u32::from(luma(c(0), c(1), c(2))) + c(3);
            255 - ink.min(255) as u8
        }
        _ => return None,
    })
}

fn luma(r: u32, g: u32, b: u32) -> u8 {
    ((r * 2126 + g * 7152 + b * 722) / 10000) as u8
}

/// What a raw image's samples mean.
enum Space {
    /// So many samples to a pixel.
    Direct(usize),
    /// One sample to a pixel, an index into gray values.
    Indexed(Vec<u8>),
}

fn space(doc: &Document, dict: &Dictionary) -> Option<Space> {
    if dict
        .get(b"ImageMask")
        .ok()
        .and_then(|o| o.as_bool().ok())
        .unwrap_or(false)
    {
        return Some(Space::Direct(1));
    }
    space_of(doc, dict.get(b"ColorSpace").ok()?)
}

fn space_of(doc: &Document, object: &Object) -> Option<Space> {
    match deref(doc, object)? {
        Object::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" => Some(Space::Direct(1)),
            b"DeviceRGB" | b"CalRGB" | b"RGB" => Some(Space::Direct(3)),
            b"DeviceCMYK" | b"CMYK" => Some(Space::Direct(4)),
            _ => None,
        },
        Object::Array(a) => match a.first()?.as_name().ok()? {
            b"CalGray" => Some(Space::Direct(1)),
            b"CalRGB" => Some(Space::Direct(3)),
            b"ICCBased" => {
                let icc = deref(doc, a.get(1)?)?.as_stream().ok()?;
                let n = icc.dict.get(b"N").ok()?.as_i64().ok()?;
                matches!(n, 1 | 3 | 4).then_some(Space::Direct(n as usize))
            }
            b"Indexed" | b"I" => {
                let Space::Direct(base) = space_of(doc, a.get(1)?)? else {
                    return None;
                };
                let table = match deref(doc, a.get(3)?)? {
                    Object::String(s, _) => s.clone(),
                    Object::Stream(s) => s
                        .decompressed_content()
                        .unwrap_or_else(|_| s.content.clone()),
                    _ => return None,
                };
                table
                    .chunks_exact(base)
                    .map(to_gray)
                    .collect::<Option<Vec<u8>>>()
                    .map(Space::Indexed)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Raw samples, packed at 1, 2, 4 or 8 bits, each row starting on a byte.
fn raw(doc: &Document, stream: &Stream, max_pixels: u64) -> Option<Gray> {
    let dict = &stream.dict;
    let width = u32::try_from(dict.get(b"Width").ok()?.as_i64().ok()?).ok()?;
    let height = u32::try_from(dict.get(b"Height").ok()?.as_i64().ok()?).ok()?;
    fits(width, height, max_pixels)?;
    let is_mask = dict
        .get(b"ImageMask")
        .ok()
        .and_then(|o| o.as_bool().ok())
        .unwrap_or(false);
    let bits = if is_mask {
        1
    } else {
        dict.get(b"BitsPerComponent")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(8)
    };
    if !matches!(bits, 1 | 2 | 4 | 8) {
        return None;
    }
    let bits = bits as usize;
    let space = space(doc, dict)?;
    let per_pixel = match space {
        Space::Direct(n) => n,
        Space::Indexed(_) => 1,
    };
    let data = stream.decompressed_content().ok()?;
    let row_bytes = (width as usize * per_pixel * bits).div_ceil(8);
    if data.len() < row_bytes * height as usize {
        return None;
    }
    let inverted = decode_is_inverted(doc, dict);
    let top = (1u32 << bits) - 1;
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    let mut samples = [0u8; 4];
    for row in data.chunks_exact(row_bytes).take(height as usize) {
        for x in 0..width as usize {
            for (i, sample) in samples.iter_mut().enumerate().take(per_pixel) {
                let at = (x * per_pixel + i) * bits;
                let byte = u32::from(row[at / 8]);
                let value = (byte >> (8 - bits - at % 8)) & top;
                *sample = match &space {
                    Space::Indexed(_) => value as u8,
                    Space::Direct(_) => {
                        let v = (value * 255 / top) as u8;
                        if inverted { 255 - v } else { v }
                    }
                };
            }
            pixels.push(match &space {
                Space::Indexed(table) => table.get(usize::from(samples[0])).copied().unwrap_or(255),
                Space::Direct(_) => to_gray(&samples[..per_pixel])?,
            });
        }
    }
    Gray::new(width, height, pixels, max_pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn image(dict: Dictionary, data: Vec<u8>) -> Gray {
        gray(&Document::new(), &Stream::new(dict, data), u64::MAX).expect("decodes")
    }

    #[test]
    fn one_bit_rows_start_on_a_byte() {
        let got = image(
            dictionary! { "Width" => 3, "Height" => 2, "BitsPerComponent" => 1, "ColorSpace" => "DeviceGray" },
            vec![0b1010_0000, 0b0100_0000],
        );
        assert_eq!(got.pixels, [255, 0, 255, 0, 255, 0]);
    }

    #[test]
    fn decode_turns_raw_samples_around() {
        let got = image(
            dictionary! {
                "Width" => 2, "Height" => 1, "BitsPerComponent" => 1, "ColorSpace" => "DeviceGray",
                "Decode" => vec![1.into(), 0.into()],
            },
            vec![0b1000_0000],
        );
        assert_eq!(got.pixels, [0, 255]);
    }

    #[test]
    fn an_image_mask_paints_its_zeros_black() {
        let got = image(
            dictionary! { "Width" => 2, "Height" => 1, "ImageMask" => true },
            vec![0b0100_0000],
        );
        assert_eq!(got.pixels, [0, 255]);
    }

    #[test]
    fn an_indexed_image_goes_through_its_table() {
        let palette: Vec<Object> = vec![
            "Indexed".into(),
            "DeviceGray".into(),
            1.into(),
            Object::string_literal(vec![10u8, 200u8]),
        ];
        let got = image(
            dictionary! {
                "Width" => 2, "Height" => 1, "BitsPerComponent" => 1, "ColorSpace" => palette,
            },
            vec![0b0100_0000],
        );
        assert_eq!(got.pixels, [10, 200]);
    }

    #[test]
    fn cmyk_black_is_dark_and_no_ink_is_white() {
        let got = image(
            dictionary! { "Width" => 2, "Height" => 1, "BitsPerComponent" => 8, "ColorSpace" => "DeviceCMYK" },
            vec![0, 0, 0, 255, 0, 0, 0, 0],
        );
        assert_eq!(got.pixels, [0, 255]);
    }

    #[test]
    fn group4_of_blank_rows_is_white() {
        // Group 4: one "vertical 0" code (1) per row keeps the white reference line.
        let got = image(
            dictionary! {
                "Width" => 8, "Height" => 3, "BitsPerComponent" => 1, "ColorSpace" => "DeviceGray",
                "Filter" => "CCITTFaxDecode",
                "DecodeParms" => dictionary! { "K" => -1, "Columns" => 8, "Rows" => 3 },
            },
            vec![0b1110_0000],
        );
        assert_eq!((got.width, got.height), (8, 3));
        assert!(got.pixels.iter().all(|&p| p == 255));
    }

    #[test]
    fn a_one_bit_figure_is_gray_and_an_rgb_one_is_left_to_the_plain_path() {
        let one_bit = Stream::new(
            dictionary! { "Width" => 2, "Height" => 1, "BitsPerComponent" => 1, "ColorSpace" => "DeviceGray" },
            vec![0b1000_0000],
        );
        let got = figure_pixels(&Document::new(), &one_bit, u64::MAX).expect("a figure");
        assert_eq!(got.into_luma8().into_raw(), [255, 0]);
        let indexed: Vec<Object> = vec![
            "Indexed".into(),
            "DeviceRGB".into(),
            0.into(),
            Object::string_literal(vec![1u8, 2, 3]),
        ];
        let colour = Stream::new(
            dictionary! { "Width" => 1, "Height" => 1, "BitsPerComponent" => 8, "ColorSpace" => indexed },
            vec![0],
        );
        assert!(figure_pixels(&Document::new(), &colour, u64::MAX).is_none());
    }

    #[test]
    fn too_many_pixels_are_not_decoded() {
        let stream = Stream::new(
            dictionary! { "Width" => 4, "Height" => 4, "BitsPerComponent" => 8, "ColorSpace" => "DeviceGray" },
            vec![0; 16],
        );
        assert!(gray(&Document::new(), &stream, 15).is_none());
    }

    #[test]
    fn garbage_is_none_in_every_coded_format() {
        for filter in ["JBIG2Decode", "JPXDecode", "DCTDecode"] {
            let stream = Stream::new(
                dictionary! { "Width" => 4, "Height" => 4, "Filter" => filter },
                vec![1, 2, 3, 4],
            );
            assert!(
                gray(&Document::new(), &stream, u64::MAX).is_none(),
                "{filter}"
            );
        }
    }
}
