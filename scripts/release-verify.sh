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
  grep -qF "tag = \"v$V\"" <<< "$BODY" && ok "release notes give the Cargo.toml line" || bad "release notes lack the Cargo.toml line"
else
  bad "no release v$V"
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
