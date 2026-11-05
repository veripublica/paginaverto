# paginaverto: detail for code agents and maintainers

`README.md` is the short introduction. This file holds the detail and is kept
true at every release. Past changes: `CHANGELOG.md`. API docs:
<https://docs.rs/paginaverto>.

Figures below name the versions they were measured on. Current version: 0.1.4.

## Scope

paginaverto reads PDFs and turns them into a structured document. It ships as a
Rust library (crates `paginaverto` and `paginaverto-ocr`) and a WebAssembly
package (`@veripublica/paginaverto-wasm`), released together under one version.
It also ships a CLI (below), with binaries for eight targets. The web page at
<https://veripublica.github.io/paginaverto/> runs the WebAssembly build; the
file never leaves the browser.

0.1 was a parity release: it did what
[epublift](https://github.com/ePubLift/epublift)'s PDF import did in cli-v4.5.1.
From 0.2.0 the output changes, and every change is measured
([Measurements](#measurements)). epublift builds on it.

What it reads into a `Document`, metadata, pages, and headings, paragraphs and
figures in reading order, from three kinds of PDF:

1. **Born-digital,** with a text layer. Simple fonts and composite (Type0/CID)
   fonts with a `/ToUnicode` map are read. Word spaces come from the fonts' real
   glyph widths.
2. **A scan with an embedded OCR text layer.** That layer is reused as it is.
3. **A pure scan.** The page images go through
   [ocrs](https://github.com/robertknight/ocrs), behind the `ocr` feature.

It then removes running heads and feet and page numbers, joins words split by a
line-break hyphen, and finds the headings. Chapters start where the PDF's own
outline says, when it is a real table of contents. Otherwise a heading is what
looks like one (capitals, or a large font on a born-digital page), and a printed
table of contents, if the book has one, tells the chapters from the subheadings.

Where it is going: lists, captions and footnotes linked to where they are
referenced, figures at their positions, language and reading direction on each
part, and page images as input. The format already has room for them.

Rules the code follows:

- **Errors are values.** `read` catches panics in dependencies and returns
  `Error::Internal`.
- **No network and no global state; the API is `Send`.** OCR models come from
  the caller as bytes.
- **The default build never pulls ocrs or rten.** CI checks it.
- **The seam between the two crates is positioned text:** OCR yields `Line`s, so
  the PDF side and the structure step never depend on the OCR engine.

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
paginaverto = "0.2"
# With OCR for scans:
# paginaverto = { version = "0.2", features = ["ocr"] }
```

On crates.io from 0.1.2; 0.1.0 and 0.1.1 are git tags only. The JavaScript
package is `@veripublica/paginaverto-wasm` on npm. It reads text layers only.

## API

```rust
pub fn read(input: Input<'_>, opts: &Options) -> Result<Document, Error>;

enum Input<'a> { Path(&'a Path), Bytes(&'a [u8]) }
#[non_exhaustive] struct Options {
    language: Option<String>, ocr: Ocr, limits: Limits,
    threads: Option<usize>, cancel: Cancel, progress: Option<Progress>,
}
enum Ocr { Off, Models { detection: Vec<u8>, recognition: Vec<u8> } }
#[non_exhaustive] struct Document {
    language: Option<String>, metadata: Metadata, pages: Vec<Page>, blocks: Vec<Block>,
}
impl Document { fn chapters(&self) -> Vec<Chapter<'_>>; }
struct Chapter<'a> { title: Option<String>, blocks: &'a [Block] }
struct Metadata {
    info: BTreeMap<String, String>, xmp: Option<String>,
    catalog_lang: Option<String>, outline: Vec<OutlineEntry>,
}
struct OutlineEntry { level: u32, title: String, page: Option<u32> }
struct Page {
    number: u32, width: f32, height: f32,
    text_source: TextSource, problems: Vec<PageProblem>,
}
enum TextSource { TextLayer, EmbeddedOcr, Ocr, None }
enum PageProblem { OcrFailed, UndecodableImage(Option<&'static str>), ImageNotRead }
struct Block { id: String, kind: BlockKind, boxes: Vec<BlockBox>, confidence: Option<f32> }
impl Block { fn text(&self) -> String; fn runs(&self) -> &[Run]; fn level(&self) -> Option<u32>; }
enum BlockKind { Heading { level: u32, runs: Vec<Run> }, Paragraph { runs: Vec<Run> }, Figure(Figure) }
struct Run { text: String, italic: bool }
struct BlockBox { page: u32, x: f32, y: f32, width: f32, height: f32 }
struct Figure { data: Vec<u8>, media_type: &'static str, ext: &'static str }
#[non_exhaustive] enum Error {
    Open(String), ScanNeedsOcr, LanguageRequired, Models(String),
    UndecodableImages(Vec<String>), OcrFoundNothing, UndecodableFonts, NoText,
    QualityGate, LimitExceeded { what: LimitKind, limit: u64 }, Cancelled,
    Internal(String),
}
```

A scan's checks come in this order: images no build can decode yet
(`UndecodableImages`: JBIG2, JPEG 2000, CCITT fax, when every page's image is one
of them), then OCR off, then language missing, then models.

```rust
use paginaverto::{Input, Options, read};

let doc = read(Input::Path("book.pdf".as_ref()), &Options::default())?;
for chapter in doc.chapters() {
    println!("{:?}", chapter.title);
    for block in chapter.blocks {
        println!("{}", block.text()); // empty for a figure
    }
}
for page in doc.pages.iter().filter(|p| !p.problems.is_empty()) {
    println!("page {}: {:?}", page.number, page.problems);
}
```

### The document

Blocks are one flat list in reading order; a chapter is the span after a heading
of level 1. Rules that hold across the Rust types and the CLI's file:

- **Pages** are numbered from 1. `text_source` is `text_layer`, `embedded_ocr`,
  `ocr` or `none`. `embedded_ocr` did not go through ocrs or its alphabet.
- **A page that cannot be read does not fail the document.** `problems` holds
  `ocr_failed`, `undecodable_images`, or `image_not_read`: a page with only an
  image, in a document read from its text layer, where OCR does not run. Whole
  documents are still refused, as `Error` lists.
- **Boxes** are in PDF points from the visible page's top left, after its
  rotation, one per page a block touches. For text they follow the font's size
  rather than the drawn glyphs. An empty list means the position is not known:
  a figure, a title only the outline holds, a page read through lopdf (most
  embedded OCR layers), and OCR on a turned page.
- **Heading levels.** With a real outline, an entry found on its page or the next
  is a heading at its level. A top-level entry found on neither still starts a
  chapter, with the outline's title, at the top of its page. A single entry at the
  top gives way to the level below it. A heading the outline does not know sits
  below every level it has. Without an outline, a heading that a printed table of
  contents lists is level 1 and one it does not list is level 2; with no such page
  every heading is level 1. A heading other than an outline entry needs 400
  characters of body after it, or it stays a paragraph.
- **An outline is a table of contents** when at least half its entries have a page
  and at least half have distinct titles. Scanning software writes one entry per
  page named after its number; those are ignored and still given raw in `metadata`.
- **Italics** come from the font: its name, its slant or its italic flag. Text read
  through lopdf or OCR has none.
- **`language`** is the option, else the catalog's `/Lang`, else none. Nothing is
  guessed. **`confidence`** is always `None`: ocrs gives only a character and a
  rectangle.

Not produced yet, though the format has room: `footnote`, `list`, `caption` and
`table` blocks, figure boxes, and language or direction below the document.

The CLI's file has the same shape as JSON. `schema_version` (`SCHEMA_VERSION`, 1)
rises only for a breaking change; new kinds and fields do not raise it. A reader
shows a kind it does not know as a paragraph, from its `runs`.

```json
{
  "schema_version": 1,
  "producer": { "tool": "paginaverto", "version": "0.2.0" },
  "language": "tr",
  "metadata": {
    "info": { "Title": "…" }, "xmp": null, "catalog_lang": "tr",
    "outline": [ { "level": 1, "title": "…", "page": 7 } ]
  },
  "pages": [
    { "number": 7, "width": 419.5, "height": 595.3, "text_source": "text_layer", "problems": [] }
  ],
  "blocks": [
    { "id": "b1", "kind": "heading", "level": 1, "runs": [ { "text": "…" } ],
      "boxes": [ { "page": 7, "x": 72.0, "y": 90.5, "width": 200.0, "height": 18.0 } ],
      "confidence": null },
    { "id": "b2", "kind": "paragraph",
      "runs": [ { "text": "…" }, { "text": "…", "italic": true } ],
      "boxes": [], "confidence": null },
    { "id": "b3", "kind": "figure", "file": "figures/fig001.jpg",
      "media_type": "image/jpeg", "boxes": [] }
  ]
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

`inspect(input, &opts)` returns an `Inspection`: whether `read` would treat the
document as a scan, each page's size and text source, and the fonts in use that
have no usable `/ToUnicode`. It needs no OCR.

### Command line

Build with the `cli` feature (it also turns on `ocr`):
`cargo install --locked paginaverto --features cli`. It follows CLI.md v0.7.

```sh
paginaverto -i book.pdf                 # convert, the default: writes book.zip
paginaverto convert -i scan.pdf --language tr --ocr-models models/ -o scan.zip
paginaverto inspect -i book.pdf
paginaverto inspect -i a.pdf -i b.pdf --format json
```

`convert` takes one input and writes a zip of `document.json` and `figures/`:
`-o`, `-f`, `--language`, `--ocr-models <DIR>` (the `.onnx` files, else the
`.rten`, refused unless their SHA-256 matches), `--threads`, `--format`. It never
replaces a file without `-f`, never writes over its input, and a partial file
never takes the output's name. On a terminal, stderr counts the pages; a pipe or
log stays clean.

Exit `0`: the document was written. `1`: written, but a scan lost text on a page.
`2`: could not run, or the PDF was refused, and nothing was written.

Its `--format json` adds `output`, and `summary` holds `pages`, `chapters`,
`blocks`, `figures`, `text_layer`, `embedded_ocr`, `ocr`, `none`, `errors`,
`warnings`. Its `items` are findings on `page N`, with `data` `page`, `problem`:
`PV-OCR-FAILED` (error), `PV-IMAGE-UNDECODABLE` (error in a document read by OCR,
warning otherwise), `PV-IMAGE-NOT-READ` (warning).

`inspect` takes one input or more. Exit `0`: every input read, nothing makes its
text unreadable. `1`: a font whose text cannot be decoded, or a scan with a page
image that cannot be decoded. `2`: could not run, or could not read an input.

`--format json` is the family's envelope (FORMATS.md). `inspect`'s tool-owned parts:

- `summary`: `pages`, `text_layer`, `embedded_ocr`, `scan`, `empty`,
  `reads_as_scan`, `errors`, `warnings`, `infos`.
- `items`, every one a finding with `location` set to `page N` or the font's name:
  - `PV-PAGE` (info), per page; `data`: `page`, `source`, `width`, `height`;
  - `PV-IMAGE-UNDECODABLE` (error in a scan, warning otherwise); `data`: `page`,
    `format`;
  - `PV-FONT-UNREADABLE` (error) and `PV-FONT-NO-TOUNICODE` (info); `data`:
    `subtype`, `pages`.

The page source is read from what the page holds: a page of text over a whole-page
image counts as `embedded_ocr`, whoever made that text. A simple font called
readable can still decode wrongly, if its encoding names glyphs without meaning.

### WebAssembly

`read(pdf: Uint8Array)` returns a `Book`; `version()` returns the version. The
`.d.ts` declares `Book` (`language`, `outline`, `pages`, `chapters`), `Page`,
`OutlineEntry`, `Chapter`, `Run`, `Block` (a union you narrow on `kind`:
`"paragraph"` with `text` and `runs`, and `level` when it is a subheading;
`"figure"` with `mediaType`, `ext` and `data`), `ErrorKind` and
`PaginavertoError`, which is what `read` throws.

## Limits

- OCR reads ASCII only. The ocrs 0.13.1 model is trained on ASCII text, so it
  cannot produce ç ğ ı İ ö ş ü, other accented letters or non-Latin scripts. The
  text-layer path has no such limit.
- Figures are placed after their page's text, not at their exact position.
  JPEG 2000, CCITT, JBIG2 and CMYK images are skipped.
- A document classified as a scan goes through OCR on every page, and scans yield
  no figures.
- Tables and equations come out as plain text.
- A PDF made only of vector drawings cannot be read: it has no text layer and no
  image to recognise.
- Cancel is checked between pages, so it waits for the pages being read, and not
  during the load or while OCR's models load.
- The limits bound the work, but are no guarantee of a memory ceiling: a page's
  content is parsed into many times its own size.
- The language is required for OCR, and recorded, but OCR does not use it.
- Chapters from a printed table of contents and from the look of the text are
  counted, not checked against a reference. See [Measurements](#measurements).
- An outline's chapter that is found on neither its page nor the next starts at
  the top of its page, wherever its text really begins, and its printed title stays
  in the text as a paragraph.

Since 0.1.4, figures with
filters in front of `DCTDecode` (`/Filter [/FlateDecode /DCTDecode]`) have those
filters undone. epublift's import emitted the still-deflated bytes under an
`image/jpeg` label, which made an invalid EPUB. If the filters cannot be undone,
or the result is not a JPEG, there is no figure.

The engines are pinned exactly (lopdf 0.45.0, ocrs 0.13.1, rten 0.26.0, image
0.25.10), because their behaviour is part of the output. They have not moved in 0.2.0; each
move changes the output, so each will be measured on its own.
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
- **Public accuracy set, 2026-11-05, 0.2.0.** Known text, about 1,100
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
    | Turkish | 13.04% | 16.01% | 14.09% | 17.26% |
    | Turkish, letters folded to ASCII | 7.38% | 10.74% | 8.72% | 12.18% |

    Paragraphs found, of the 6 in each text: 6 on every clean page, up from 12.
    Noisy pages give 16 to 19, down from 22 to 27: the lines join correctly, and the
    rest are stray letters OCR sees in the noise.

    The model's ASCII alphabet costs Turkish about 5.5 points. The other part of
    the gap to English (about 6 points on clean pages) is the model reading
    Turkish words badly even without those letters.
- **Text path on one Turkish book,** 0.1.4 against 0.2.0, against a
  Mistral OCR export (not ground truth): words matching 95.80% to 98.85%, wrongly
  split 1,222 to 16, paragraphs 1,023 to 2,308 (Mistral 2,458).
- **epublift's earlier numbers,** on one book each: about 99.8% word overlap for
  a born-digital book, about 92% word accuracy for OCR of a clean prose scan.

- **Chapters, 0.2.0,** 2026-11-05, same 18 PDFs, against the commit
  before. Eleven are byte-identical, and one is refused the same. Three kept every
  word and only changed levels, with no usable outline, so a printed table of
  contents decided: 154 to 23, 223 to 56 and 28 to 23 chapters. Their outlines,
  where they have one, are one entry per page with 1 or 3 distinct titles among 299
  and 570, and one more of that kind is ignored at 8 of 37. Three have real outlines
  and now have one chapter per top-level entry, plus what comes before the first:
  5 to 6 (6 entries), 9 to 13 (12), 9 to 45 (44). They lost no letters. Of those 62
  entries, 31 were found on their page or the next, 31 are titles only the outline
  holds.
- **OCR paragraphs, 0.2.0,** 2026-11-05: on the two scans read by OCR,
  357 to 278 paragraphs on one with few letters on its pages, 26 to 5 on the other,
  with the same letters on that one. The first keeps 61 more letters of 543; a short
  line that now joins a paragraph is no longer dropped as a head or page number,
  which is the likely reason, not checked. The other 15 PDFs that read are unchanged.
- **Italics, 0.2.0,** 2026-11-05, on one Turkish book against Mistral's
  export, which is not ground truth. Of Mistral's 107 italic spans counted here, 102
  are italic in ours and 105 are in our text at all. Ours has 169 italic runs, 100 of
  which Mistral marks; whether the other 69 are right is not known.
- **Boxes, 0.2.0,** 2026-11-05, share of text blocks with a position:
  99 to 100% on 9 PDFs, 43 to 85% on 3, and 0 to 4% on 5, which are read through
  lopdf or by OCR on a turned page. One scan is refused.
- **`inspect`, 0.2.0,** 2026-11-05, same 18 PDFs: all read. The one
  known JBIG2 scan reports 292 of 292 pages undecodable, exit `1`. In 7 of the PDFs
  with text, 1 to 23 image-only pages are in a format that cannot be decoded; those
  report warnings, exit `0`.
- **Threads, 0.2.0,** 2026-11-05, same 18 PDFs on 10 cores: chapters,
  paragraphs and figures byte-identical to one thread and to the commit before.
  That is at 1 and 8 threads, and at 1, 2, 4 and 8 on the two scans. Text layers
  read 3.7 to 5.1 times faster at 8 threads on the larger books. OCR, on one small
  scan repeated to 180 pages: 13.6 s and 363 MB at 1 thread, 10.5 s and 594 MB at
  2, 9.3 s and 961 MB at 4, 9.3 s and 1,387 MB at 8. Hence at most 4 by default.
  Larger page images would cost more memory; that is not measured. The default
  limits refused none of the 18.

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
cargo clippy --workspace --all-targets --locked --features paginaverto/cli -- -D warnings
cargo test --workspace --locked && cargo test --workspace --locked --features paginaverto/cli
cargo +1.97 check --workspace --locked --features paginaverto/cli
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
