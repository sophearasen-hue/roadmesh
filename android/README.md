# android/: RoadMesh Android radio (Flutter plugin `roadmesh_android`)

BUILD_PLAN Step 5. The phone's safety engine runs here, in a **foreground service** (type `connectedDevice` + `location`, targetSdk 36), so warnings keep working with the screen off. The Flutter app only shows what the service reports.

| File | What it does |
|---|---|
| `android/src/main/kotlin/com/roadmesh/android/DriveService.kt` | The drive session: GPS at up to 10 Hz, a 100 ms loop that runs the Rust engine (transmit → decide), publishes alerts and peers, and keeps the field log |
| `BleRadio.kt` | Bluetooth LE extended advertising (one message per advertisement, as service data under the RoadMesh UUID; Coded PHY when supported) and one long-running filtered scan. Phones without extended advertising are receive-only |
| `KeystoreKeys.kt` | Device key + 20 pseudonym keys a week in the Android Keystore (StrongBox where available), never exportable; signs for the core (AUDIT S-02) |
| `Enrolment.kt` | Certificate request / import (developer CA files until the server exists, Step 9) |
| `FieldLog.kt` | Gate 1 log: delivery % per distance band, signal strength, battery. No coordinates, no pseudonyms |
| `Crypto.kt` | DER ↔ raw signatures, compressed public keys |
| `RoadmeshAndroidPlugin.kt`, `lib/roadmesh_android.dart` | Flutter method/event channels |
| `android/src/main/kotlin/com/roadmesh/core/` (generated) | UniFFI bindings to the Rust core (`ffi/`) |

## Build

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
```

```bash
cargo install cargo-ndk --locked --version 4.1.2
```

```bash
ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<version> scripts/build-android-core.sh
```

```bash
cd app && flutter build apk --debug -Proadmesh.devCaAnchor=<anchor from devca>
```

The last line uses Gradle's `-P` through Flutter: pass it as `flutter build apk --debug --android-project-arg roadmesh.devCaAnchor=<anchor>` if your Flutter version does not forward `-P`.

## Field test (Gate 1: ≥ 100 m at 90 % delivery, < 8 %/hour battery)

You need two Android phones (Android 8 or newer with Bluetooth LE extended advertising; most phones from 2019 on), a laptop with `adb`, and a straight road or open area at least 300 m long.

1. **Make a developer CA** on the laptop (once). Keep its folder out of git:
   ```bash
   cargo run -p devca -- init ~/.roadmesh-devca
   ```
   It prints the trust anchor (68 hex characters).
2. **Build and install** the debug app with that anchor on both phones:
   ```bash
   cd app && flutter run --debug --android-project-arg roadmesh.devCaAnchor=<anchor>
   ```
3. On each phone open **Radio test**, tap **Start drive** (allow Bluetooth, location and notifications), then **Create certificate request**.
4. Issue certificates for each phone:
   ```bash
   adb pull /sdcard/Android/data/com.roadmesh.app/files/roadmesh-request.bin req.bin
   ```
   ```bash
   cargo run -p devca -- issue ~/.roadmesh-devca req.bin certs.bin
   ```
   ```bash
   adb push certs.bin /sdcard/Android/data/com.roadmesh.app/files/roadmesh-certificates.bin
   ```
   Then tap **Import certificates**: the screen shows `Certificates: 20`.
5. **Done-when check:** stand next to each other. Each phone lists the other, with speed and distance.
6. **Range:** one person stays, the other walks or rides away slowly to 250 m and back, for at least 10 minutes. Phones in a pocket or on the dashboard, as in real use.
7. **Battery:** charge both to 100 %, run a drive for one hour with the screen off.
8. Tap **Export field log** and pull it:
   ```bash
   adb pull /sdcard/Android/data/com.roadmesh.app/files/roadmesh-field-log.csv
   ```
   Each row: minute, battery %, distance band, messages received / expected, delivery %, median signal strength. Send me the files; Gate 1 passes if the 100–150 m band is at least 90 % and the battery falls less than 8 % in the hour.

Also check with a Bluetooth sniffer (for example nRF Connect on a third phone) that the advertising address changes when the pseudonym changes (AUDIT S-13).
