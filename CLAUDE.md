# paginaverto

Reads PDFs and page images and turns them into a structured document: a Rust
library (a CLI comes in 0.2). A veripublica family tool. epublift is the
first consumer: it builds EPUBs from paginaverto's output.

- **Repo:** `github.com/veripublica/paginaverto` (public, `main`).
- **License:** `AGPL-3.0-only OR LicenseRef-veripublica-Commercial`. Never
  depend on anything AGPL-only or more restrictive (not imagoverto). Models
  keep their own licenses (ocrs models: CC BY-SA 4.0). No external PRs until
  a CLA exists.
- **The plan:** `veripublica/paginaverto/PLAN-0.1.0.md`. Read it before
  starting a step.

## 0.1.0 is a parity release (Baris, 2026-10-31)

- Same output as epublift cli-v4.5.1's PDF import (`src/pdf/` @ db50c3a):
  chapters, paragraphs and figures byte for byte, same refusals.
- **Fix nothing in 0.1.0**, not even the obvious faults; they are listed in
  the plan and fixed from 0.2.0, each measured. One exception, Baris's yes on
  2026-11-04: 0.1.4 undoes the filters in front of `DCTDecode`, because those
  figures made invalid EPUBs.
- epublift is **read only** (`/Users/baris/Documents/Projects/epublift`).
  Questions go by note. Write our own code.
- `Cargo.lock` matches epublift's versions package for package. Do not run a
  plain `cargo update`; engines are pinned with `=` in the workspace.
- Library on crates.io (`paginaverto`, `paginaverto-ocr`) and the browser
  binding on npm (`@veripublica/paginaverto-wasm`), from 0.1.2 on.

## Layout

- `crates/paginaverto`: the library. `ocr` feature pulls
  `crates/paginaverto-ocr` (ocrs + rten). The default build must never pull
  ocrs/rten (CI checks it).
- The seam between the two crates is positioned text: OCR yields `Line`s.
- `crates/paginaverto-wasm`: browser bindings + `demo/` (family-web template
  v8 copies, never edited; re-copy from a family-web tag). `pages.yml`
  deploys it to `veripublica.github.io/paginaverto/` on every push to
  `main`. Text layers only. The nav list comes from family-web's `nav.json`.
- Edition 2024, MSRV 1.97 (never newer than epublift's). `Send`, no global
  state, no network: OCR models come from the caller as bytes.
- Errors are values. `read` catches panics and returns `Error::Internal`.

## Checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --locked --features paginaverto/ocr -- -D warnings
cargo test --workspace --locked && cargo test --workspace --locked --features paginaverto/ocr
cargo +1.97 check --workspace --locked --features paginaverto/ocr
```

## Releasing

A `v*` tag releases everything: the GitHub Release page (`release.yml`),
crates.io (`publish-crate.yml`: paginaverto-ocr, then paginaverto) and npm
(`publish-npm.yml`: @veripublica/paginaverto-wasm). No binaries until there
is a CLI. A registry version can never be reused. **Commit, tag and push only with Baris's explicit yes,
each time, asked right before the tag** — an earlier yes to the work does
not cover the release. The family's model (kepubverto, epubsana): scripts do
the work, the agents run them, the main session reviews, asks, tags, pushes.

1. **Notes first.** Every change goes into `CHANGELOG.md` under
   `## [Unreleased]`, Keep a Changelog headings (`### Added`, `### Changed`,
   `### Fixed`, `### Security`), written for a reader of the release page.
   That section *is* the release notes.
2. `scripts/bump.sh X.Y.Z`: workspace version, the `=X.Y.Z` pin of
   paginaverto-ocr, `Cargo.lock` (three entries only), `[Unreleased]` →
   `[X.Y.Z] - YYYY-MM-DD` under a fresh empty `[Unreleased]`. Read the diff,
   sweep `README.md` for stale numbers, commit `Release X.Y.Z` (no
   attribution).
3. **`release-prep` agent** runs `scripts/preflight.sh X.Y.Z`: version and
   lock, notes present and `[Unreleased]` empty, clean tree, tag free, not
   behind origin, no attribution since the last tag, licence copies match,
   the version absent from crates.io and npm, every CI gate (fmt, clippy ×3,
   tests ×2, doc, no ocrs/rten by default, wasm-pack, `cargo publish
   --workspace --dry-run`, no local-only file in the packages, audit, MSRV).
   Quote its last line (`READY` / `NOT READY`).
