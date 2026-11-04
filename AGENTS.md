# paginaverto: detail for code agents and maintainers

`README.md` is the short introduction. This file holds the detail and is kept
true at every release. Past changes: `CHANGELOG.md`. API docs:
<https://docs.rs/paginaverto>.

Figures below name the versions they were measured on. Current version: 0.1.4.

## Scope

paginaverto reads PDFs and turns them into a structured document. It ships as a
Rust library (crates `paginaverto` and `paginaverto-ocr`) and a WebAssembly
package (`@veripublica/paginaverto-wasm`), released together under one version.
There is no CLI yet: it comes in 0.2 with the document format, because dumping
today's thin `Book` as JSON would freeze that format early. The web page at
<https://veripublica.github.io/paginaverto/> runs the WebAssembly build; the
file never leaves the browser.

0.1 is a parity release. It does what
[epublift](https://github.com/ePubLift/epublift)'s PDF import did in cli-v4.5.1
(the same chapters, paragraphs, figures and refusals), and epublift builds on
it. The one deliberate difference is in [Limits](#limits-of-01).

What it reads into a `Book`, chapters with an optional title and their
paragraphs and figures in reading order, from three kinds of PDF:

1. **Born-digital,** with a text layer. Simple fonts and composite (Type0/CID)
   fonts with a `/ToUnicode` map are read. Word spaces come from the fonts' real
   glyph widths.
2. **A scan with an embedded OCR text layer.** That layer is reused as it is.
3. **A pure scan.** The page images go through
   [ocrs](https://github.com/robertknight/ocrs), behind the `ocr` feature.

It then removes running heads and feet and page numbers, joins words split by a
line-break hyphen, and finds the headings that start chapters (by capitals, or
by font size on born-digital pages).

Where it is going, from 0.2: a structured document with headings, paragraphs,
lists, figures and captions, and footnotes linked to where they are referenced.
Each part carries its language, reading direction, source page and position,
and recognition confidence. Page images as input too.

Rules the code follows:

- **Errors are values.** `read` catches panics in dependencies and returns
  `Error::Internal`.
- **No network and no global state; the API is `Send`.** OCR models come from
  the caller as bytes.
- **The default build never pulls ocrs or rten.** CI checks it.
- **The seam between the two crates is positioned text:** OCR yields `Line`s, so
  the PDF side and the structure step never depend on the OCR engine. The boxes
  on `Line` stay empty until 0.2.

## What it declines to do

- Building an EPUB: that is epublift's job.
- Editing or writing PDFs.
- AI-based conversion and cloud services. It works offline.
- Downloading models.
- Claiming a language it has not measured. A language counts as supported once
  it has a test set and a measured character error rate, stated separately for
  the text-layer path and the OCR path. None does yet. English and Turkish come
  first.

## Install

```toml
[dependencies]
paginaverto = "0.1"
# With OCR for scans:
# paginaverto = { version = "0.1", features = ["ocr"] }
```

On crates.io from 0.1.2; 0.1.0 and 0.1.1 are git tags only. The JavaScript
package is `@veripublica/paginaverto-wasm` on npm. It reads text layers only.

## API

```rust
pub fn read(input: Input<'_>, opts: &Options) -> Result<Book, Error>;

enum Input<'a> { Path(&'a Path), Bytes(&'a [u8]) }
#[non_exhaustive] struct Options { language: Option<String>, ocr: Ocr }
enum Ocr { Off, Models { detection: Vec<u8>, recognition: Vec<u8> } }
#[non_exhaustive] struct Book { chapters: Vec<Chapter> }
struct Chapter { title: Option<String>, blocks: Vec<Block> }
#[non_exhaustive] enum Block { Paragraph(String), Figure(Figure) }
struct Figure { data: Vec<u8>, media_type: &'static str, ext: &'static str }
#[non_exhaustive] enum Error {
    Open(String), ScanNeedsOcr, LanguageRequired, Models(String),
    UndecodableImages(Vec<String>), OcrFoundNothing, UndecodableFonts, NoText,
    QualityGate, Internal(String),
}
```

A scan's checks come in this order: images no build can decode yet
(`UndecodableImages`: JBIG2, JPEG 2000, CCITT fax, when every page's image is one
of them), then OCR off, then language missing, then models.

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

Build with the `ocr` feature, then pass the two ocrs models and the content
language:

```rust
let mut opts = paginaverto::Options::default();
opts.language = Some("en".into());
opts.ocr = paginaverto::Ocr::Models {
    detection: std::fs::read("text-detection.onnx")?,
    recognition: std::fs::read("text-recognition.onnx")?,
};
```

`ONNX_MODELS` names the two files to fetch from `MODEL_BASE_URL` and gives their
SHA-256; check it before loading. The older `.rten` files (`MODELS`) still load,
but the ocrs bucket no longer serves them (403 since at least 2026-10-31). Both
forms carry the same models and gave byte-identical text on every scan measured.

### WebAssembly

`read(pdf: Uint8Array)` returns a `Book`; `version()` returns the version. The
`.d.ts` declares `Book`, `Chapter`, `Block` (a union you narrow on `kind`:
`"paragraph"` with `text`, `"figure"` with `mediaType`, `ext` and `data`),
`ErrorKind` and `PaginavertoError`, which is what `read` throws.

## Limits of 0.1

Being fixed on `main` for 0.2.0, each measured; what is still true of `main`
stays here.

- OCR reads ASCII only. The ocrs 0.13.1 model is trained on ASCII text, so it
  cannot produce ç ğ ı İ ö ş ü, other accented letters or non-Latin scripts. The
  text-layer path has no such limit.
- Figures are placed after their page's text, not at their exact position.
  JPEG 2000, CCITT, JBIG2 and CMYK images are skipped.
- A document classified as a scan goes through OCR on every page, and scans yield
  no figures.
- An OCR failure on one page gives an empty page, silently.
- Tables and equations come out as plain text.
- A PDF made only of vector drawings cannot be read: it has no text layer and no
  image to recognise.
- Pages are read one after another. There is no progress report, no cancel and no
  limit on pages, streams or pixels.
- The language is required for OCR but not used yet.

The one deliberate difference from epublift cli-v4.5.1, since 0.1.4: figures with
filters in front of `DCTDecode` (`/Filter [/FlateDecode /DCTDecode]`) have those
filters undone. epublift's import emitted the still-deflated bytes under an
`image/jpeg` label, which made an invalid EPUB. If the filters cannot be undone,
or the result is not a JPEG, there is no figure.

The engines are pinned exactly (lopdf 0.45.0, ocrs 0.13.1, rten 0.26.0, image
0.25.10), because their behaviour is part of the output. They move from 0.2.0 on.
`Cargo.lock` matches epublift's versions package for package, so do not run a
plain `cargo update`.

## Measurements

- **Parity, 0.1.0 against epublift cli-v4.5.1,** 2026-10-31, on 18 local PDFs
  (books, two of them scans, one refused): the chapters, paragraphs and figures
  of the 17 that convert are byte-identical, and the refused one gets the same
  error. That measures faithfulness to epublift, not accuracy.
- **0.1.4 against 0.1.3,** 2026-11-04, same 18 PDFs: 11 figures changed, in two
  books (10 of 23, 1 of 9), and all 11 are now JPEGs. Chapters, paragraphs, the
  other figures and the refusal are the same.
- **Public accuracy set, 2026-11-05, unreleased main.** Known text, about 1,100
  characters each of English and Turkish written for this set
  (`crates/paginaverto/tests/corpus/`). It is small and synthetic: it catches
  regressions and shows the size of a problem, and it is no estimate for real
  books. No language counts as supported on it yet.
  - **Text layer** (`tests/accuracy.rs`, nine ways of laying out a line, run in
    CI): CER 0.00% and every paragraph found, in both languages. 0.1.4 had up to
    17.96% (English) and 12.37% (Turkish) on the layouts where it lost spaces.
  - **OCR** (`tools/ocr-accuracy`, by hand, ocrs 0.13.1, 200 dpi), CER:

    | | Arial clean | Arial noisy | Times clean | Times noisy |
    | --- | ---: | ---: | ---: | ---: |
    | English | 1.20% | 3.70% | 1.85% | 5.46% |
    | Turkish | 13.04% | 16.01% | 14.09% | 17.07% |
    | Turkish, letters folded to ASCII | 7.38% | 10.74% | 8.72% | 11.98% |

    The model's ASCII alphabet costs Turkish about 5.5 points. The other part of
    the gap to English (about 6 points on clean pages) is the model reading
    Turkish words badly even without those letters.
- **Text path on one Turkish book,** 0.1.4 against unreleased main, against a
  Mistral OCR export (not ground truth): words matching 95.80% to 98.85%, wrongly
  split 1,222 to 16, paragraphs 1,023 to 2,308 (Mistral 2,458).
- **epublift's earlier numbers,** on one book each: about 99.8% word overlap for
  a born-digital book, about 92% word accuracy for OCR of a clean prose scan.

## Layout

- `crates/paginaverto`: the library. The `ocr` feature pulls `paginaverto-ocr`.
- `crates/paginaverto-ocr`: character recognition (ocrs + rten).
- `crates/paginaverto-wasm`: browser bindings and the web page (`demo/`),
  deployed to GitHub Pages on every push to `main`.
- `scripts/`: `bump.sh`, `preflight.sh`, `wait-runs.sh`, `release-verify.sh`.

Edition 2024, minimum Rust version 1.97.

## Build and test

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --locked --features paginaverto/ocr -- -D warnings
cargo test --workspace --locked && cargo test --workspace --locked --features paginaverto/ocr
cargo +1.97 check --workspace --locked --features paginaverto/ocr
```

The OCR accuracy tool stays out of the workspace:

```sh
cd tools/ocr-accuracy && cargo run --release -- <models dir> <font.ttf> ...
```

Test fixtures are generated in the tests from known text (`tests/generated.rs`).
None comes from anyone's documents.

A `v*` tag releases everything: the GitHub Release page from the tag's
`CHANGELOG.md` section, crates.io and npm through trusted publishing.

## Contributing and licence

External pull requests are not accepted until a CLA exists; see
`CONTRIBUTING.md`. Issues are welcome.

AGPL-3.0-only or a commercial licence (`LICENSE`, `LICENSE-COMMERCIAL.md`,
`LICENSING.md`). Extracting a document's text puts no licence on the result. The
ocrs models are CC BY-SA 4.0, and paginaverto does not distribute them. Never add
a dependency that is AGPL-only or more restrictive.
