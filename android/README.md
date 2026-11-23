# android/ — Kotlin native modules

Placeholder until BUILD_PLAN Step 5.

Will contain:
- Bluetooth LE extended advertising + scanning transport, connected to `core/` via FFI
- Foreground service of type `connectedDevice` (and `location`), targetSdkVersion 36+
- Android Keystore / StrongBox key generation and signing (AUDIT S-02)
- Random resolvable BLE addresses rotated together with the pseudonym (AUDIT S-13)

Open decision before Step 5: Flutter keeps its own Android project in `app/android/`.
This folder should become a Flutter plugin package (e.g. `plugins/roadmesh_android`)
or a Gradle module that `app/android` includes. Not changed without approval.
