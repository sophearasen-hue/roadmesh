# ios/ — Swift native modules

Placeholder until BUILD_PLAN Step 6 (transport) and Step 10 (CarPlay).

Will contain:
- CoreBluetooth transport for RSMs, connected to `core/` via FFI
- CarPlay Driving Task scene (list/grid template, SPEC §9)
- Secure Enclave key generation and signing (AUDIT S-02)

Open decision before Step 5: Flutter keeps its own iOS project in `app/ios/`.
This folder should become a Flutter plugin package (e.g. `plugins/roadmesh_ios`)
or hold Swift packages that `app/ios` links. Not changed without approval.
