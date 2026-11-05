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

- **A public accuracy set:** known English and Turkish text, read back from
  generated PDFs. The text-layer path runs in CI on nine ways of laying out a
  line, with a character error rate of 0.00%; 0.1.4 reached 17.96% on some.
  `tools/ocr-accuracy` measures the OCR path by hand: 1.20% to 5.46% for English
  and 13.04% to 17.07% for Turkish. The set is small and synthetic, so no language
  counts as supported on it yet. Details are in `AGENTS.md`.
- **`Error::UndecodableImages`** names the formats (JBIG2, JPEG 2000, CCITT fax)
  when every page image of a scan is in one that cannot be decoded yet. Such a
  PDF used to end in `OcrFoundNothing`, which blamed the images' quality, after
  the caller had fetched the models. `Error` is `#[non_exhaustive]`, so this is
  additive; the npm package's `ErrorKind` gains the same name.

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
