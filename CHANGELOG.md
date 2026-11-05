# Changelog

All notable changes to `paginaverto` (and `paginaverto-ocr` and
`paginaverto-wasm`, which track the same version) are documented here. Each
version's section is also its GitHub Release notes.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
paginaverto is pre-1.0, so breaking changes land as minor-version bumps
(`0.x.0`), per [Cargo's SemVer compatibility
rules](https://doc.rust-lang.org/cargo/reference/semver.html).

## [Unreleased]

### Added

- **Turkish scans read as Turkish.** OCR can now read lines with PaddlePaddle's
  PP-OCRv5 Latin model (`LATIN_RECOGNITION`, 8 MB, Apache-2.0), whose alphabet
  holds ç ğ ı İ ö ş ü â î û and the other Latin-script letters. ocrs still finds
  the lines. On the public accuracy set, the character error rate fell from
  13.04–17.26% to 0.29–0.58% in Turkish, and from 1.20–5.46% to 0.19–0.37% in
  English. On one Turkish scan, checked against the publisher's EPUB, the share
  of its words found rose from 55.70% to 95.40%. The set is small and synthetic,
  and the scan is one book, so no language counts as supported yet. Only Turkish
  and English are measured.
- **Turkish İ and ı.** With the content language `tr` (or `az`), OCR favours the
  dotted capital İ over I and the dotless ı over i where the model hesitates
  between them. On the Turkish scan, İ found rose from 712 to 983 of 1,059, and
  words found from 94.18% to 95.40%. The language had been required for OCR but
  unused; this is its first use.
- **Every image format a PDF scan uses:** JBIG2, JPEG 2000 and CCITT fax, and
  raw images at 1, 2 or 4 bits, CMYK, indexed colour and image masks. All pure
  Rust. A scan stored this way used to be refused; the one measured now reads.
  In a PDF with a text layer, such images now become figures (PNG) instead of
  being skipped, if they look like pictures: at least 10,000 pixels, and, for
  gray ones, no longer than 8 times their width and at least 1.5% ink. In two
  books, 175 such images were looked at one by one. The rule keeps every
  drawing, chart, photo and icon seen, and drops the rules, specks and empty
  boxes; about 16 fragments of larger drawings still pass. Those books gained
  figures, 23 to 64 and 9 to 31, and their text is unchanged. Indexed-colour and
  CMYK raw images are still skipped.
- **Covers and plates of scans with a text layer:** a page whose layer holds no
  text keeps its image as a figure, unless it is blank. A page built of layers,
  as library scans build them (a background, and over it a colour layer shown
  through a text mask), is put together first. On the 18 PDFs, 34 such pages,
  each looked at: 13 kept (covers, plates, end papers and a pale worksheet), 21
  blank or holding only a library stamp or a shelf mark. A page counts as blank
  when less than 3% of it differs from its paper.
- **`paginaverto::recognises(language)`** says whether OCR with the Latin model
  can read a language: its BCP-47 tag is written in Latin letters and the model
  has every one of them. True for 51 languages and Serbian in Latin letters
  (`sr-Latn`), Turkish and English the only
  ones measured; false for other scripts, a script subtag other than `Latn`, and
  Azerbaijani, Vietnamese and Uzbek, whose letters the model lacks.
- **Constructors for tests and tools:** `Block::paragraph`, `Block::heading`,
  `Block::figure`, `Run::new` and `Figure::new`, with `Document::default()`, so a
  caller can build a document by hand.
- **Page images as input:** a PNG, JPEG or TIFF file (every page of a
  multi-page TIFF), or a folder of them, read in natural name order.
  `read` and `paginaverto -i` take them as they take a PDF; the format is found
  from the bytes. Each image is one page, sized from its resolution or taken at
  300 dpi.
- **Offline OCR, step by step,** in `AGENTS.md`: which two files to download
  once, their checksums, and how to use them on a machine with no network. The
  command-line tool's help and its error for a missing model point there.

### Changed

- **The command-line tool prefers the Latin model** when `--ocr-models` holds
  it. With only ocrs's own recognition model, which reads ASCII, the text is the
  same as in 0.2.0.
- **A text-layer page holding only an image** that decodes is reported as
  `image_not_read`, not `undecodable_images`: 29 pages in 5 of the 18 PDFs
  measured. Their text is unchanged.
- **OCR lines out of place are put back.** ocrs can give a line after the
  lines below it, which put it at the end of its page. Within each column, lines
  now go top to bottom.
- **CI runs OCR** on the public accuracy set with the published models, and fails
  above a character error rate of 2%.
- **OCR takes longer with the Latin model:** 261 s for a 292-page scan, where
  ocrs's model took 139 s.
- **`Error::UndecodableImages`** now means the images failed to decode, not
  that their format is unknown to paginaverto.
- The web binding's `.wasm` grows with the image decoders, from 2.70 to 3.59 MB
  (1.08 to 1.34 MB gzipped).

### Fixed

- **Paragraphs indented by exactly one font size** now start where they should.
  The indent had to be more than one font size, so one book read as 98
  paragraphs of over 3,000 characters, 16 of them over 6,000; now 6 and none,
  from 1,652 to 2,543 paragraphs. Five other PDFs gain 1 to 21 paragraphs, and
  the book measured against Mistral is unchanged.

## [0.2.0] - 2026-11-05

### Added

- **A public accuracy set:** known English and Turkish text, read back from
  generated PDFs. The text-layer path runs in CI on nine ways of laying out a
  line, with a character error rate of 0.00%; 0.1.4 reached 17.96% on some.
  `tools/ocr-accuracy` measures the OCR path by hand: 1.20% to 5.46% for English
  and 13.04% to 17.26% for Turkish. The set is small and synthetic, so no language
  counts as supported on it yet. Details are in `AGENTS.md`.
- **`Error::UndecodableImages`** names the formats (JBIG2, JPEG 2000, CCITT fax)
  when every page image of a scan is in one that cannot be decoded yet. Such a
  PDF used to end in `OcrFoundNothing`, which blamed the images' quality, after
  the caller had fetched the models. `Error` is `#[non_exhaustive]`, so this is
  additive; the npm package's `ErrorKind` gains the same name.

- **A structured document.** `read` returns a `Document`: the file's metadata and
  outline, each page's size and where its text came from (`text_layer`,
  `embedded_ocr`, `ocr` or `none`), and one flat list of headings with levels,
  paragraphs and figures in reading order. Blocks carry their positions and italic
  runs where the reading knows them. `Document::chapters()` gives the chapters.
