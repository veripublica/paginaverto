//! Character recognition for paginaverto: a page image in, lines of text out.
//!
//! This crate is the OCR side of paginaverto's seam. The PDF side and the
//! structure step only ever see [`Line`]s, whichever produced them.
//!
//! Lines are found by ocrs 0.13.1 on rten 0.26. They are read by PaddlePaddle's
//! PP-OCRv5 Latin model ([`LATIN_RECOGNITION`]), whose alphabet holds Turkish
//! and the other Latin-script languages, or by ocrs's own model, which knows
//! ASCII only and gives the same text as paginaverto 0.2. The models are not
//! part of this crate; the caller supplies their bytes, and which recognition
//! model it is is detected from the model itself.

#![warn(missing_docs)]

use std::fmt;

use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use rten::Model;
use rten_imageproc::{BoundingRect, RotatedRect};
use rten_tensor::{AsView, Layout};

/// Where the ocrs project publishes its models.
///
/// Fetch [`ONNX_MODELS`] from here. Since at least 2026-10-31 the bucket
/// answers 403 for the `.rten` files of [`MODELS`], which only a copy you
/// already have can provide.
pub const MODEL_BASE_URL: &str = "https://ocrs-models.s3-accelerate.amazonaws.com";

/// One model file the caller fetches from [`MODEL_BASE_URL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
    /// The file name, also its path under [`MODEL_BASE_URL`].
    pub name: &'static str,
    /// SHA-256 of the file, lowercase hex. Check it before loading.
    pub sha256: &'static str,
}

/// The detection model, then ocrs's own recognition model, in ONNX form: the
/// files ocrs serves. Its recognition model reads ASCII only; prefer
/// [`LATIN_RECOGNITION`]. CC BY-SA 4.0 (trained on HierText); whoever redistributes them
/// gives the attribution their license asks for.
///
/// On 2026-10-31 they gave byte-identical text to [`MODELS`] on every scan
/// measured, at the same speed.
pub const ONNX_MODELS: [ModelFile; 2] = [
    ModelFile {
        name: "text-detection.onnx",
        sha256: "a917b23dbd9524b465df7e922641b3ff2981623df4ded5a0234004ef2fee7cfe",
    },
    ModelFile {
        name: "text-recognition.onnx",
        sha256: "86c145c2edb96c8caed5b1ebb8f44d706408922211451c309c625157dd6061c5",
    },
];

/// The recognition model to use: PaddlePaddle's PP-OCRv5 Latin model, in ONNX
/// form. Apache-2.0. Its 836 characters cover Turkish (ç ğ ı İ ö ş ü â î û) and
/// the other Latin-script languages, with digits and punctuation. Pass it as the
/// recognition model with [`ONNX_MODELS`]'s detection model.
///
/// Fetch it from [`LATIN_RECOGNITION_URL`] and save it under this name.
pub const LATIN_RECOGNITION: ModelFile = ModelFile {
    name: "latin_PP-OCRv5_mobile_rec.onnx",
    sha256: "7888113072263cb471b93f66dd5e2ad70548dc526fa1ace760d0d973dd121498",
};

/// Where [`LATIN_RECOGNITION`] is published, pinned to the upstream commit
/// whose file has that SHA-256 (PaddlePaddle on Hugging Face, as
/// `inference.onnx`).
pub const LATIN_RECOGNITION_URL: &str = "https://huggingface.co/PaddlePaddle/latin_PP-OCRv5_mobile_rec_onnx/resolve/89d3a50e2c27e2e7cceeab0e944c25c807d5db4f/inference.onnx";

/// The same two models in the RTen form epublift downloaded until 2026-10
/// (copies of 2026-10-26). No longer served (403); kept so that an existing
/// cache still checks. Prefer [`ONNX_MODELS`].
pub const MODELS: [ModelFile; 2] = [
    ModelFile {
        name: "text-detection.rten",
        sha256: "f15cfb56bd02c4bf478a20343986504a1f01e1665c2b3a0ad66340f054b1b5ca",
    },
    ModelFile {
        name: "text-recognition.rten",
        sha256: "e484866d4cce403175bd8d00b128feb08ab42e208de30e42cd9889d8f1735a6e",
    },
];

