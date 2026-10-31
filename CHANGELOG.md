# Changelog

## Unreleased

- Workspace: `paginaverto` (library) and `paginaverto-ocr` (character
  recognition, behind the `ocr` feature).
- Public API: `read`, `Input`, `Options`, `Ocr`, `Book`, `Chapter`, `Block`,
  `Figure`, `Error`. A panic in a dependency is returned as `Error::Internal`.
- The text layer: classification, our own content-stream reader with a
  `/ToUnicode` CMap parser and glyph-width word spacing, lopdf's extractor,
  the cleaner of the two per page, figures (JPEG verbatim, raw gray/RGB as
  PNG).
- Structure: running heads and feet, page numbers, line-break hyphens,
  headings by capitals or font size, chapters.
- OCR: ocrs 0.13.1 models from the caller's bytes, flat-field illumination
  correction. Model names, URL and SHA-256 exported.
- Everything reproduces epublift cli-v4.5.1's PDF import, quirks included
  (marked `PARITY:` in the code).
- README: what 0.1.0 does, how to use it and its OCR models, accuracy and
  parity, the known limits kept on purpose.
