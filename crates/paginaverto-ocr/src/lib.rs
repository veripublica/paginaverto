//! Character recognition for paginaverto: a page image in, lines of text out.
//!
//! This crate is the OCR side of paginaverto's seam. The PDF side and the
//! structure step only ever see [`Line`]s, whichever produced them.
//!
//! 0.1.0 wraps ocrs 0.13.1 on rten 0.26, unchanged from epublift cli-v4.5.1:
//! the same illumination correction, the same models, the same text. The
//! models are not part of this crate; the caller supplies their bytes (see
//! [`MODELS`]).

use std::fmt;

use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;

/// Where the ocrs project published its models.
///
/// As of 2026-10-31 this bucket answers 403 for the `.rten` files (it still
/// serves `.onnx`), so a fresh download fails; the hashes in [`MODELS`] are
/// of the copies epublift downloaded on 2026-10-26.
pub const MODEL_BASE_URL: &str = "https://ocrs-models.s3-accelerate.amazonaws.com";

/// One model file the caller fetches from [`MODEL_BASE_URL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
    /// The file name, also its path under [`MODEL_BASE_URL`].
    pub name: &'static str,
    /// SHA-256 of the file, lowercase hex. Check it before loading.
    pub sha256: &'static str,
}

/// The detection model, then the recognition model. CC BY-SA 4.0 (trained on
/// HierText); whoever redistributes them gives the attribution their license
/// asks for.
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
    pub text: String,
    /// Where the line sits on the page image, in pixels. Always `None` in
    /// 0.1.0; filled from 0.2.0 on.
    pub bbox: Option<Rect>,
}

/// An axis-aligned box in image pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
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
pub struct Engine(OcrEngine);

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Engine")
    }
}

impl Engine {
    /// Load the detection and recognition models from their bytes.
    pub fn new(detection: Vec<u8>, recognition: Vec<u8>) -> Result<Engine, Error> {
        let detection =
            Model::load(detection).map_err(|e| Error::Models(format!("detection: {e}")))?;
        let recognition =
            Model::load(recognition).map_err(|e| Error::Models(format!("recognition: {e}")))?;
        OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection),
            recognition_model: Some(recognition),
            ..Default::default()
        })
        .map(Engine)
        .map_err(|e| Error::Models(format!("ocrs engine init failed: {e}")))
    }

    /// Recognise one page image (any format the `image` crate decodes here:
    /// JPEG or PNG). Lines come top to bottom, as ocrs orders them.
    pub fn recognise(&self, image_bytes: &[u8]) -> Result<Vec<Line>, Error> {
        let img = image::load_from_memory(image_bytes)
            .map_err(|e| Error::Image(e.to_string()))?
            .into_rgb8();
        let img = flatten_illumination(&img);
        let (w, h) = img.dimensions();
        let source = ImageSource::from_bytes(img.as_raw(), (w, h))
            .map_err(|e| Error::Image(e.to_string()))?;
        let input = self
            .0
            .prepare_input(source)
            .map_err(|e| Error::Recognise(format!("prepare input: {e}")))?;
        let text = self
            .0
            .get_text(&input)
            .map_err(|e| Error::Recognise(e.to_string()))?;
        Ok(text
            .lines()
            .map(|l| Line {
                text: l.to_string(),
                bbox: None,
            })
            .collect())
    }
}

/// Flat-field illumination correction, for phone photos with shadow
/// gradients: estimate the lighting by downscaling 24× and back (a blurred
/// background), then divide it out, so paper turns uniformly bright and ink
/// stays dark. The result is gray, in RGB.
fn flatten_illumination(img: &image::RgbImage) -> image::RgbImage {
    use image::imageops::{self, FilterType};
    let gray = image::DynamicImage::ImageRgb8(img.clone()).into_luma8();
    let (w, h) = gray.dimensions();
    let small = imageops::resize(
        &gray,
        (w / 24).max(1),
        (h / 24).max(1),
        FilterType::Gaussian,
    );
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
        let img = image::RgbImage::from_pixel(96, 96, image::Rgb([180, 180, 180]));
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
    fn bad_model_bytes_are_an_error() {
        assert!(matches!(
            Engine::new(vec![1, 2, 3], vec![4, 5, 6]),
            Err(Error::Models(_))
        ));
    }
}
