#!/usr/bin/env bash
# Cross-compile the Rust safety core for Android and generate its Kotlin bindings.
# Needs: ANDROID_NDK_HOME (or ANDROID_HOME with an NDK), cargo-ndk, the Android Rust targets.
#   rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
#   cargo install cargo-ndk --locked --version 4.1.2
set -euo pipefail
cd "$(dirname "$0")/.."
PLUGIN=android/android/src/main
PROFILE=${PROFILE:-release}
FLAG=$([ "$PROFILE" = release ] && echo --release || echo "")
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -P 26 -o "$PLUGIN/jniLibs" build $FLAG -p roadmesh-ffi
# bindings from the host build of the same crate (library mode reads the UniFFI metadata)
cargo build -p roadmesh-ffi $FLAG
LIB=target/$PROFILE/libroadmesh_ffi.so
rm -rf "$PLUGIN/kotlin/com/roadmesh/core"
cargo run -q -p uniffi-bindgen -- generate --library "$LIB" --language kotlin \
  --no-format --out-dir "$PLUGIN/kotlin"
echo "core libraries: $(ls "$PLUGIN"/jniLibs/*/libroadmesh_ffi.so | wc -l) ABIs; bindings: $PLUGIN/kotlin/com/roadmesh/core"
