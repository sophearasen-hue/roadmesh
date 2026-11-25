# Store privacy answers: Google Play Data safety and Apple App Privacy

The store forms must match this page, and this page must match the code (CLAUDE.md "Store rules", AUDIT L-05 and L-13). The app's `PrivacyInfo.xcprivacy` declares the same data types. Last checked against the code: Step 14.

## What the app handles

| Data | Leaves the phone? | Where it goes | Why | How long |
|---|---|---|---|---|
| **Precise location** | Yes. Inside signed safety messages broadcast by Bluetooth to nearby phones, about 10 a second while a drive runs. Inside **hazard events only** (hard braking, stopped traffic) sent to the edge server over 4G. | Nearby phones (radio). The edge server forwards events in real time to phones in the same area and does not store them. | App functionality: collision warnings | Nearby phones: deleted 10 min after the vehicle leaves range. Server: not stored. Own phone: no position trail is stored. |
| **Pseudonym** (random ID in each message) | Yes, with the location | Nearby phones; the server sees it in hazard events | Lets phones track a vehicle for a few seconds | Changes about every 5 minutes (only when stopped or at a junction) |
| **Device ID** (a key made in the phone's secure hardware, and its certificates) | Yes, to the edge server when the phone enrols and connects | Edge server (RoadMesh). It knows which certificates belong to one install, never to a person: there are no accounts. | Security: only genuine apps can send warnings; misbehaving devices can be revoked | While the app is installed. After "Delete my data", the server keeps only what blocks misuse, until the last certificate expires (at most 1 week). |
| **Driving behaviour** (own trip score, speeds, event counts; scores of nearby vehicles) | No | Phone only | App functionality: the Behaviour screen | Own trip: until "Delete my data". Nearby vehicles: deleted 10 min after they leave range. |
| **Misbehaviour reports** (signed messages a phone found impossible, e.g. a car at 300 km/h) | Yes, to the edge server | Edge server | Security: revoke forged senders | Counts kept until the accused certificates expire |
| **Diagnostics** (field-test log: delivery %, signal strength, battery % per minute; no positions or IDs) | Only if the tester exports and sends the file | The tester | Field testing | Until deleted; debug builds only |

**Not handled at all:** names, phone numbers, email, plates, contacts, photos, microphone, advertising IDs, payment data, web browsing.

- **Tracking:** none. No data is used for ads or shared with data brokers.
- **Encryption in transit:** Bluetooth messages are signed but not encrypted. They are meant to be heard by nearby vehicles, and carry only position, motion and a pseudonym. Server traffic uses TLS 1.3 with certificate pinning and mutual TLS.
- **Deletion:** "Delete my data" in Settings removes the trip, the behaviour records and every vehicle heard from the phone, and asks the server to forget the device (see above). Backups are turned off, so nothing is copied to Google or iCloud backups.

## Google Play Data safety form

| Question | Answer |
|---|---|
| Does your app collect or share any of the required user data types? | Yes |
| Is all of the user data collected by your app encrypted in transit? | Yes. Server traffic uses TLS 1.3. Explain in the notes that Bluetooth safety broadcasts are signed, public by design and carry no personal identity. |
| Do you provide a way for users to request that their data is deleted? | Yes, in the app: Settings → Delete my data |
| **Location → Precise location** | Collected: yes. Shared: yes, with other users of the app nearby (broadcast). Processed ephemerally: yes for the server (forwarded, not stored). Required, since the app cannot warn without it. Purpose: App functionality. |
| **Device or other IDs** | Collected: yes. Shared: no. Required. Purpose: Fraud prevention, security, and compliance. |
| **App activity → Other actions** (driving behaviour) | Not collected: it stays on the phone. |
| **App info and performance → Diagnostics** | Not collected: the field log is debug-only and user-exported. |
| Everything else (personal info, financial, health, messages, photos, audio, files, calendar, contacts, web history) | Not collected |

## Apple App Privacy ("nutrition label")

| Question | Answer |
|---|---|
| Do you or your partners collect data from this app? | Yes |
| **Location → Precise Location** | Used for App Functionality. Not linked to the user's identity. Not used for tracking. |
| **Identifiers → Device ID** | Used for App Functionality (security). Not linked to the user's identity. Not used for tracking. |
| All other categories | Not collected |

## Permissions and why

**Android:**
- `BLUETOOTH_SCAN`, `BLUETOOTH_ADVERTISE`, `BLUETOOTH_CONNECT`: hear and send safety messages, and keep short links to nearby iPhones. No pairing. Scanning is used to receive messages; `neverForLocation` is not set because the app holds location anyway.
- `ACCESS_FINE_LOCATION`: own position in the safety message.
- `FOREGROUND_SERVICE` with the types `connectedDevice` and `location`: keep warning with the screen off.
- `POST_NOTIFICATIONS`: the persistent drive notification.
- `INTERNET`: the edge server.
- `VIBRATE`: alert vibration.

**iOS:**
- **Bluetooth (always):** the safety messages.
- **Location when in use, and in the background during a drive:** the own position.
- **Background modes:** `bluetooth-central`, `bluetooth-peripheral` and `location`, each justified in `docs/STORE.md`.
- **Purpose texts:** in English and Khmer (`app/ios/Runner/*.lproj/InfoPlist.strings`).
