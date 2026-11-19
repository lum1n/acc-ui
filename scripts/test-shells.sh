#!/usr/bin/env bash
# Runs the MCP server tests under the shells and awks hosts actually have:
# the local sh, dash + mawk (Debian), busybox ash + awk, and a stand-in for
# macOS (bash 3.2 as sh + the one true awk, scripts/macos-sh.Dockerfile).
# Needs Docker for all but the first.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
echo "== local sh"
cargo test -q -p acc-ui-agent --test server
docker build -q -t acc-ui-macos-sh -f scripts/macos-sh.Dockerfile scripts >/dev/null
for image in debian:bookworm-slim busybox:latest acc-ui-macos-sh; do
  echo "== $image"
  # Each test starts a few containers; run them one at a time.
  ACC_UI_TEST_SH="$ROOT/scripts/sh-in-container.sh $image" \
    cargo test -q -p acc-ui-agent --test server -- --test-threads=1
done
