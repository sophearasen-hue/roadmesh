#!/usr/bin/env bash
# Build the Rust safety core for iOS as an XCFramework and generate its Swift bindings.
# macOS with Xcode only. Needs the iOS Rust targets:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
set -euo pipefail
cd "$(dirname "$0")/.."
PLUGIN=ios/ios/roadmesh_ios
OUT=$PLUGIN/Frameworks/RoadMeshCoreFFI.xcframework
for t in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  cargo build --release -p roadmesh-ffi --target "$t"
done
GEN=target/uniffi-swift
rm -rf "$GEN" && mkdir -p "$GEN/headers"
cargo run -q -p uniffi-bindgen -- generate --library target/aarch64-apple-ios/release/libroadmesh_ffi.a \
  --language swift --no-format --out-dir "$GEN"
cp "$GEN/RoadMeshCoreFFI.h" "$GEN/headers/"
cp "$GEN/RoadMeshCoreFFI.modulemap" "$GEN/headers/module.modulemap"
mkdir -p target/ios-sim-universal
lipo -create target/aarch64-apple-ios-sim/release/libroadmesh_ffi.a target/x86_64-apple-ios/release/libroadmesh_ffi.a \
  -output target/ios-sim-universal/libroadmesh_ffi.a
rm -rf "$OUT"
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libroadmesh_ffi.a -headers "$GEN/headers" \
  -library target/ios-sim-universal/libroadmesh_ffi.a -headers "$GEN/headers" \
  -output "$OUT"
cp "$GEN/RoadMeshCore.swift" "$PLUGIN/Sources/roadmesh_ios/RoadMeshCore.swift"
echo "built $OUT and Swift bindings"
