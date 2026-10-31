//! WebAssembly bindings for [`paginaverto`]: read a PDF into chapters in the
//! browser. The bytes never leave the page.
//!
//! ```js
//! import init, { read, version } from "paginaverto-wasm";
//! await init();
//! const book = read(new Uint8Array(await file.arrayBuffer()));
//! for (const c of book.chapters) {
//!   c.title;                       // string or null
//!   for (const b of c.blocks) {
//!     b.kind === "paragraph" ? b.text : b.data; // figure: Uint8Array, b.mediaType
//!   }
//! }
//! ```
//!
//! Text layers only: a scan with no text layer throws, with
//! `error.kind === "ScanNeedsOcr"`.

#![warn(missing_docs)]

use js_sys::{Array, Object, Reflect, Uint8Array};
use paginaverto::{Block, Book, Error, Input, Options};
use wasm_bindgen::prelude::*;

/// The crate version.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Read a PDF. Returns `{ chapters: [{ title, blocks }] }`, where each block
/// is `{ kind: "paragraph", text }` or `{ kind: "figure", mediaType, ext,
/// data }`. Throws an `Error` whose `kind` names the reason (`Open`,
/// `ScanNeedsOcr`, `UndecodableFonts`, `NoText`, `QualityGate`, `Internal`)
/// and whose `message` says it in English.
#[wasm_bindgen]
pub fn read(pdf: &[u8]) -> Result<JsValue, JsValue> {
    match paginaverto::read(Input::Bytes(pdf), &Options::default()) {
        Ok(book) => Ok(to_js(&book)),
        Err(e) => Err(error(&e)),
    }
}

/// The name of an error's kind, as the `kind` property carries it.
pub fn kind(e: &Error) -> &'static str {
    match e {
        Error::Open(_) => "Open",
        Error::ScanNeedsOcr => "ScanNeedsOcr",
        Error::LanguageRequired => "LanguageRequired",
        Error::Models(_) => "Models",
        Error::OcrFoundNothing => "OcrFoundNothing",
        Error::UndecodableFonts => "UndecodableFonts",
        Error::NoText => "NoText",
        Error::QualityGate => "QualityGate",
        _ => "Internal",
    }
}

fn error(e: &Error) -> JsValue {
    let err = js_sys::Error::new(&e.to_string());
    set(&err, "kind", &kind(e).into());
    err.into()
}

fn set(target: &JsValue, key: &str, value: &JsValue) {
    // Cannot fail on a plain object or Error.
    let _ = Reflect::set(target, &key.into(), value);
}

fn to_js(book: &Book) -> JsValue {
    let chapters = Array::new();
    for c in &book.chapters {
        let blocks = Array::new();
        for b in &c.blocks {
            let o = Object::new();
            match b {
                Block::Paragraph(text) => {
                    set(&o, "kind", &"paragraph".into());
                    set(&o, "text", &text.into());
                }
                Block::Figure(f) => {
                    set(&o, "kind", &"figure".into());
                    set(&o, "mediaType", &f.media_type.into());
                    set(&o, "ext", &f.ext.into());
                    set(&o, "data", &Uint8Array::from(f.data.as_slice()).into());
                }
                _ => continue,
            }
            blocks.push(&o);
        }
        let ch = Object::new();
        set(
            &ch,
            "title",
            &c.title.as_deref().map_or(JsValue::NULL, Into::into),
        );
        set(&ch, "blocks", &blocks);
        chapters.push(&ch);
    }
    let out = Object::new();
    set(&out, "chapters", &chapters);
    out.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_named() {
        assert_eq!(kind(&Error::ScanNeedsOcr), "ScanNeedsOcr");
        assert_eq!(kind(&Error::Internal("x".into())), "Internal");
    }
}
