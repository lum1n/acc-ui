#!/usr/bin/env bash
# Generates acc-ui-ffi's Swift bindings (acc_ui_ffi.swift, acc_ui_ffiFFI.h,
# acc_ui_ffiFFI.modulemap) into OUT_DIR from a host build. Runs anywhere
# cargo does. Apps that link acc-ui-ffi into their own Rust static library
# (Sessh) take the bindings from here; scripts/build-xcframework.sh uses it
# for the standalone Swift package.
#
#   scripts/swift-bindings.sh OUT_DIR
#
# Honors CARGO_TARGET_DIR. Prints the path of the built host library.
set -euo pipefail
OUT="${1:?usage: swift-bindings.sh OUT_DIR}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
cd "$ROOT"
cargo build -q -p acc-ui-ffi --release
LIB="$TARGET/release/libacc_ui_ffi.so"
[[ -f "$TARGET/release/libacc_ui_ffi.dylib" ]] && LIB="$TARGET/release/libacc_ui_ffi.dylib"
mkdir -p "$OUT"
cargo run -q -p acc-ui-ffi --features bindgen --bin uniffi-bindgen -- \
  generate --library "$LIB" --language swift --out-dir "$OUT" >&2
echo "$LIB"
