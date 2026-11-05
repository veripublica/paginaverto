#!/usr/bin/env bash
#
# Step 2 of a release: everything CI runs, plus what the release needs, before
# the tag. kepubverto's and epubveri's scripts/preflight.sh are the model.
#
# Usage:
#   scripts/preflight.sh          # check the version in Cargo.toml
#   scripts/preflight.sh 0.2.0    # check a version you are about to tag
#
# Every check runs even after one fails. The last line is the verdict:
# READY or NOT READY. READY means the tag may be pushed — after Baris says
# yes, each time.

set -uo pipefail
cd "$(dirname "$0")/.."

PASS=0; FAIL=0; FAILED=()
T=$SECONDS
took() { local d=$((SECONDS - T)); T=$SECONDS; printf ' [%ds]' "$d"; }
ok()   { printf '  ok    %s' "$1"; took; echo; PASS=$((PASS + 1)); }
bad()  { printf '  FAIL  %s' "$1"; took; echo; FAIL=$((FAIL + 1)); FAILED+=("$1"); }
skip() { printf '  --    %s' "$1"; took; echo; }

check() {
  local label="$1"; shift
  local out
  if out=$("$@" 2>&1); then ok "$label"; else
    bad "$label"; printf '%s\n' "$out" | tail -15 | sed 's/^/        /'
  fi
}

WS_V=$(grep -m1 '^version = "' Cargo.toml | cut -d'"' -f2)
PIN_V=$(sed -nE 's/^paginaverto-ocr = \{.*version = "=([0-9.]+)".*/\1/p' Cargo.toml)
V="${1:-$WS_V}"
echo "version $V (workspace $WS_V, paginaverto-ocr pin $PIN_V)"

# ------------------------------------------------------------------ version
[ "$WS_V" = "$V" ] && ok "workspace version is $V" || bad "workspace version is $WS_V, not $V"
[ "$PIN_V" = "$V" ] && ok "paginaverto-ocr pinned =$V" || bad "paginaverto-ocr pinned =$PIN_V, not =$V"
for p in paginaverto paginaverto-ocr paginaverto-wasm; do
  lv=$(awk -v p="$p" '$0=="name = \""p"\""{getline; print; exit}' Cargo.lock | cut -d'"' -f2)
  [ "$lv" = "$V" ] && ok "Cargo.lock has $p $V" || bad "Cargo.lock has $p $lv, not $V"
done
# The release notes: the section must exist and not be empty.
NOTES=$(awk -v v="$V" '$0 ~ "^## \\[" v "\\]"{f=1;next} f&&/^## \[/{exit} f&&NF{n++} END{print n+0}' CHANGELOG.md)
[ "$NOTES" -gt 0 ] && ok "CHANGELOG.md has a [$V] section ($NOTES lines: the release notes)" \
  || bad "CHANGELOG.md has no [$V] section, or it is empty"
awk '/^## \[Unreleased\]/{f=1;next} f&&/^## \[/{exit} f&&NF{n++} END{exit n?1:0}' CHANGELOG.md \
  && ok "[Unreleased] is empty (nothing left out of $V)" \
  || bad "[Unreleased] still has entries: move them into [$V]"
# The licence files each package ships are copies of the repository's.
for c in paginaverto paginaverto-ocr paginaverto-wasm; do
  for f in LICENSE LICENSE-COMMERCIAL.md; do
    cmp -s "$f" "crates/$c/$f" && ok "crates/$c/$f matches $f" || bad "crates/$c/$f differs from $f (re-copy it)"
  done
done
# paginaverto-ocr builds in PaddleOCR's character list, so it ships the notice.
f=THIRD-PARTY-NOTICES.md
cmp -s "$f" "crates/paginaverto-ocr/$f" && ok "crates/paginaverto-ocr/$f matches $f" \
  || bad "crates/paginaverto-ocr/$f differs from $f (re-copy it)"

# ------------------------------------------------------------------ tree
if [ -z "$(git status --porcelain)" ]; then ok "working tree clean"
else bad "working tree dirty"; git status --short | sed 's/^/        /'; fi

git rev-parse -q --verify "refs/tags/v$V" >/dev/null && bad "tag v$V exists locally" || ok "tag v$V free locally"
if git fetch -q origin 2>/dev/null; then
  B=$(git rev-list --count HEAD..origin/main)
  [ "$B" = 0 ] && ok "main is not behind origin" || bad "main is $B commit(s) behind origin/main"
  git ls-remote -q --exit-code --tags origin "v$V" >/dev/null && bad "tag v$V exists on origin" \
    || ok "tag v$V free on origin"
else
  bad "git fetch failed"
fi