/// A recognised line of text.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Line {
    /// The line's text, as the recognition model gives it.
    pub text: String,
    /// Where the line sits on the page image, in pixels: the box around its
    /// characters. `None` for a line without characters.
    pub bbox: Option<Rect>,
}

/// An axis-aligned box in image pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// Why recognition failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A model's bytes did not load, or the engine did not start.
    Models(String),
    /// The page image could not be decoded.
    Image(String),
    /// Recognition itself failed.
    Recognise(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Models(e) => write!(f, "OCR models: {e}"),
            Error::Image(e) => write!(f, "page image: {e}"),
            Error::Recognise(e) => write!(f, "recognition: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// A loaded recogniser. Build it once and use it for every page.
pub struct Engine {
    detector: OcrEngine,
    /// A PP-OCR recognition model, which ocrs cannot run itself.
    paddle: Option<Paddle>,
}

/// The characters of the PP-OCRv5 Latin model, in the order of its output.
const LATIN_DICT: &str = include_str!("latin_dict.txt");

/// Every character [`LATIN_RECOGNITION`] can give, besides the space.
pub fn latin_alphabet() -> Vec<char> {
    LATIN_DICT.lines().flat_map(str::chars).collect()
}

struct Paddle {
    model: Model,
    /// Whether the text is Turkish (or Azerbaijani), where I and İ, ı and i are
    /// four letters.
    turkish: bool,
    turkish_pairs: Vec<(usize, usize)>,
    /// Output label to character: 0 is CTC's blank, the last is a space.
    labels: Vec<String>,
}

impl Paddle {
    /// Is this a PP-OCR recognition model? Its input is `[n, 3, 48, width]`;
    /// ocrs's has one channel.
    fn accepts(model: &Model) -> bool {
        let Some(id) = model.input_ids().first() else {
            return false;
        };
        let Some(shape) = model.node_info(*id).and_then(|n| n.shape()) else {
            return false;
        };
        shape.len() == 4
            && matches!(shape[1], rten::Dimension::Fixed(3))
            && matches!(shape[2], rten::Dimension::Fixed(48))
    }

    fn new(model: Model) -> Paddle {
        let mut labels = vec![String::new()];
        labels.extend(LATIN_DICT.lines().map(str::to_owned));
        labels.push(" ".to_owned());
        let at = |c: &str| labels.iter().position(|l| l == c);
        let turkish_pairs = [("I", "İ"), ("i", "ı")]
            .iter()
            .filter_map(|(a, b)| Some((at(a)?, at(b)?)))
            .collect();
        Paddle {
            model,
            labels,
            turkish: false,
            turkish_pairs,
        }
    }

    /// One line's crop at the model's height, and its box. The crop is the box around the line's words,
    /// with no margin and not turned to the line's slant: on the measured scan,
    /// a margin of a quarter line took neighbouring lines in and cost 56 points
    /// of words found, and following the slant cost 25.
    fn crop(page: &image::RgbImage, words: &[RotatedRect]) -> Option<(image::RgbImage, Rect)> {
        use image::imageops::{self, FilterType};
        let r = words
            .iter()
            .map(|w| w.bounding_rect())
            .reduce(|a, b| a.union(b))?;
        let (pw, ph) = page.dimensions();
        let x0 = r.left().max(0.0) as u32;
        let y0 = r.top().max(0.0) as u32;
        let x1 = (r.right().max(0.0) as u32).min(pw);
        let y1 = (r.bottom().max(0.0) as u32).min(ph);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let crop = imageops::crop_imm(page, x0, y0, w, h).to_image();
        let width = ((HEIGHT as f32 * w as f32 / h as f32).ceil() as u32).clamp(16, 3200);
        let line = imageops::resize(&crop, width, HEIGHT, FilterType::Triangle);
        let rect = Rect {
            x: x0 as f32,
            y: y0 as f32,
            width: w as f32,
            height: h as f32,
        };
        Some((line, rect))
    }

    /// One line's text.
    fn read_line(&self, line: &image::RgbImage) -> Result<String, Error> {
        let (width, height) = (line.width() as usize, HEIGHT as usize);
        // PP-OCR takes BGR in [-1, 1]; the page is gray, so the order does not matter.
        let mut input = rten_tensor::NdTensor::<f32, 4>::zeros([1, 3, height, width]);
        for (x, y, p) in line.enumerate_pixels() {
            for c in 0..3 {
                input[[0, c, y as usize, x as usize]] = (f32::from(p[2 - c]) / 255.0 - 0.5) / 0.5;
            }
        }
        let output = self
            .model
            .run_one(input.view().into(), None)
            .map_err(|e| Error::Recognise(e.to_string()))?;
        let output: rten_tensor::NdTensor<f32, 3> = output
            .try_into()
            .map_err(|_| Error::Recognise("unexpected output".to_owned()))?;
        let [_, steps, classes] = output.shape();
        let mut text = String::new();
        let mut last = 0;
        for t in 0..steps {
            let mut best = (0..classes)
                .max_by(|&a, &b| output[[0, t, a]].total_cmp(&output[[0, t, b]]))
                .unwrap_or(0);
            if self.turkish {
                for &(plain, turkish) in &self.turkish_pairs {
                    if best == plain
                        && output[[0, t, turkish]] * TURKISH_FAVOUR > output[[0, t, plain]]
                    {
                        best = turkish;
                    }
                }
            }
            // CTC: a repeat is one character, the blank separates real repeats.
            if best != 0
                && best != last
                && let Some(c) = self.labels.get(best)
            {
                text.push_str(c);
            }
            last = best;
        }
        Ok(text)
    }

    /// Every line of a page, in ocrs's order, one run of the model each. Lines
    /// padded into batches gave no speed and lost text: on the accuracy set CER
    /// rose to 14–38% on some pages, the padding read as letters.
    fn read(&self, page: &image::RgbImage, lines: &[Vec<RotatedRect>]) -> Result<Vec<Line>, Error> {
        let crops: Vec<_> = lines.iter().filter_map(|l| Self::crop(page, l)).collect();
        let texts = crops
            .iter()
            .map(|(line, _)| self.read_line(line))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(crops
            .into_iter()
            .zip(texts)
            .filter(|(_, text)| !text.trim().is_empty())
            .map(|((_, rect), text)| Line {
                text,
                bbox: Some(rect),
            })
            .collect())
    }
}

/// In Turkish text, İ wins over I, and ı over i, unless the model thinks the
/// plain letter at least this many times likelier. The model reads İ as I and
/// ı as i too often: on one Turkish scan, 712 İ of the reference's 1,115 with
/// 561 I against 117. Measured on that scan, against its publisher's EPUB:
/// words found 94.18% with no favour, 94.91% at 3, 95.40% at 10, 95.41% at 30,
/// where i began to fall below the reference's count (34,524 against 34,881).
const TURKISH_FAVOUR: f32 = 10.0;

/// The height PP-OCR's recognition model takes its lines at.
const HEIGHT: u32 = 48;

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Engine")
    }
}

