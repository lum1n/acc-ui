#!/usr/bin/env bash
# Builds acc-ui-ffi for iOS (device + simulator) and macOS and packages it as
# a Swift package in bindings/swift: AccUIFFI.xcframework plus the generated
# bindings as the `AccUICore` library, which the AccUI renderers build on.
# Needs macOS and Xcode.
#
# Apps that already ship another Rust static library (Sessh links acc-ffi)
# should not use the xcframework: two Rust static libraries in one binary
# duplicate std. Depend on crates/acc-ui-ffi from that library and generate
# the bindings with scripts/swift-bindings.sh instead.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
[[ "$(uname)" == "Darwin" ]] || { echo "build-xcframework.sh needs macOS and Xcode" >&2; exit 1; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT="$ROOT/bindings/swift"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

"$ROOT/scripts/swift-bindings.sh" "$STAGE/gen" >/dev/null

TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios aarch64-apple-darwin x86_64-apple-darwin)
rustup target add "${TARGETS[@]}" 2>/dev/null || true
for t in "${TARGETS[@]}"; do
  IPHONEOS_DEPLOYMENT_TARGET=16.0 MACOSX_DEPLOYMENT_TARGET=13.0 \
    cargo build -q -p acc-ui-ffi --release --target "$t"
done

lib() { echo "$CARGO_TARGET_DIR/$1/release/libacc_ui_ffi.a"; }
mkdir -p "$STAGE/headers" "$STAGE/sim" "$STAGE/mac"
cp "$STAGE/gen/acc_ui_ffiFFI.h" "$STAGE/headers/"
cp "$STAGE/gen/acc_ui_ffiFFI.modulemap" "$STAGE/headers/module.modulemap"
lipo -create "$(lib aarch64-apple-ios-sim)" "$(lib x86_64-apple-ios)" -output "$STAGE/sim/libacc_ui_ffi.a"
lipo -create "$(lib aarch64-apple-darwin)" "$(lib x86_64-apple-darwin)" -output "$STAGE/mac/libacc_ui_ffi.a"

rm -rf "$OUT"
mkdir -p "$OUT/Sources/AccUICore"
xcodebuild -create-xcframework \
  -library "$(lib aarch64-apple-ios)" -headers "$STAGE/headers" \
  -library "$STAGE/sim/libacc_ui_ffi.a" -headers "$STAGE/headers" \
  -library "$STAGE/mac/libacc_ui_ffi.a" -headers "$STAGE/headers" \
  -output "$OUT/AccUIFFI.xcframework"
cp "$STAGE/gen/acc_ui_ffi.swift" "$OUT/Sources/AccUICore/"
cat >"$OUT/Package.swift" <<'PKG'
// swift-tools-version:5.9
import PackageDescription

let package = Package(
  name: "AccUICore",
  platforms: [.iOS(.v16), .macOS(.v13)],
  products: [.library(name: "AccUICore", targets: ["AccUICore"])],
  targets: [
    .binaryTarget(name: "AccUIFFI", path: "AccUIFFI.xcframework"),
    .target(name: "AccUICore", dependencies: ["AccUIFFI"], path: "Sources/AccUICore"),
  ]
)
PKG
echo "Swift package ready in bindings/swift"
