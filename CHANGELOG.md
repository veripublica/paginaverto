# Changelog

## Unreleased

- Workspace: `paginaverto` (library) and `paginaverto-ocr` (character
  recognition, behind the `ocr` feature).
- Public API: `read`, `Input`, `Options`, `Ocr`, `Book`, `Chapter`, `Block`,
  `Figure`, `Error`. A panic in a dependency is returned as `Error::Internal`.
