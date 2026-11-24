# ios/ — Swift native modules

Placeholder until BUILD_PLAN Step 6 (transport) and Step 10 (CarPlay).

Will contain:
- CoreBluetooth transport for RSMs, connected to `core/` via FFI
- CarPlay Driving Task scene (list/grid template, SPEC §9)
- Secure Enclave key generation and signing (AUDIT S-02)

Decided in Step 5: this folder becomes the Flutter plugin package `roadmesh_ios`
(like `android/` = `roadmesh_android`), with the Rust core linked through the
UniFFI Swift bindings from `ffi/` (module `RoadMeshCore`). Flutter keeps its own
iOS project in `app/ios/`.
