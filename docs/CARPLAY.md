# CarPlay (BUILD_PLAN Step 10)

RoadMesh appears in CarPlay as a **Driving Task** app (SPEC §9). Apple allows simple templates in this category, not custom maps, so the car shows one list:

| Row | Content |
|---|---|
| Alert | The current Warning or Danger with the driving advice (e.g. "Motorbike crossing from your right in 3.4 s — Brake firmly"), coloured green / orange / red; "Alerts muted" under it when muted |
| Speed | Own speed against the limit ("48 km/h · limit 40"); red dot when more than 5 km/h over |
| Nearest vehicles | Up to 5: type, speed and distance, dot coloured by risk band |
| Mute / Unmute | Button in the navigation bar. It mutes the alert tone and vibration on the phone too, and is remembered. |

The map stays on the phone.

**How it works.**
- **Text.** The text is built in Flutter (`app/lib/ui/carplay.dart`) from the English/Khmer ARB strings and sent on the `roadmesh/carplay` channel. It is sent only when it changes.
- **Drawing.** The iOS scene (`ios/.../CarPlay.swift`, class `RoadMeshCarPlaySceneDelegate`) only draws it.
- **Safety logic.** Every alert comes from the Rust core. CarPlay and Flutter only word it.

## What you need to do

1. **Request the entitlement (AUDIT L-06).** Use the CarPlay entitlement request form at developer.apple.com/contact/carplay, category **Driving Task**.
   - **Description to give:** a road-safety app that warns drivers about nearby vehicles. It shows the current warning, the driver's speed against the limit and the nearest vehicles. It never controls the vehicle.
   - **Timing:** allow several weeks. The app must work without it, and it does; the phone screens are unchanged.
2. **Once granted, set up signing.**
   - In the Apple Developer portal, add the CarPlay Driving Task capability to the app ID (`com.roadmesh.app`) and regenerate the provisioning profiles.
   - In Xcode, add `Runner/Runner.entitlements` with `com.apple.developer.carplay-driving-task = true`. Set **Code Signing Entitlements** on the Runner target. Add `com.apple.developer.devicecheck.appattest-environment = production` in the same file (docs/SERVER.md).
3. **Check it in Xcode's CarPlay simulator ("done when").**
   1. On a Mac, run `scripts/build-ios-core.sh` and then `flutter run` on an iPhone simulator.
   2. In the Simulator, choose I/O → External Displays → CarPlay.
   3. Start a drive on the phone (a debug build can use the demo traffic).
   4. The CarPlay window shows RoadMesh with the list above. Use the hazard demo to see a red Danger row, and the Mute button to see the bell state change on the phone's Settings tab.

   Without the entitlement, the CarPlay simulator does not show the app. CI only checks that the scene compiles.

## Notes

- **CarPlay launched on its own.** If the car starts the app while the phone app has never been opened, the list is empty until Flutter starts. Open the app on the phone once.
- **Alert tone on iPhone.** iOS plays a system sound plus vibration and follows the ringer switch. Spoken alerts and the earbud audio for motorbike mode (SPEC §9) are still to come.
