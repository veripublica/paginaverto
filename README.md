# paginaverto

Reads PDFs and page images (scans and photos) and turns them into a
structured document, from headings to footnotes. It is a Rust library, and
will be a command-line tool for people and scripts who want the text and
structure of a document without writing code. Part of the
[veripublica](https://github.com/veripublica) tools. The name is Latin:
*pagina*, page, and *verto*, to turn.

**Status:** 0.1.0 is in preparation, as a library only. It is a parity
release: it does exactly what
[epublift](https://github.com/ePubLift/epublift)'s PDF import did in
cli-v4.5.1, so that epublift can move to it. The structured document
described under [Scope](#scope) comes from 0.2.0 on.

## What 0.1.0 does

It reads a PDF into a book: chapters, each with an optional title and its
paragraphs and figures in reading order. It handles three kinds of PDF:

1. **Born-digital**, with a text layer. The text is extracted directly. Both
   simple fonts and composite (Type0/CID) fonts with a `/ToUnicode` map are
   read, and word spaces come from the fonts' real glyph widths.
2. **A scan with an embedded OCR text layer** (most "searchable" PDFs). That
   layer is reused as it is.
3. **A pure scan.** The page images go through character recognition with
   [ocrs](https://github.com/robertknight/ocrs), behind the `ocr` feature.

Then it removes running heads and feet and page numbers, joins words split
by a line-break hyphen, and finds the headings that start chapters (by
capitals, or by font size on born-digital pages).

It refuses, with an error value, a PDF it cannot read: a scan when OCR is
off, a text layer whose fonts do not decode, a result that is mostly
non-letters. It never panics at the caller: a panic inside a dependency
comes back as `Error::Internal`. It does no network access.

## Use

0.1.0 is distributed as a git tag, not on crates.io:

```toml
[dependencies]
paginaverto = { git = "https://github.com/veripublica/paginaverto", tag = "v0.1.0" }
# With OCR for scans:
# paginaverto = { git = "…", tag = "v0.1.0", features = ["ocr"] }
```

```rust
use paginaverto::{Block, Input, Options, read};

let book = read(Input::Path("book.pdf".as_ref()), &Options::default())?;
for chapter in &book.chapters {
    println!("{:?}", chapter.title);
    for block in &chapter.blocks {
        if let Block::Paragraph(text) = block {
            println!("{text}");
        }
    }
}
```

### OCR

Build with the `ocr` feature, then pass the two ocrs models and the
content language:

```rust
let mut opts = paginaverto::Options::default();
opts.language = Some("en".into());
opts.ocr = paginaverto::Ocr::Models {
    detection: std::fs::read("text-detection.rten")?,
    recognition: std::fs::read("text-recognition.rten")?,
};
```

paginaverto never downloads the models: the caller provides their bytes.
`paginaverto::MODELS` names the two files and gives their SHA-256; check
it before loading. **As of 2026-10-31 the ocrs bucket no longer serves the
`.rten` files** (it answers 403; it still serves `.onnx`), so a copy you
already have is the only source for now. Moving to the current ocrs models
is planned for 0.2.0.

The default build pulls in neither ocrs nor rten.

## Accuracy

- **No language counts as supported yet.** A language will be supported
  once it has a test set and a measured character error rate, stated here
  separately for the text-layer path and the OCR path. English and Turkish
  come first.
- **Parity with epublift,** measured 2026-10-31 on 18 local PDFs (books, two
  of them scans, one refused): the chapters, paragraphs and figures of the
  17 that convert are byte-identical to epublift cli-v4.5.1's, and the
  refused one gets the same error. That measures faithfulness to epublift,
  not accuracy.
- epublift's earlier measurements, on one book each: about 99.8% word
  overlap for a born-digital book, about 92% word accuracy for OCR of a
  clean prose scan.

## Known limits of 0.1.0

They are kept on purpose: changing them would change the output, and then
parity with epublift would prove nothing. Each one is fixed from 0.2.0 on,
and measured. In the code they are marked `PARITY:`.

- **OCR reads ASCII only.** The ocrs 0.13.1 model is trained on ASCII text,
  so it cannot produce ç ğ ı İ ö ş ü, other accented letters or non-Latin
  scripts. The text-layer path has no such limit.
- Capitals outside ASCII (İ, Ç, Ş, …) are not lowercased when running heads
  and headings are matched, so those can be missed in Turkish books.
- Paragraphs can come out coarse on some born-digital PDFs.
- Figures are placed after their page's text, not at their exact position.
  JPEG 2000, CCITT, JBIG2 and CMYK images are skipped.
- Tables and equations come out as plain text.
- A PDF made only of vector drawings cannot be read: there is no text layer
  and no image to recognise.
- Pages are read one after another; there is no progress report, no cancel
  and no limit on pages, streams or pixels yet.
- The language is required for OCR but not used yet.

The PDF and OCR engines are pinned to the exact versions epublift used
(lopdf 0.45.0, ocrs 0.13.1, rten 0.26.0), because their behaviour is part
of the output. They move from 0.2.0 on.

## Scope

What paginaverto is for, beyond 0.1.0:

- **Input:** PDF files and page images. Text comes from a PDF's text layer
  where one exists, and from character recognition (OCR) where none does.
- **Output:** a structured document. It has headings, paragraphs, lists,
  figures and captions, and footnotes linked to the place they are referenced
  from. Each part carries its language, its reading direction, the page it
  came from and its position there, and how confident the recognition was.
- **Offline only.** It calls no cloud service.
- **Languages:** a language counts as supported once it has a test set and a
  measured accuracy, which this README will state. The first two are English
  and Turkish.
- **Not here:** building an EPUB (that is
  [epublift](https://github.com/ePubLift/epublift)'s job), editing or writing
  PDFs, and AI-based conversion.

## Layout

- `crates/paginaverto`: the library.
- `crates/paginaverto-ocr`: character recognition, used through the `ocr`
  feature. It hands the rest of paginaverto lines of text, so the PDF side
  and the structure step never depend on the OCR engine.

Minimum Rust version: 1.97.

## Licence

AGPL-3.0-only OR a commercial license ([LICENSE](LICENSE),
[LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md)). Extracting a document's text
with `paginaverto` puts no license on the result. Recognition models keep
their own licenses: the ocrs models are CC BY-SA 4.0, and paginaverto does
not distribute them. See [LICENSING.md](LICENSING.md).
