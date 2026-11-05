# paginaverto

paginaverto reads a PDF and gives you its text and structure: chapters, paragraphs and figures. It handles PDFs with a text layer and, with OCR switched on, plain scans. It's a Rust library and a command-line tool, and it's what [epublift](https://github.com/ePubLift/epublift) uses to import PDFs. The name is Latin for turning pages.

The easiest way to try it is [the web page](https://veripublica.github.io/paginaverto/). Your file stays in your browser and nothing is uploaded. The page only reads PDFs with a text layer, because OCR isn't in the browser yet.

To use the library, add it with `cargo add paginaverto`, or `cargo add paginaverto --features ocr` for scans. Then:

```rust
use paginaverto::{Input, Options, read};

let doc = read(Input::Path("book.pdf".as_ref()), &Options::default())?;
for chapter in doc.chapters() {
    println!("{:?}", chapter.title);
    for block in chapter.blocks {
        println!("{}", block.text());
    }
}
```

The command-line tool is on the [releases page](https://github.com/veripublica/paginaverto/releases/latest), or `cargo install --locked paginaverto --features cli`. `paginaverto -i book.pdf` writes `book.zip`, with the document as JSON and its figures.

For JavaScript there is [`@veripublica/paginaverto-wasm`](https://www.npmjs.com/package/@veripublica/paginaverto-wasm) on npm.

This is an early version. So far I've made it do what epublift's own PDF import did, faults included, so that I could check the switch byte by byte. I'm fixing those faults one at a time from 0.2, and that's also when a command-line tool and a richer document format arrive. OCR only reads ASCII for now, so Turkish scans lose their letters, and I don't call any language supported until I've measured it.

It runs offline and never downloads anything. You provide the OCR models yourself. The API, the known limits and the measurements are in [AGENTS.md](https://github.com/veripublica/paginaverto/blob/main/AGENTS.md).

The documents you read are yours, and extracting their text puts no licence on the result. The code is AGPL-3.0-only, or under a commercial licence if you want to embed it in a closed-source product or run a modified version as a service. [LICENSING.md](https://github.com/veripublica/paginaverto/blob/main/LICENSING.md) has the details.