- **Pages that cannot be read are reported instead of silently left empty.**
  `Page::problems` holds `ocr_failed`, `undecodable_images` or `image_not_read`, and
  the rest of the document is still returned.
- **A command-line tool.** `paginaverto -i book.pdf` writes `book.zip`, holding
  `document.json` and `figures/`. `paginaverto inspect` reports, page by page,
  whether the text comes from a text layer, an OCR layer embedded over a page image,
  or a scan only OCR can read, and lists the fonts with no usable `/ToUnicode` map.
  It follows the family's CLI convention v0.7, with the shared JSON report under
  `--format json`. Binaries for Linux, macOS and Windows, on x86_64 and ARM64, are
  on the release page. All eight were built by the release workflow, and six were
  started there; the x86_64 macOS and ARM64 Windows ones were built but never
  started. OCR has run only on an ARM64 Mac, because the models cannot be downloaded
  for CI. From crates.io:
  `cargo install --locked paginaverto --features cli`. The library has the same
  report as `paginaverto::inspect`.
- **Limits (`Options::limits`):** the file's size, the number of pages, what one
  stream and one page's content may decode to, and the pixels of one image. Going
  over one refuses the document with `Error::LimitExceeded`, which names it. The
  defaults (4 GiB, 50,000 pages, 1 GiB, 128 MiB, 256 million pixels) refused none
  of the 18 PDFs measured. Compressed object streams are covered too, which lopdf
  would otherwise drop silently, making the PDF look like a scan.
