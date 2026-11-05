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
//! for (const p of book.pages) {
//!   p.textSource; p.problems;      // where its text came from, what went wrong
//! }
//! ```
//!
//! Text layers only: a scan with no text layer throws, with
//! `error.kind === "ScanNeedsOcr"`.

#![warn(missing_docs)]

use js_sys::{Array, Object, Reflect, Uint8Array};
use paginaverto::{Block, BlockKind, Document, Error, Input, Options};
use wasm_bindgen::prelude::*;

/// The shapes `read` returns and throws, for the generated `.d.ts`. The
/// objects are built by hand in [`to_js`] and [`error`]; a native test keeps
/// every error kind listed here.
// Read by the macro below at build time and by a native test; rustc's
// dead-code pass sees neither use on wasm32.
#[allow(dead_code)]
const TS_TYPES: &str = r#"
/** A stretch of text with one style. */
export interface Run {
  text: string;
  italic: boolean;
}

/** One piece of chapter content, in reading order. */
export type Block =
  | {
      kind: "paragraph";
      /** The paragraph's text, whitespace collapsed to single spaces. */
      text: string;
      /** The same text piece by piece, with what is shown in italics. */
      runs: Run[];
      /** A heading inside the chapter: how deep it sits, from 2. */
      level?: number;
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

/** One page of the PDF, in points. */
export interface Page {
  /** From 1. */
  number: number;
  width: number;
  height: number;
  textSource: "text_layer" | "embedded_ocr" | "ocr" | "none";
  /** What went wrong on the page, such as `undecodable_images`. */
  problems: string[];
}

/** An entry of the PDF's own outline, whatever it is worth. */
export interface OutlineEntry {
  level: number;
  title: string;
  /** From 1; `null` when it points nowhere this file can say. */
  page: number | null;
}

/** What a PDF turned into. `chapters` is never empty. */
export interface Book {
  /** The content's language, if the file says. */
  language: string | null;
  outline: OutlineEntry[];
  pages: Page[];
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
  | "LimitExceeded"
  | "Cancelled"
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
        Error::LimitExceeded { .. } => "LimitExceeded",
        Error::Cancelled => "Cancelled",
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

fn runs_js(block: &Block) -> Array {
    let runs = Array::new();
    for r in block.runs() {
        let o = Object::new();
        set(&o, "text", &r.text.as_str().into());
        set(&o, "italic", &r.italic.into());
        runs.push(&o);
    }
    runs
}

fn to_js(doc: &Document) -> JsValue {
    let chapters = Array::new();
    for c in doc.chapters() {
        let blocks = Array::new();
        for b in c.blocks {
            let o = Object::new();
            match &b.kind {
                BlockKind::Figure(f) => {
                    set(&o, "kind", &"figure".into());
                    set(&o, "mediaType", &f.media_type.into());
                    set(&o, "ext", &f.ext.into());
                    set(&o, "data", &Uint8Array::from(f.data.as_slice()).into());
                }
                // Anything else with text reads as a paragraph.
                _ => {
                    set(&o, "kind", &"paragraph".into());
                    set(&o, "text", &b.text().into());
                    set(&o, "runs", &runs_js(b));
                    if let Some(level) = b.level() {
                        set(&o, "level", &level.into());
                    }
                }
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
    let pages = Array::new();
    for p in &doc.pages {
        let o = Object::new();
        set(&o, "number", &p.number.into());
        set(&o, "width", &p.width.into());
        set(&o, "height", &p.height.into());
        set(&o, "textSource", &p.text_source.name().into());
        let problems = Array::new();
        for problem in &p.problems {
            problems.push(&problem.name().into());
        }
        set(&o, "problems", &problems);
        pages.push(&o);
    }
    let outline = Array::new();
    for e in &doc.metadata.outline {
        let o = Object::new();
        set(&o, "level", &e.level.into());
        set(&o, "title", &e.title.as_str().into());
        set(&o, "page", &e.page.map_or(JsValue::NULL, Into::into));
        outline.push(&o);
    }
    let out = Object::new();
    set(
        &out,
        "language",
        &doc.language.as_deref().map_or(JsValue::NULL, Into::into),
    );
    set(&out, "outline", &outline);
    set(&out, "pages", &pages);
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
            Error::LimitExceeded {
                what: paginaverto::LimitKind::Pages,
                limit: 1,
            },
            Error::Cancelled,
            Error::Internal(String::new()),
        ];
        for e in &all {
            let k = format!("\"{}\"", kind(e));
            assert!(TS_TYPES.contains(&k), "{k} missing from ErrorKind");
        }
    }
}