impl Engine {
    /// Load the detection and recognition models from their bytes, ONNX or
    /// RTen (the format is detected from the bytes).
    pub fn new(detection: Vec<u8>, recognition: Vec<u8>) -> Result<Engine, Error> {
        let detection =
            Model::load(detection).map_err(|e| Error::Models(format!("detection: {e}")))?;
        let recognition =
            Model::load(recognition).map_err(|e| Error::Models(format!("recognition: {e}")))?;
        let (recognition, paddle) = if Paddle::accepts(&recognition) {
            (None, Some(Paddle::new(recognition)))
        } else {
            (Some(recognition), None)
        };
        let detector = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection),
            recognition_model: recognition,
            ..Default::default()
        })
        .map_err(|e| Error::Models(format!("ocrs engine init failed: {e}")))?;
        Ok(Engine { detector, paddle })
    }

    /// Tell the engine the content's language, a BCP-47 tag. With the Latin
    /// model it changes one thing: in Turkish and Azerbaijani (`tr`, `az`), the
    /// dotted capital İ and the dotless ı are favoured where the model hesitates
    /// between them and I or i. ocrs's own model has neither letter, so it
    /// ignores this.
    pub fn set_language(&mut self, language: &str) {
        let primary = language
            .split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if let Some(paddle) = &mut self.paddle {
            paddle.turkish = matches!(primary.as_str(), "tr" | "az");
        }
    }

    /// Recognise one page image (any format the `image` crate decodes here:
    /// JPEG or PNG). Lines come top to bottom, as ocrs orders them.
    pub fn recognise(&self, image_bytes: &[u8]) -> Result<Vec<Line>, Error> {
        let img = image::load_from_memory(image_bytes)
            .map_err(|e| Error::Image(e.to_string()))?
            .into_luma8();
        self.recognise_image(&img)
    }

    /// Recognise one page given as 8-bit gray pixels, row by row.
    pub fn recognise_gray(
        &self,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<Vec<Line>, Error> {
        let needed = u64::from(width) * u64::from(height);
        if (pixels.len() as u64) < needed {
            return Err(Error::Image("fewer pixels than its size".to_owned()));
        }
        let img = image::GrayImage::from_raw(width, height, pixels[..needed as usize].to_vec())
            .ok_or_else(|| Error::Image("no pixels".to_owned()))?;
        self.recognise_image(&img)
    }

    fn recognise_image(&self, gray: &image::GrayImage) -> Result<Vec<Line>, Error> {
        let img = flatten_illumination(gray);
        let (w, h) = img.dimensions();
        let source = ImageSource::from_bytes(img.as_raw(), (w, h))
            .map_err(|e| Error::Image(e.to_string()))?;
        let input = self
            .detector
            .prepare_input(source)
            .map_err(|e| Error::Recognise(format!("prepare input: {e}")))?;
        let words = self
            .detector
            .detect_words(&input)
            .map_err(|e| Error::Recognise(e.to_string()))?;
        let lines = self.detector.find_text_lines(&input, &words);
        if let Some(paddle) = &self.paddle {
            return paddle.read(&img, &lines);
        }
        let lines = self
            .detector
            .recognize_text(&input, &lines)
            .map_err(|e| Error::Recognise(e.to_string()))?;
        Ok(lines
            .into_iter()
            .flatten()
            .map(|l| Line {
                text: l.to_string(),
                bbox: (!l.chars().is_empty()).then(|| {
                    let r = l.bounding_rect();
                    Rect {
                        x: r.left() as f32,
                        y: r.top() as f32,
                        width: r.width() as f32,
                        height: r.height() as f32,
                    }
                }),
            })
            .collect())
    }
}

