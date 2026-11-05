#!/usr/bin/env bash
#
# After the release push: verify what was published, not the workflows.
# Read-only; works in a temporary directory and changes nothing anywhere.
#
# Usage: scripts/release-verify.sh 0.2.0
#
# One line per item (ok / FAIL) and a verdict; exit 0 only if all pass.

set -uo pipefail
cd "$(dirname "$0")/.."

V="${1:?usage: scripts/release-verify.sh X.Y.Z}"
REPO=veripublica/paginaverto
SITE=https://veripublica.github.io/paginaverto
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
PASS=0; FAIL=0
ok()  { echo "  ok    $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL  $1"; FAIL=$((FAIL + 1)); }

# ------------------------------------------------------------------ tag
T=$(git ls-remote --tags origin "refs/tags/v$V^{}" | cut -f1)
[ -n "$T" ] && ok "tag v$V on origin (${T:0:7})" || bad "tag v$V not on origin"

# ------------------------------------------------------------------ release page
if R=$(gh release view "v$V" --repo "$REPO" --json isDraft,isPrerelease,body,name 2>/dev/null); then
  ok "release v$V exists"
  [ "$(jq -r .isDraft <<< "$R")" = false ] && ok "release is published, not a draft" || bad "release is a draft"
  # The body must be this version's CHANGELOG section: its first line, and the footer.
  FIRST=$(awk -v v="$V" '$0 ~ "^## \\[" v "\\]"{f=1;next} f&&/^## \[/{exit} f&&NF{print;exit}' CHANGELOG.md)
  BODY=$(jq -r .body <<< "$R")
  [ -n "$FIRST" ] && grep -qF -- "$FIRST" <<< "$BODY" \
    && ok "release notes are CHANGELOG's [$V] section" || bad "release notes do not match CHANGELOG's [$V] section"
  # Footer: from 0.1.2 the registries; before that, the git-tag line.
  if [ "$(printf '%s\n' "$V" 0.1.2 | sort -V | head -1)" = 0.1.2 ]; then
    grep -qF "crates.io/crates/paginaverto/$V" <<< "$BODY" && ok "release notes link crates.io and npm" || bad "release notes lack the registry links"
  else
    grep -qF "tag = \"v$V\"" <<< "$BODY" && ok "release notes give the Cargo.toml line" || bad "release notes lack the Cargo.toml line"
  fi
else
  bad "no release v$V"
fi

# ------------------------------------------------------------------ binaries
# RELEASING §5: 8 archives, no version in the names, checksums over all of them,
# a folder of the same name holding the binary and the licences. From 0.2.0 on.
if [ "$(printf '%s\n' "$V" 0.2.0 | sort -V | head -1)" = 0.2.0 ]; then
  ASSETS=$(gh release view "v$V" --repo "$REPO" --json assets --jq '.assets[].name' 2>/dev/null || true)
  for T in x86_64-unknown-linux-gnu x86_64-unknown-linux-musl aarch64-unknown-linux-gnu \
           aarch64-unknown-linux-musl x86_64-apple-darwin aarch64-apple-darwin; do
    grep -qxF "paginaverto-$T.tar.gz" <<< "$ASSETS" && ok "binary $T" || bad "no binary for $T"
  done
  for T in x86_64-pc-windows-msvc aarch64-pc-windows-msvc; do
    grep -qxF "paginaverto-$T.zip" <<< "$ASSETS" && ok "binary $T" || bad "no binary for $T"
  done
  grep -qxF SHA256SUMS.txt <<< "$ASSETS" && ok "SHA256SUMS.txt attached" || bad "no SHA256SUMS.txt"
  D=$(mktemp -d)
  if gh release download "v$V" --repo "$REPO" --dir "$D" \
       --pattern SHA256SUMS.txt --pattern 'paginaverto-aarch64-apple-darwin.tar.gz' 2>/dev/null; then
    [ "$(wc -l < "$D/SHA256SUMS.txt" | tr -d ' ')" = 8 ] && ok "checksums cover 8 archives" || bad "SHA256SUMS.txt is not 8 lines"
    (cd "$D" && grep ' paginaverto-aarch64-apple-darwin.tar.gz$' SHA256SUMS.txt | shasum -a 256 -c - >/dev/null 2>&1) \
      && ok "an archive matches its checksum" || bad "an archive does not match its checksum"
    L=$(tar tzf "$D/paginaverto-aarch64-apple-darwin.tar.gz")
    for F in paginaverto LICENSE LICENSE-COMMERCIAL.md LICENSING.md README.md; do
      grep -qxF "paginaverto-aarch64-apple-darwin/$F" <<< "$L" && ok "archive holds $F" || bad "archive lacks $F"
    done
    tar xzf "$D/paginaverto-aarch64-apple-darwin.tar.gz" -C "$D"
    [ "$("$D/paginaverto-aarch64-apple-darwin/paginaverto" -V 2>/dev/null)" = "paginaverto $V" ] \
      && ok "the binary says its version" || bad "the binary does not say paginaverto $V"
  else
    bad "could not download the checksums and an archive"
  fi
  rm -rf "$D"
