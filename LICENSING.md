# Licensing, in plain terms

`paginaverto` is dual-licensed: **AGPL-3.0-only** ([`LICENSE`](./LICENSE)) **OR** a
commercial license ([`LICENSE-COMMERCIAL.md`](./LICENSE-COMMERCIAL.md)).

This page answers the questions people ask before adopting it, and it is short
on purpose. **The full answers, including the GPL-compatibility reasoning
clause by clause, are in one shared FAQ for the veripublica tools:**

**→ [Licensing FAQ](https://github.com/veripublica/epubsana/blob/main/LICENSING-FAQ.md)**

There is only one canonical text, so that the projects cannot drift into
saying different things. This page does not repeat its legal reasoning. It
covers what is specific to `paginaverto`: the documents it reads, and the
recognition models it uses. It ends with the commitment behind all of it.

---

## Is the text I extract covered by your license?

**No. Unconditionally.**

The license governs this software, not the data it processes. Reading a PDF
or a page image with `paginaverto` no more licenses that document, or the text
taken from it, than a compiler licenses the program it compiles.

What `paginaverto` produces is your document's own text and images, arranged
by structure: headings, paragraphs, footnotes and figures. It adds nothing
expressive of ours. Whether you may use the document itself is between you and
its rights holder. That question was the same before you ran this tool.

## I digitise documents commercially. Do I need the commercial license?

**No.** You need no commercial license and no permission to run
`paginaverto`: on your own documents or on customers' documents, inside your
company, as often as you like, and for money. How you may *use* it is the same
for hobbyists and companies.

## What is the commercial license actually for?

It covers two narrow cases. Both are about **distributing or serving this
code**, not about using it:

1. **Embedding `paginaverto` in a closed-source product you distribute**, such
   as a reading app, a scanner's software or a digitisation pipeline, without
   meeting the AGPL's source-disclosure obligations.
2. **Running a modified version as a network service** without publishing your
   modifications.

Nothing else. If you are not doing one of those two things, the AGPL is free
and sufficient.

## What about the recognition models?

Character recognition uses trained models. **A model is not code, and it is
not covered by either of the licenses above.** Each model keeps its own
license, which this project states where it uses the model.

- The models of the [ocrs](https://github.com/robertknight/ocrs) engine are
  trained on Google Research's HierText dataset. They are licensed under
  **CC BY-SA 4.0**, which allows commercial use. If you redistribute them,
  give the attribution their authors ask for. If you change a model and
  distribute it, the changed model is also CC BY-SA 4.0.
- If this project ever trains a model of its own, this page will name the
  data it was trained on and the license that follows from that.

The commercial license covers `paginaverto`'s code. It cannot relicense a
model that someone else trained.

## Can a Sigil or calibre plugin use `paginaverto`? Can GPL-3.0 software?

Yes to both. The reasoning is a licence-compatibility argument, and it is
kept in one place: the
**[Licensing FAQ](https://github.com/veripublica/epubsana/blob/main/LICENSING-FAQ.md)**.
It walks through GPLv3 §13 and AGPLv3 §13 and quotes both.

## Contributing

External contributions are not accepted yet. Selling commercial licenses
requires the copyright holder to hold full copyright, so a CLA has to exist
before any external code can be merged, and that mechanism is not built yet.

---

## The commitment, and the disclaimer

I am not a lawyer, and nothing here is legal advice. Where this page and
[`LICENSE`](./LICENSE) could be read differently, **`LICENSE` governs.**

What is *not* hedged is the commitment itself, which I can make plainly as the
copyright holder: **I will not come after users, plugin authors, or editor
projects.** The AGPL is here because work of mine was once closed and
commercialised by someone else and I got nothing back. It is aimed at that,
and at nothing that a person reading their own documents is doing.

Questions: baris@kayadelen.com