/// Flat-field illumination correction, for phone photos with shadow
/// gradients: estimate the lighting by downscaling 24× and back (a blurred
/// background), then divide it out, so paper turns uniformly bright and ink
/// stays dark. The result is gray, in RGB.
fn flatten_illumination(gray: &image::GrayImage) -> image::RgbImage {
    use image::imageops::{self, FilterType};
    let (w, h) = gray.dimensions();
    let small = imageops::resize(gray, (w / 24).max(1), (h / 24).max(1), FilterType::Gaussian);
    let background = imageops::resize(&small, w, h, FilterType::Triangle);

    let mut out = image::RgbImage::new(w, h);
    for (x, y, px) in gray.enumerate_pixels() {
        let p = f32::from(px[0]);
        let b = f32::from(background.get_pixel(x, y)[0]).max(1.0);
        let v = (p / b * 245.0).min(255.0) as u8;
        out.put_pixel(x, y, image::Rgb([v, v, v]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattening_whitens_an_even_page() {
        let img = image::GrayImage::from_pixel(96, 96, image::Luma([180]));
        let out = flatten_illumination(&img);
        assert!(
            out.pixels()
                .all(|p| p[0] == 245 && p[1] == 245 && p[2] == 245)
        );
    }

    #[test]
    fn engine_is_send() {
        fn send<T: Send>() {}
        send::<Engine>();
    }

    #[test]
    fn model_hashes_are_sha256_hex() {
        for m in MODELS
            .iter()
            .chain(&ONNX_MODELS)
            .chain([&LATIN_RECOGNITION])
        {
            assert_eq!(m.sha256.len(), 64, "{}", m.name);
            assert!(
                m.sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
        }
    }

    #[test]
    fn bad_model_bytes_are_an_error() {
        assert!(matches!(
            Engine::new(vec![1, 2, 3], vec![4, 5, 6]),
            Err(Error::Models(_))
        ));
    }
}