- **Cancel (`Options::cancel`):** stops a read from another thread, between pages,
  with `Error::Cancelled`.
- **Progress (`Options::progress`):** reports each page as it finishes.

### Changed

- **`read` returns a `Document`; `Book`, its `Chapter` and `Block` are gone.** This
  is the breaking change of the release. `document.chapters()` replaces
  `book.chapters`, and a block's text is `block.text()`. The npm package's `Book`
  keeps its shape and gains `language`, `outline` and `pages`; a subheading in it is
  a paragraph with a `level`.
- **Chapters follow the PDF's own table of contents.** A real outline now sets them,
  one per top-level entry: 9 to 45 chapters on a book with 44 entries, 9 to 13 with
  12, 5 to 6 with 6. Outlines that only name each page are ignored. Without an
  outline, a printed table of contents tells chapters from subheadings, which stay
  in the chapter as headings of level 2: 223 to 56 and 154 to 23 chapters on two
  books whose subheadings had each become a chapter. Measured by count only, since
  these books have no reference for their chapters. PDFs with neither read as before.
- **A scan refused for lack of OCR no longer says OCR is coming later.** It is here.
- **OCR's lines join into paragraphs,** by their gaps and first-line indents, instead
  of each line being a paragraph: 6 paragraphs of 6 on clean generated pages, where
  there were 12.
- **Pages are read in parallel,** on every core but two (`Options::threads`). The
  output does not depend on the thread count: byte-identical on 18 PDFs at 1 and 8
  threads. Text layers read about 4 times faster on 10 cores. OCR uses at most 4
  threads by default and gains less, about 1.5 times on one scan, since the models
  run on a pool of their own.
- **Better text from PDFs with a text layer.** Parity with epublift cli-v4.5.1
  ends here: this changes the output of most books.
  - Our own extractor now follows the text position properly: showing text
    advances it, character, word and horizontal spacing count, a large `TJ`
    adjustment is a word space, and `q`/`Q` save the text state.
  - It is now the first choice. lopdf's reading is the fallback where ours lost
    letters or clearly glues words. A scan's text layer placed word by word no
    longer comes out as one word per paragraph.
  - Paragraphs: one stray outdented line no longer makes a page look indented, a
    lower-case line never starts a paragraph, a change of font size or of
    capitals does, and a paragraph continues across a page.
  - Running heads: one that names a short chapter is caught, and the Turkish
    capitals (İ, Ç, Ş, …) now match when heads and headings are compared.
  - The smaller inherited faults are gone: a full-page image in the page tree's
    `/Resources` counts, a chapter's minimum body is counted in characters, the
    quality gate counts chapter titles, and text never holds Unicode
    noncharacters such as U+FFFE.

  On one Turkish book with a reference, the words matching it rose from 95.80%
  to 98.85%, and words wrongly split fell from 1,222 to 16.

## [0.1.4] - 2026-11-04

### Fixed

- **Figures stored as `/Filter [/FlateDecode /DCTDecode]` are real JPEGs now.**
  The bytes came out still deflated under an `image/jpeg` label, so an EPUB
  built from them was invalid. Every filter in front of `DCTDecode` is now
  undone, each with its own `/DecodeParms`. If that fails or does not end in a
  JPEG, there is no figure. Scans read through OCR take the same path.

  This is the one deliberate break of parity with epublift cli-v4.5.1 in the
  0.1 series. Compared with 0.1.3 on 18 local PDFs, 11 figures in two books
  changed, and all 11 are now JPEGs. Chapters, paragraphs, every other figure
  and the one refusal are the same.

### Changed

- The web page uses family-web template v8 (family-web v0.8.0): its offline
  list of family tools now names all six, imagoverto and paginaverto
  included. No visible change online, where the list already came from
  family-web.
- `README.md` is now a short introduction. The API, limits, measurements and
  layout moved to the new `AGENTS.md`.

## [0.1.3] - 2026-10-31

### Added