4. **Ask Baris**, with the version, the notes, and the preflight verdict.
5. On his yes: `git tag -a vX.Y.Z -m "…"` and `git push origin main vX.Y.Z`
   (main and tag together).
6. **`release-verify` agent** runs `scripts/wait-runs.sh vX.Y.Z` (five
   runs: CI, Pages, Release, crates.io, npm) then
   `scripts/release-verify.sh X.Y.Z` (tag on origin, release page with the
   CHANGELOG section and registry links, both crates served with their
   licences and no local-only file, the npm tarball with both licences, the
   `.d.ts` and the `.wasm`, npm `latest`, the tag as a git dependency, web
   page up). Quote both verdicts. Don't rename the two publish workflows:
   the trust links name them.
7. **After:** note to `epublift/inbox/` (version, what changed, the line to
   depend on), copy to conventions when the release matters to the family.

**Trusted publishing is linked** (2026-10-31): crates.io, both crates →
`publish-crate.yml`; npm, the package → `publish-npm.yml`. 0.1.3 was the
first release published entirely by the workflows. Check the crates.io link
any time with `publish-crate.yml`'s `verify_only` (publishes nothing). Claude
never enters a token; a registry login, if ever needed again, is Baris's.

`release.yml` writes the release page from the tag's CHANGELOG section and
fails if there is none. `workflow_dispatch` with a `tag` input creates or
refreshes the page of an existing tag (notes from main's CHANGELOG).

## Data rules

- PDFs on this machine are Baris's personal documents: numbers from them may
  be reported, their text never (not in notes, issues, tests, fixtures,
  commits). Public fixtures are generated from known text or public domain.

## Family notes — cross-project channel

`/Users/baris/Documents/Projects/veripublica/` (no git, never published) is
where the family's sessions leave each other notes. Rules: its `PROTOCOL.md`.

- **At session start:** a hook prints how many notes wait in
  `veripublica/paginaverto/inbox/`. Mention the count; **do not read them
  until Baris asks.**
- **Sending:** copy `veripublica/TEMPLATE.md` to
  `veripublica/<recipient>/inbox/YYYY-MM-DD_paginaverto_<recipient>_<slug>.md`.
  First paragraph = what you need from them. Only ever add files to another
  project's inbox.
- **Done with a note** (answered, acted on, read-if-fyi, or Baris says so):
  move it to `veripublica/paginaverto/archive-inbox/`, unchanged. Never
  delete.
- **Keep this file under ~2,000 words** (R13): what is true now. History goes
  to `veripublica/paginaverto/HISTORY.md`.
- **Never credit Claude, Claude Code or Anthropic on GitHub** — not in
  commits (no `Co-Authored-By` trailer), PR or issue bodies ("Generated with
  Claude Code"), release notes, code or docs. Check every commit message and
  PR body before it goes out.
- **Releases:** the project's release checklist (its own commands) lives in
  this file's "Releasing" section or a file it points to. Use the
  `release-prep` agent for the pre-commit checks and `release-verify` after the
  tag is pushed. Commit, tag and push stay in the main session, with Baris's
  approval.
- **Never write into another project's repository.** Public issues stay the
  decision record; notes carry the reasoning.
- Files this project wants kept but not in its repo go in
  `veripublica/paginaverto/`.
- **Before naming a new long option,** check `veripublica/NAMES.md`. If the
  name is not there, claim it by note to `conventions/inbox/` before writing
  the code (R14).
- **Drafts and working files** (a comment before posting, a script, an
  intermediate result) go in `veripublica/paginaverto/scratchpad/`, not
  the harness's temporary directory. Tidy it yourself; deleting there is fine.
  **Bulk or generated data** (extracted corpora, build outputs, many files a
  long run reads or writes) goes to an unsynced path such as the harness's
  temporary directory, never under `veripublica/` (iCloud): real copies, not
  symlinks, deleted after the run; only the numbers come back (R1).
- **Long jobs use every core but two;** nothing long runs single-threaded (R1).
- **Public posts** (MobileRead, Bluesky, …): read `veripublica/POSTING.md` and
  `veripublica/platforms/<platform>.md` before writing one; save it in
  `veripublica/paginaverto/<platform>_posts/` before it goes out. A lesson
  learned on a post goes to `conventions/inbox/`.