fi

REG=no; [ "$(printf '%s\n' "$V" 0.1.2 | sort -V | head -1)" = 0.1.2 ] && REG=yes
# ------------------------------------------------------------------ crates.io
[ "$REG" = yes ] && for c in paginaverto-ocr paginaverto; do
  if curl -sfL -A "paginaverto-release (github.com/veripublica/paginaverto)" \
       -o "$W/$c.crate" "https://static.crates.io/crates/$c/$c-$V.crate"; then
    ok "crates.io serves $c $V"
    L=$(tar tzf "$W/$c.crate")
    for f in LICENSE LICENSE-COMMERCIAL.md README.md; do
      grep -qx "$c-$V/$f" <<< "$L" && ok "$c crate has $f" || bad "$c crate lacks $f"
    done
    LEAK=$(grep -E "/CLAUDE\.md$|/scripts/" <<< "$L" || true)
    [ -z "$LEAK" ] && ok "$c crate has no local-only file" || bad "$c crate ships: $LEAK"
  else
    bad "crates.io does not serve $c $V"
  fi
done

# ------------------------------------------------------------------ npm
if [ "$REG" = yes ]; then
TGZ="https://registry.npmjs.org/@veripublica/paginaverto-wasm/-/paginaverto-wasm-$V.tgz"
for _ in $(seq 1 "${NPM_POLLS:-40}"); do curl -sfL -o "$W/n.tgz" "$TGZ" && break; sleep 15; done
if [ -s "$W/n.tgz" ]; then
  ok "npm serves @veripublica/paginaverto-wasm $V"
  L=$(tar tzf "$W/n.tgz")
  for f in package/LICENSE package/LICENSE-COMMERCIAL.md package/paginaverto_wasm.d.ts package/paginaverto_wasm_bg.wasm; do
    grep -qx "$f" <<< "$L" && ok "npm tarball has ${f#package/}" || bad "npm tarball lacks ${f#package/}"
  done
  LATEST=$(curl -sf "https://registry.npmjs.org/@veripublica%2fpaginaverto-wasm" | jq -r '."dist-tags".latest')
  [ "$LATEST" = "$V" ] && ok "npm latest is $V" || bad "npm latest is $LATEST, not $V"
else
  bad "npm tarball not served after $(( ${NPM_POLLS:-40} * 15 ))s: $TGZ"
fi
fi

# ------------------------------------------------------------------ the tag as a dependency
mkdir -p "$W/dep/src" && echo 'fn main() {}' > "$W/dep/src/main.rs"
cat > "$W/dep/Cargo.toml" <<TOML
[package]
name = "verify-dep"
version = "0.0.0"
edition = "2024"
[dependencies]
paginaverto = { git = "https://github.com/$REPO", tag = "v$V" }
[workspace]
TOML
if (cd "$W/dep" && cargo generate-lockfile -q 2>/dev/null) \
   && grep -q "^version = \"$V\"" <(awk '/^name = "paginaverto"$/{getline; print}' "$W/dep/Cargo.lock"); then
  ok "the tag resolves as a git dependency, at paginaverto $V"
else
  bad "the tag does not resolve as a git dependency at $V"
fi

# ------------------------------------------------------------------ web page
for p in "" demo/ pkg/paginaverto_wasm_bg.wasm worker.js; do
  c=$(curl -s -o /dev/null -w '%{http_code}' "$SITE/$p")
  [ "$c" = 200 ] && ok "web page serves /$p" || bad "web page /$p answers $c"
done

echo
echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ] && echo "VERIFIED: v$V" || { echo "NOT VERIFIED: v$V"; exit 1; }