LAST=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
# CLAUDE.md is a file name, not a credit; anything else naming Claude fails.
if git log --format='%an %ae%n%B' ${LAST:+$LAST..}HEAD | sed 's/CLAUDE\.md//g' | grep -qiE 'claude|anthropic|co-authored-by'; then
  bad "a commit since ${LAST:-the root} names Claude/Anthropic or has a Co-Authored-By"
else
  ok "no attribution in commits since ${LAST:-the root}"
fi

# ------------------------------------------------------------------ registries
for c in paginaverto-ocr paginaverto; do
  curl -sf -A "paginaverto-release (github.com/veripublica/paginaverto)" \
    "https://crates.io/api/v1/crates/$c/$V" >/dev/null \
    && bad "crates.io already has $c $V" || ok "crates.io does not have $c $V"
done
curl -sf "https://registry.npmjs.org/@veripublica%2fpaginaverto-wasm/$V" >/dev/null \
  && bad "npm already has $V" || ok "npm does not have $V"

# ------------------------------------------------------------------ CI gates
check "cargo fmt --all --check" cargo fmt --all --check
check "clippy, default (-D warnings)" cargo clippy --workspace --all-targets --locked -- -D warnings
check "clippy, ocr (-D warnings)" cargo clippy --workspace --all-targets --locked --features paginaverto/ocr -- -D warnings
check "clippy, cli (-D warnings)" cargo clippy --workspace --all-targets --locked --features paginaverto/cli -- -D warnings
check "clippy, paginaverto-wasm for wasm32" cargo clippy -p paginaverto-wasm --target wasm32-unknown-unknown --locked -- -D warnings
for feat in "" "paginaverto/ocr" "paginaverto/cli"; do
  label="cargo test${feat:+ --features $feat}"
  if out=$(cargo test --workspace --locked ${feat:+--features "$feat"} 2>&1); then
    sum=$(grep '^test result:' <<< "$out" | awk '{p+=$4; f+=$6; i+=$8} END {print p" passed, "f" failed, "i" ignored"}')
    ok "$label ($sum)"
  else
    bad "$label"; grep -E '^test .* FAILED|^failures:|panicked' <<< "$out" | head -15 | sed 's/^/        /'
  fi
done
check "the OCR accuracy tool compiles" cargo check --manifest-path tools/ocr-accuracy/Cargo.toml --locked
check "cargo doc (-D warnings)" env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --features paginaverto/ocr
if cargo tree -p paginaverto -p paginaverto-wasm -e normal --locked | grep -qE '(ocrs|rten) v'; then
  bad "the default build pulls ocrs/rten"
else
  ok "the default build pulls no ocrs/rten"
fi
if command -v wasm-pack >/dev/null; then
  check "wasm-pack build (the web page's package)" \
    wasm-pack --quiet build crates/paginaverto-wasm --target web --no-pack --out-dir "${TMPDIR:-/tmp}/paginaverto-preflight-pkg"
  rm -rf "${TMPDIR:-/tmp}/paginaverto-preflight-pkg"
else
  skip "wasm-pack not installed (pages.yml builds it)"
fi
# What crates.io will receive, built from the packaged files alone (both
# crates, in order, against each other). Needs a clean tree.
if [ -n "$(git status --porcelain)" ]; then
  skip "cargo publish --dry-run: needs a clean tree (see above)"
else
  check "cargo publish --workspace --dry-run (package + verify)" cargo publish --workspace --dry-run --locked
  PKG=$(cargo package --workspace --list --locked 2>/dev/null)
  STRAY=$(grep -E '(^|/)CLAUDE\.md$|^scripts/|(^|/)[^/]* [0-9]+(\.[A-Za-z0-9]+)?$' <<< "$PKG" || true)
  [ -z "$STRAY" ] && ok "packages hold no local-only file or conflict copy" \
    || { bad "stray files would ship"; printf '%s\n' "$STRAY" | sed 's/^/        /'; }
fi
if command -v cargo-audit >/dev/null; then
  check "cargo audit (advisory database fetched now)" cargo audit --file Cargo.lock
else
  bad "cargo audit is not installed: cargo install cargo-audit --locked"
fi
MSRV=$(grep -m1 '^rust-version' Cargo.toml | cut -d'"' -f2)
if rustup run "$MSRV" rustc -V >/dev/null 2>&1; then
  check "compiles at the MSRV ($MSRV)" rustup run "$MSRV" cargo check --workspace --locked --features paginaverto/cli
else
  skip "MSRV $MSRV not installed here (CI gates it)"
fi

echo "  note: sweep README.md for stale numbers; the [$V] section is what the release page shows"
echo
echo "$PASS passed, $FAIL failed, in $((SECONDS / 60)) min $((SECONDS % 60)) s"
if [ "$FAIL" -gt 0 ]; then
  echo "NOT READY:"; for f in "${FAILED[@]}"; do echo "  - $f"; done; exit 1
fi
echo "READY: ask Baris, then git tag -a v$V -m '…' && git push origin main v$V"
