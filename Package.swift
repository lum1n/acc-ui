// swift-tools-version:5.9
// The acc-ui Swift package: AccUIFFI, the prebuilt xcframework from the
// GitHub release, and AccUICore, the generated UniFFI wrapper over it
// (Sources/AccUICore, kept in sync by CI with scripts/release.sh swift-sources).
//
// The release workflow rewrites the url and checksum below for each v* tag
// (scripts/release.sh package VERSION CHECKSUM) and moves the tag onto that
// commit, so a tagged Package.swift always points at its own release.
import PackageDescription

let package = Package(
  name: "AccUI",
  platforms: [.iOS(.v16), .macOS(.v13)],
  products: [.library(name: "AccUICore", targets: ["AccUICore"])],
  targets: [
    .binaryTarget(
      name: "AccUIFFI",
      url: "https://github.com/lum1n/acc-ui/releases/download/v0.1.0/AccUIFFI.xcframework.zip",
      checksum: "0000000000000000000000000000000000000000000000000000000000000000"
    ),
    .target(name: "AccUICore", dependencies: ["AccUIFFI"], path: "Sources/AccUICore"),
  ]
)
