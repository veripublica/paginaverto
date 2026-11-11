# paginaverto

Reads PDFs and page images and turns them into a structured document: a Rust
library and a CLI (`cli` feature). A veripublica family tool. epublift is the first consumer: it builds EPUBs from
paginaverto's output.

- **Repo:** `github.com/veripublica/paginaverto` (public, `main`).
- **License:** `AGPL-3.0-only OR LicenseRef-veripublica-Commercial`. Never
  depend on anything AGPL-only or more restrictive (not imagoverto). Models
  keep their own licenses (ocrs models: CC BY-SA 4.0). No external PRs until
  a CLA exists.
- **The plan:** `veripublica/paginaverto/PLAN-0.2.0.md` (0.3 is next: Turkish
  OCR, JBIG2/JPEG2000, page images). Read it before starting a step.

## Output and epublift

- 0.1 matched epublift cli-v4.5.1's PDF import. From 0.2.0 the output may change,
  and every change is measured on the 18 local PDFs and the public accuracy set.
- epublift is **read only** (`/Users/baris/Documents/Projects/epublift`).
  Questions go by note. Write our own code.
- `Cargo.lock` follows R19: shared crates come from the project before us
  (imagoverto's tag, then the ones before it), epublift follows us. Targets:
  `family-deps.py --targets paginaverto`. Never a plain `cargo update`;
  engines are pinned with `=` in the workspace.
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
cargo clippy --workspace --all-targets --locked --features paginaverto/cli -- -D warnings
cargo test --workspace --locked && cargo test --workspace --locked --features paginaverto/cli
cargo +1.97 check --workspace --locked --features paginaverto/cli
```

## Releasing

A `v*` tag releases everything: the GitHub Release page (`release.yml`), the
8 binaries on it (`release-binaries.yml`, with `dry_run` to try it untagged),
crates.io (`publish-crate.yml`: paginaverto-ocr, then paginaverto) and npm
(`publish-npm.yml`: @veripublica/paginaverto-wasm). A registry version can never be reused. **Commit, tag and push only with Baris's explicit yes,
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
6. **`release-verify` agent** runs `scripts/wait-runs.sh vX.Y.Z` (six
   runs: CI, Pages, Release, Release binaries, crates.io, npm) then
   `scripts/release-verify.sh X.Y.Z` (tag on origin, release page with the
   CHANGELOG section and registry links, both crates served with their
   licences and no local-only file, the 8 binaries and their checksums, the npm tarball with both licences, the
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

`release.yml` writes the release page from the tag's CHANGELOG section as a
draft and fails if there is none; `release-binaries.yml` publishes it once the 8
archives are on it, so `latest` always has binaries. Run that workflow's `dry_run`
on the commit to be tagged. `workflow_dispatch` with a `tag` input creates or
refreshes the page of an existing tag (notes from main's CHANGELOG).

## Data rules

- PDFs on this machine are Baris's personal documents: numbers from them may
  be reported, their text never (not in notes, issues, tests, fixtures,
  commits). Public fixtures are generated from known text or public domain.

## Family notes

@/Users/baris/Documents/Projects/veripublica/FAMILY.md
Your folder: `veripublica/paginaverto/`.
