#!/usr/bin/env bash
# Runs the MCP server tests under the shells and awks hosts actually have:
# the local sh, dash + mawk (Debian), and busybox ash + awk. Needs Docker
# for the last two.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo test -q -p acc-ui-agent --test server
for image in debian:bookworm-slim busybox:latest; do
  echo "== $image"
  # Each test starts a few containers; run them one at a time.
  ACC_UI_TEST_SH="$ROOT/scripts/sh-in-container.sh $image" \
    cargo test -q -p acc-ui-agent --test server -- --test-threads=1
done
