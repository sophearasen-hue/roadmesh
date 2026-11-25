# Store submission pack (BUILD_PLAN Step 14, AUDIT section C)

Everything the App Store and Google Play ask for, in one place. Privacy answers: `docs/DATA_SAFETY.md`. Privacy policy and support pages: `docs/legal/`. Screenshots: `docs/store/screenshots/`. The Khmer texts are machine-assisted drafts, and a native speaker must review them before upload (AUDIT P-44).

## Listing

| Field | English | ខ្មែរ |
|---|---|---|
| Name | RoadMesh | RoadMesh |
| Subtitle (iOS, 30 chars) / short description (Play, 80 chars) | Warnings from nearby vehicles | ការព្រមានពីយានយន្តនៅជិត |
| Category | Navigation (iOS) / Auto & Vehicles (Play) | |
| Age rating | 4+ / Everyone | |

**Description (English).**

> RoadMesh warns you about vehicles you may not see: a motorbike crossing from behind a building, a car braking hard two vehicles ahead, stopped traffic around the bend.
>
> Phones with RoadMesh talk to each other directly over Bluetooth, about ten times a second, and pass warnings on to phones further away. No internet is needed for warnings. Each phone predicts the next few seconds and tells you clearly: "Motorbike crossing from your right in 3 seconds — brake gently".
>
> • Works for cars, motorbikes and tuk-tuks
> • English and Khmer
> • CarPlay: the current warning, your speed and the nearest vehicles
> • Private by design: no account, no names or phone numbers; your phone sends a random ID that changes every few minutes
>
> RoadMesh is driver assistance, not a guarantee. It cannot see vehicles without the app, and it never brakes or steers for you. Keep your eyes on the road and follow the traffic law.

**Description (ខ្មែរ, needs native review).**

> RoadMesh ព្រមានអ្នកអំពីយានដែលអ្នកប្រហែលជាមើលមិនឃើញ៖ ម៉ូតូដែលកំពុងឆ្លងកាត់ពីក្រោយអគារ រថយន្តដែលហ្វ្រាំងខ្លាំងនៅខាងមុខ ឬចរាចរណ៍ដែលឈប់នៅកាច់ផ្លូវ។
>
> ទូរស័ព្ទដែលមាន RoadMesh និយាយគ្នាដោយផ្ទាល់តាមប៊្លូធូស ប្រហែល ១០ ដងក្នុងមួយវិនាទី ហើយបញ្ជូនការព្រមានបន្តទៅទូរស័ព្ទដែលនៅឆ្ងាយជាងនេះ។ ការព្រមានមិនត្រូវការអ៊ីនធឺណិតទេ។
>
> • សម្រាប់រថយន្ត ម៉ូតូ និងរ៉ឺម៉ក
> • ភាសាអង់គ្លេស និងខ្មែរ
> • CarPlay៖ ការព្រមានបច្ចុប្បន្ន ល្បឿនរបស់អ្នក និងយាននៅជិតបំផុត
> • ឯកជនភាព៖ គ្មានគណនី គ្មានឈ្មោះ ឬលេខទូរស័ព្ទ
>
> RoadMesh ជាជំនួយដល់អ្នកបើកបរ មិនមែនជាការធានាទេ។ វាមិនដែលហ្វ្រាំង ឬបង្វិលចង្កូតជំនួសអ្នកឡើយ។

**Keywords (iOS).** road safety, collision warning, V2V, motorbike, Cambodia, Khmer, CarPlay, driver assistance

**URLs.**
- **Privacy policy and support:** publish `docs/legal/*.md` as web pages on your domain, and enter the URLs in both stores (L-09, L-17).
- **Support contact:** a monitored email address.

## Apple App Review

**App Review notes (paste into App Store Connect).**

> RoadMesh exchanges safety messages with other phones running the app over Bluetooth LE. A single test device sees no other vehicles, so the Drive screen shows "All clear" and an empty map.
>
> A demo video showing two phones warning each other is attached. On first launch, choose a language and vehicle type, then accept the safety notice. Tap "Start driving" to begin.
>
> The app never controls the vehicle. While moving faster than 10 km/h, only the glanceable Drive screen is available. CarPlay shows a Driving Task list template.

