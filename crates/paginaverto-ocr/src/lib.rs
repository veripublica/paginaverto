//! Character recognition for paginaverto: a page image in, lines of text out.
//!
//! This crate is the OCR side of paginaverto's seam. The PDF side and the
//! structure step only ever see [`Line`]s, whichever produced them.
//!
//! 0.1.0 wraps ocrs 0.13.1 on rten 0.26, unchanged from epublift cli-v4.5.1.
//! The models are not part of this crate: the caller supplies their bytes.

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
