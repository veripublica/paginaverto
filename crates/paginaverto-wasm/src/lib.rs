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

/// The shapes `read` returns and throws, for the generated `.d.ts`. The
/// objects are built by hand in [`to_js`] and [`error`]; a native test keeps
/// every error kind listed here.
// Read by the macro below at build time and by a native test; rustc's
// dead-code pass sees neither use on wasm32.
#[allow(dead_code)]
const TS_TYPES: &str = r#"
/** One piece of chapter content, in reading order. */
export type Block =
  | {
      kind: "paragraph";
      /** The paragraph's text, whitespace collapsed to single spaces. */
      text: string;
    }
  | {
      kind: "figure";
      /** A JPEG verbatim, or raw pixels re-encoded as PNG. */
      mediaType: "image/jpeg" | "image/png";
      ext: "jpg" | "png";
      /** The image file's bytes. */
      data: Uint8Array;
    };

/** An optional heading and its content. */
export interface Chapter {
  /** The heading that starts the chapter; `null` before the first heading. */
  title: string | null;
  blocks: Block[];
}

/** What a PDF turned into. `chapters` is never empty. */
export interface Book {
  chapters: Chapter[];
}

/** Why a PDF could not be read; the `kind` of a thrown `PaginavertoError`. */
export type ErrorKind =
  | "Open"
  | "ScanNeedsOcr"
  | "LanguageRequired"
  | "Models"
  | "UndecodableImages"
  | "OcrFoundNothing"
  | "UndecodableFonts"
  | "NoText"
  | "QualityGate"
  | "Internal";

/** What `read` throws: an `Error` whose `kind` names the reason. */
export interface PaginavertoError extends Error {
  kind: ErrorKind;
}
"#;

#[wasm_bindgen(typescript_custom_section)]
const TS_SECTION: &str = TS_TYPES;

/// The crate version.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Read a PDF into a `Book` (chapters of paragraphs and figures). Throws a
/// `PaginavertoError`: an `Error` whose `kind` names the reason (`Open`,
/// `ScanNeedsOcr`, `UndecodableFonts`, `NoText`, `QualityGate`, `Internal`)
/// and whose `message` says it in English. Text layers only: a scan throws
/// `ScanNeedsOcr`.
#[wasm_bindgen(unchecked_return_type = "Book")]
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
        Error::UndecodableImages(_) => "UndecodableImages",
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

    #[test]
    fn every_kind_is_in_the_typescript_types() {
        let all = [
            Error::Open(String::new()),
            Error::ScanNeedsOcr,
            Error::LanguageRequired,
            Error::Models(String::new()),
            Error::UndecodableImages(Vec::new()),
            Error::OcrFoundNothing,
            Error::UndecodableFonts,
            Error::NoText,
            Error::QualityGate,
            Error::Internal(String::new()),
        ];
        for e in &all {
            let k = format!("\"{}\"", kind(e));
            assert!(TS_TYPES.contains(&k), "{k} missing from ErrorKind");
        }
    }
}
