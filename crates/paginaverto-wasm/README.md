# paginaverto-wasm

WebAssembly bindings for [paginaverto](https://github.com/veripublica/paginaverto): read a PDF's text
and chapters in the browser. The bytes never leave the page.

Live: <https://veripublica.github.io/paginaverto/>

```sh
npm install @veripublica/paginaverto-wasm
```

```js
import { read } from "@veripublica/paginaverto-wasm"; // bundler build
// or, without a bundler: wasm-pack build --target web, then
// import init, { read } from "./pkg/paginaverto_wasm.js"; await init();
const book = read(new Uint8Array(await file.arrayBuffer()));
// book.chapters: [{ title, blocks: [{ kind: "paragraph", text } | { kind: "figure", mediaType, ext, data }] }]
// A refusal throws an Error with `kind` (Open, ScanNeedsOcr, UndecodableFonts, NoText, QualityGate, Internal).
```

- **Text layers only.** OCR is not built for the browser; a scan throws
  `ScanNeedsOcr`.
- A panic cannot be caught in WebAssembly: it traps the instance. The demo
  runs the reader in a worker and starts a new one after a trap.
- Licence: AGPL-3.0-only OR the veripublica commercial licence (`LICENSE`,
  `LICENSE-COMMERCIAL.md`, both in the package).
- The demo page (`demo/`) uses family-web's template v7 (family-web v0.7.1);
  `tokens.css` and `demo.css` are unedited copies.

Build and serve locally:

```sh
wasm-pack build crates/paginaverto-wasm --target web --no-pack
python3 -m http.server -d crates/paginaverto-wasm 8000   # then open /demo/
```
