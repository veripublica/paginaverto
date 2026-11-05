#!/usr/bin/env bash
#
# Waits for every workflow run on one commit, then prints them and exits 0
# only if all succeeded. Run it in the background after the push.
#
# Usage: scripts/wait-runs.sh <tag-or-commit> [expected-runs] [timeout-seconds]
#
# A release push sends main and the tag together, so one commit carries six
# runs: CI and Pages (main), and Release, Release binaries, crates.io and npm
# (the tag). A plain push to main expects 2.

set -euo pipefail
# ^{commit}: our tags are annotated, and a bare rev-parse of one gives the tag
# object, which no run is filed under (0.1.3's wait saw 0 runs and timed out).
SHA=$(git rev-parse "${1:?usage: scripts/wait-runs.sh <tag-or-commit> [expected-runs] [timeout-seconds]}^{commit}")
N="${2:-6}"
LIMIT="${3:-3000}"
REPO=veripublica/paginaverto

runs() { gh run list --repo "$REPO" --commit "$SHA" --limit 20 --json name,status,conclusion,databaseId; }

START=$SECONDS
while :; do
  J=$(runs)
  have=$(jq length <<< "$J")
  open=$(jq '[.[] | select(.status != "completed")] | length' <<< "$J")
  [ "$have" -ge "$N" ] && [ "$open" = 0 ] && break
  if [ $((SECONDS - START)) -ge "$LIMIT" ]; then
    echo "timed out after ${LIMIT}s: $have of $N runs seen, $open still running"
    jq -r '.[] | "  \(.status)\t\(.conclusion)\t\(.name)\t\(.databaseId)"' <<< "$J"
    exit 2
  fi
  sleep 15
done

echo "$have runs on ${SHA:0:7}, $((SECONDS - START))s waited:"
jq -r '.[] | "  \(.conclusion)\t\(.name)\t\(.databaseId)"' <<< "$J"
[ "$(jq '[.[] | select(.conclusion != "success")] | length' <<< "$J")" = 0 ]
