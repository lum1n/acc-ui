#!/usr/bin/env bash
# Builds the generated bindings with a small program that replays a recorded
# session through SessionView, and runs it. Linux or macOS.
#
#   scripts/ffi-smoke.sh swift            # needs swiftc
#   scripts/ffi-smoke.sh kotlin           # needs kotlinc and java; JNA_JAR=path/to/jna.jar
#
# Honors CARGO_TARGET_DIR. KOTLINC overrides the kotlinc binary.
set -euo pipefail
LANG_="${1:?usage: ffi-smoke.sh swift|kotlin}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
FIXTURE="$ROOT/crates/acc-ui/tests/sessions/claude-fix-and-review.ndjson"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

case "$LANG_" in
swift)
  "$ROOT/scripts/swift-bindings.sh" "$WORK/gen" >/dev/null
  cp "$WORK/gen/acc_ui_ffiFFI.modulemap" "$WORK/gen/module.modulemap"
  swiftc -I "$WORK/gen" -L "$TARGET/release" -lacc_ui_ffi \
    "$WORK/gen/acc_ui_ffi.swift" "$ROOT/crates/acc-ui-ffi/tests/swift/main.swift" \
    -o "$WORK/smoke"
  LD_LIBRARY_PATH="$TARGET/release" DYLD_LIBRARY_PATH="$TARGET/release" \
    "$WORK/smoke" "$FIXTURE"
  ;;
kotlin)
  : "${JNA_JAR:?set JNA_JAR to the JNA jar}"
  "$ROOT/scripts/kotlin-bindings.sh" "$WORK/gen"
  "${KOTLINC:-kotlinc}" -nowarn -include-runtime -cp "$JNA_JAR" -d "$WORK/smoke.jar" \
    "$WORK/gen/io/github/lum1n/accui/acc_ui_ffi.kt" "$ROOT/crates/acc-ui-ffi/tests/kotlin/Main.kt"
  java --enable-native-access=ALL-UNNAMED -Djna.library.path="$TARGET/release" \
    -cp "$WORK/smoke.jar:$JNA_JAR" MainKt "$FIXTURE"
  ;;
*)
  echo "usage: ffi-smoke.sh swift|kotlin" >&2
  exit 2
  ;;
esac
