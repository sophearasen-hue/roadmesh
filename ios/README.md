# ios/: RoadMesh iOS radio (Flutter plugin `roadmesh_ios`)

BUILD_PLAN Step 6. Same role as `android/`: the drive session runs the Rust safety engine next to the radio; the Flutter app only shows what it reports (shared Dart API: `app/lib/radio.dart`).

**iPhones cannot broadcast a whole message.** iOS lets apps advertise only service UUIDs (about 28 bytes), so an iPhone sends its frames as GATT notifications to phones that connect to it, cut to the link's packet size by the Rust core (`core/src/fragment.rs`). Decision of 2026-11-24, "option A"; sending over 4G is added in Step 9 as a fallback.

| | Android phone | iPhone |
|---|---|---|
| Sends | Extended advertisement (whole frame) | Advertises the RoadMesh UUID; pushes frames as notifications to subscribers |
| Hears Android | Scan | Scan (needs an iPhone that reports extended advertisements: to be confirmed in the field test) |
| Hears iPhones | Connects (up to 6) and subscribes | Connects (up to 6) and subscribes |

| File | What it does |
|---|---|
| `ios/roadmesh_ios/Sources/roadmesh_ios/DriveSession.swift` | Location (background updates), the 100 ms loop around the Rust engine, field log |
| `BleRadio.swift` | CoreBluetooth peripheral (advertise + notify) and central (scan, connect, subscribe) |
| `KeychainKeys.swift` | Device and pseudonym keys in the Secure Enclave, never exportable (AUDIT S-02) |
| `Enrolment.swift` | Certificate request / import with the developer CA |
| `RoadmeshIosPlugin.swift` | Flutter channels `roadmesh/radio`, `roadmesh/radio/events` |
| `PrivacyInfo.xcprivacy` | Privacy manifest (precise location for app functionality, not linked, no tracking) |
| `RoadMeshCore.swift`, `Frameworks/` (generated) | UniFFI bindings and the Rust core as an XCFramework |
| `ios/roadmesh_ios/Sources/RoadMeshKit/` | Pure Swift helpers (signature conversion, field log); unit-tested from `RoadMeshKit/` with `swift test` |

The plugin builds with **Swift Package Manager** (`flutter config --enable-swift-package-manager`).

## Build (Mac with Xcode)

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
```

```bash
scripts/build-ios-core.sh
```

```bash
flutter config --enable-swift-package-manager
```

```bash
cd app && flutter run --debug
```

## Field test: iPhone + Android (done when: they see each other)

You need one iPhone (iOS 15+), one Android phone set up as in `android/README.md`, a Mac with Xcode and your Apple developer account (free accounts work for 7-day installs), and the developer CA from `android/README.md` step 1.

1. Put the CA's trust anchor in `app/ios/Flutter/DevCa.xcconfig` (git-ignored):
   ```
   ROADMESH_DEV_CA_ANCHOR = <anchor>
   ```
2. Open `app/ios/Runner.xcworkspace`, choose your team under Signing, connect the iPhone, then run:
   ```bash
   cd app && flutter run --debug
   ```
3. In the app: **Radio test → Start drive** (allow location and Bluetooth), then **Create certificate request**.
4. In Xcode: Window → Devices and Simulators → the iPhone → RoadMesh → **Download Container**. Take `AppData/Documents/roadmesh-request.bin` and issue certificates:
   ```bash
   cargo run -p devca -- issue ~/.roadmesh-devca roadmesh-request.bin roadmesh-certificates.bin
   ```
   Put `roadmesh-certificates.bin` into `AppData/Documents/` of the downloaded container and use **Replace Container**. Back in the app tap **Import certificates** (`Certificates: 20`).
5. Keep **both apps open on screen** (iOS hides background advertisements from Android). Each phone should list the other with speed and distance. Note which directions work:
   - Android hears iPhone (link).
   - iPhone hears Android (extended advertisements).
6. Repeat the range walk from `android/README.md` and export the field log on both phones (on the iPhone it is in `Documents/roadmesh-field-log.csv`).

Known limits to check: iPhones in the background advertise only to other iPhones; the address rotation is iOS's own (check with a sniffer); whether this iPhone model reports Android's extended advertisements.
