#!/usr/bin/env bash
#
# Step 1 of a release: the workspace version (shared by all three crates), the
# exact pin of paginaverto-ocr, Cargo.lock, and the CHANGELOG heading the
# release notes are taken from. Same job as kepubverto's and epubsana's.
#
# Usage: scripts/bump.sh 0.2.0 [YYYY-MM-DD]   (date defaults to today)
#
# Commits nothing. Read the diff afterwards; sweep README.md for stale numbers.

set -euo pipefail
cd "$(dirname "$0")/.."

V="${1:?usage: scripts/bump.sh X.Y.Z [YYYY-MM-DD]}"
D="${2:-$(date +%F)}"
[[ "$V" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not a version: $V" >&2; exit 2; }

grep -q '^## \[Unreleased\]' CHANGELOG.md \
  || { echo "CHANGELOG.md has no [Unreleased] section to release" >&2; exit 1; }
! grep -q "^## \[$V\]" CHANGELOG.md \
  || { echo "CHANGELOG.md already has a [$V] section" >&2; exit 1; }
# An empty [Unreleased] would become empty release notes.
awk '/^## \[Unreleased\]/{f=1;next} f&&/^## \[/{exit} f&&NF{n++} END{exit n?0:1}' CHANGELOG.md \
  || { echo "CHANGELOG.md's [Unreleased] section is empty" >&2; exit 1; }

# [workspace.package] version, and the =X.Y.Z pin of paginaverto-ocr.
awk -v v="$V" '!done && /^version = "/ { print "version = \"" v "\""; done=1; next } { print }' \
  Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
sed -i '' -E "s/^(paginaverto-ocr = \{.*version = \")=[0-9.]+(\".*)$/\1=$V\2/" Cargo.toml
# The new section goes under a fresh, empty [Unreleased].
sed -i '' "s/^## \[Unreleased\]/## [Unreleased]\\
\\
## [$V] - $D/" CHANGELOG.md

# Updates only the three paginaverto entries of Cargo.lock; never a plain
# `cargo update` (the lock matches epublift's).
cargo check --workspace -q
# The accuracy tool has its own lock, out of the workspace, and CI checks it
# with --locked. The same: only the version lines of our own packages move.
cargo check --manifest-path tools/ocr-accuracy/Cargo.toml -q

grep -n -E '^version = |^paginaverto-ocr = ' Cargo.toml
grep -n "^## \[" CHANGELOG.md | head -3
git diff --stat