**Background modes (L-04).**

| Mode | Why it is needed |
|---|---|
| `bluetooth-central` | Hear nearby vehicles' safety messages while the screen is off during a drive. |
| `bluetooth-peripheral` | Send this vehicle's safety messages to nearby phones while the screen is off. |
| `location` | The own position in each safety message, ten times a second, while a drive runs. The user starts and stops drives, and the blue location indicator shows while active. |

**Purpose strings (L-03).** `NSBluetoothAlwaysUsageDescription`, `NSLocationWhenInUseUsageDescription` and `NSLocationAlwaysAndWhenInUseUsageDescription`, in English and Khmer. `NSMotionUsageDescription` is not needed: the app does not use motion sensors.

**Entitlements.** CarPlay Driving Task (L-06, see `docs/CARPLAY.md`). App Attest, `production` (`docs/SERVER.md`).

**Driving safety (L-07).**
- **Onboarding:** a safety notice the driver must accept once: "driver assistance, not a guarantee; never brakes or steers".
- **While moving:** no screen except Drive is usable above 10 km/h, and there is no typing while moving.
- **Alerts:** a tone or chime, plus vibration.

## Google Play

**Target API (L-10).** `targetSdk 36`, `minSdk 26`.

**Foreground service declaration (L-11).** Play Console → App content → Foreground service permissions.

| Type | Task | User impact if deferred |
|---|---|---|
| `connectedDevice` | Exchange safety messages with nearby vehicles' phones over Bluetooth LE during a drive the user started | Collision warnings would stop when the screen turns off |
| `location` | Put the vehicle's position into each safety message during that drive | Same |

**Demo video script (30–60 s, unlisted YouTube link).**
1. Two phones side by side. Open RoadMesh, accept the notice, tap Start driving. The persistent notification "RoadMesh is running" appears.
2. Turn both screens off.
3. Carry one phone past the other (or play the simulator's crossing scenario on a laptop). The receiving phone sounds the alert with its screen off.
4. Tap Stop. The notification disappears.

**Location permission declaration (L-12).** Precise location in the foreground service.

> RoadMesh is a collision-warning app. While the user drives with a drive session started, the app broadcasts the vehicle's position, speed and heading to nearby phones about ten times a second, and predicts collisions with the vehicles they report. Without precise location no warning is possible. Location is used only during a drive the user started, shown by a persistent notification, and is sent only under a pseudonym that changes every few minutes.

`ACCESS_BACKGROUND_LOCATION` is not requested. Location runs in the foreground service the user started.

**Bluetooth (L-14).** `BLUETOOTH_SCAN` without `neverForLocation`: the app already holds fine location for its function. `BLUETOOTH_ADVERTISE` and `BLUETOOTH_CONNECT` are also declared.

**Testing track (L-15).** A new personal developer account must run a closed test (12 testers for 14 days) before applying for production. An organisation account avoids this. Plan for 2 to 3 weeks either way.

**Signing (L-16).** Play App Signing on; upload key in a password manager or HSM (S-23). Release is an AAB with R8 on: `flutter build appbundle --release --obfuscate --split-debug-info=build/symbols/android`.

## Screenshots (L-09, L-17)

Generated from the app's real screens with sample traffic by `app/test/store_screenshots_test.dart`, in English and Khmer:

| Store slot | Files | Size |
|---|---|---|
| iPhone 6.9″ | `docs/store/screenshots/ios-*-{en,km}.png` | 1320 × 2868 |
| Android phone | `docs/store/screenshots/android-*-{en,km}.png` | 1080 × 1920 |

Regenerate with `flutter test --update-goldens test/store_screenshots_test.dart`. The CarPlay template is rendered by the car, so capture it from Xcode's CarPlay simulator once the entitlement is granted.