- **TypeScript types for `@veripublica/paginaverto-wasm`.** `read()` returned
  `any`; it now returns a `Book`. The `.d.ts` declares:
  - `Book`, `Chapter`, and `Block`, a union you narrow on `kind`
    (`"paragraph"` with `text`, `"figure"` with `mediaType`, `ext` and
    `data`);
  - `ErrorKind` and `PaginavertoError` for what `read()` throws.

  Checked with `tsc --strict`: correct use compiles; reading `data` from a
  paragraph, or using a possibly `null` title as a string, is an error. No
  change at runtime, and none to the Rust crates.

## [0.1.2] - 2026-10-31

### Added

- **On crates.io and npm.** `paginaverto` and `paginaverto-ocr` are on
  crates.io, so `paginaverto = "0.1"` replaces the git dependency. The
  browser binding is on npm as `@veripublica/paginaverto-wasm`. From here
  on, every release publishes all three from its tag, through trusted
  publishing (no stored token).

- **OCR models in ONNX form,** the files ocrs serves today:
  `paginaverto::ONNX_MODELS` names them and gives their SHA-256. The `.rten`
  files (`paginaverto::MODELS`) still load, but the ocrs bucket answers 403
  for them, so a fresh install could not get them. The format is detected
  from the bytes.
  - Measured 2026-10-31 on 18 local PDFs: the text is byte-identical with
    either form, at the same speed. That covers both scans with readable
    text, and the scan refused the same way with both.

## [0.1.1] - 2026-10-31

### Added

- **A web page:** <https://veripublica.github.io/paginaverto/> reads a PDF's
  text and chapters in the browser, with nothing installed and no upload.
  - It reads PDFs with a text layer. Scans need OCR, which the page does not
    have yet.
  - The new `paginaverto-wasm` crate gives JavaScript `read()` and
    `version()`.
  - On 18 local PDFs, its output is identical to the native build's.

### Fixed

- `Error::Open` keeps its cause. A file that is not a PDF reads "couldn't
  parse input: invalid file header" again, not only "couldn't parse input".
  Found by epublift's parity check. No output changes.

## [0.1.0] - 2026-10-31

The first release: epublift's PDF import (cli-v4.5.1), rewritten as a
library with the same output, so that epublift can build on it.

### Added

- **`paginaverto::read`** turns a PDF into a book: chapters, each with an
  optional title and its paragraphs and figures in reading order. It reads:
  - **born-digital PDFs.** Simple fonts and Type0/CID fonts with a
    `/ToUnicode` map are both read, and word spaces come from the fonts'
    real glyph widths;
  - **scans with an embedded OCR text layer.** That layer is reused;
  - **pure scans**, through character recognition (the `ocr` feature, with
    ocrs models the caller supplies).
- **Structure:**
  - running heads and feet and page numbers are removed;
  - words split by a line-break hyphen are joined;
  - headings start chapters, found by capitals, or by font size on
    born-digital pages.
- **Figures:** JPEG images come through verbatim; raw gray and RGB images
  are re-encoded as PNG.
- **Errors are values.** A PDF that cannot be read is refused with an
  `Error` saying why. A panic in a dependency comes back as
  `Error::Internal`; it never reaches the caller.
- **OCR models:** `paginaverto::MODELS` names the two ocrs model files and
  their SHA-256. paginaverto never downloads them.
  - As of 2026-10-31 the ocrs bucket no longer serves the `.rten` files.
- **Measured parity:** on 18 local PDFs, two of them scans and one refused,
  chapters, paragraphs and figures are byte-identical to epublift
  cli-v4.5.1's import, and the refusal is the same kind. epublift's own
  check, entry by entry in the built EPUBs, agrees.

### Known limits

They are kept on purpose for parity, to be fixed from 0.2.0 on, each one
measured:

- OCR reads ASCII only (the ocrs 0.13.1 model);
- non-ASCII capitals are not lowercased when headings are matched;
- paragraphs can be coarse;
- figures are placed per page;
- tables and equations come out as text;
- vector-only PDFs cannot be read;
- there are no limits, no progress report and no cancel yet.

See the README.
