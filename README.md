# paginaverto

Reads PDFs and page images (scans and photos) and turns them into a
structured document, from headings to footnotes. It is a Rust library, and a
command-line tool for people and scripts who want the text and structure of a
document without writing code. Part of the
[veripublica](https://github.com/veripublica) tools. The name is Latin:
*pagina*, page, and *verto*, to turn.

**Status:** being set up. Nothing is released yet.

## Scope

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

## Licence

AGPL-3.0-only OR a commercial license ([LICENSE](LICENSE),
[LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md)). Extracting a document's text
with `paginaverto` puts no license on the result. Recognition models keep
their own licenses. See [LICENSING.md](LICENSING.md).
