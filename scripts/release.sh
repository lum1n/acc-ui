#!/usr/bin/env bash
# Release helpers. The release workflow (.github/workflows/release.yml) runs
# these; they work by hand too.
#
#   scripts/release.sh swift-sources [--check]
#       Regenerates Sources/AccUICore/acc_ui_ffi.swift, the Swift wrapper the
#       root Package.swift ships. --check fails if the checked-in copy is stale.
#   scripts/release.sh package VERSION CHECKSUM
#       Points Package.swift's AccUIFFI binary target at the release asset of
#       VERSION (e.g. v0.1.0) with the given SwiftPM checksum.
#   scripts/release.sh notes VERSION
#       Prints VERSION's section of CHANGELOG.md (without its heading).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO_URL="https://github.com/lum1n/acc-ui"
SWIFT_SOURCES="$ROOT/Sources/AccUICore/acc_ui_ffi.swift"

usage() {
  sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

cmd="${1:-}"
[[ -n "$cmd" ]] || usage
shift

case "$cmd" in
swift-sources)
  gen="$(mktemp -d)"
  trap 'rm -rf "$gen"' EXIT
  "$ROOT/scripts/swift-bindings.sh" "$gen" >/dev/null
  if [[ "${1:-}" == "--check" ]]; then
    if ! diff -u "$SWIFT_SOURCES" "$gen/acc_ui_ffi.swift"; then
      echo "Sources/AccUICore/acc_ui_ffi.swift is stale: run scripts/release.sh swift-sources" >&2
      exit 1
    fi
  else
    mkdir -p "$(dirname "$SWIFT_SOURCES")"
    cp "$gen/acc_ui_ffi.swift" "$SWIFT_SOURCES"
  fi
  ;;
package)
  version="${1:?usage: release.sh package VERSION CHECKSUM}"
  checksum="${2:?usage: release.sh package VERSION CHECKSUM}"
  [[ "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "bad version: $version" >&2; exit 1; }
  [[ "$checksum" =~ ^[0-9a-f]{64}$ ]] || { echo "bad checksum: $checksum" >&2; exit 1; }
  url="$REPO_URL/releases/download/$version/AccUIFFI.xcframework.zip"
  # Rewrite the two lines inside the binary target; keep everything else.
  tmp="$(mktemp)"
  awk -v url="$url" -v sum="$checksum" '
    /^[[:space:]]*url: "/ { sub(/"[^"]*"/, "\"" url "\""); n++ }
    /^[[:space:]]*checksum: "/ { sub(/"[^"]*"/, "\"" sum "\""); n++ }
    { print }
    END { if (n != 2) exit 1 }
  ' "$ROOT/Package.swift" >"$tmp" || { rm -f "$tmp"; echo "Package.swift: url/checksum lines not found" >&2; exit 1; }
  mv "$tmp" "$ROOT/Package.swift"
  ;;
notes)
  version="${1:?usage: release.sh notes VERSION}"
  version="${version#v}"
  # From "## [VERSION]" up to the next "## [" heading or the link references.
  notes="$(awk -v v="$version" '
    index($0, "## [" v "]") == 1 { on = 1; next }
    on && (/^## \[/ || /^\[[^]]+\]: /) { exit }
    on { print }
  ' "$ROOT/CHANGELOG.md" | sed -e '/./,$!d')"
  [[ -n "$notes" ]] || { echo "CHANGELOG.md has no section for $version" >&2; exit 1; }
  printf '%s\n' "$notes"
  ;;
*)
  usage
  ;;
esac
