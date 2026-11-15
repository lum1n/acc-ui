#!/usr/bin/env bash
# Generates acc-ui-ffi's Kotlin bindings into OUT_DIR from a host build.
#
#   scripts/kotlin-bindings.sh OUT_DIR
set -euo pipefail
OUT="${1:?usage: kotlin-bindings.sh OUT_DIR}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
cd "$ROOT"
cargo build -q -p acc-ui-ffi --release
LIB="$TARGET/release/libacc_ui_ffi.so"
[[ -f "$TARGET/release/libacc_ui_ffi.dylib" ]] && LIB="$TARGET/release/libacc_ui_ffi.dylib"
mkdir -p "$OUT"
cargo run -q -p acc-ui-ffi --features bindgen --bin uniffi-bindgen -- \
  generate --library "$LIB" --language kotlin --out-dir "$OUT" >&2
