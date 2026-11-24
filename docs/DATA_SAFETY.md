# Google Play Data safety and iOS privacy answers (draft, Step 5)

Kept in step with the code; the store forms must match it exactly (CLAUDE.md "Store rules"). Final review in Step 13.

| Data | Collected? | Shared? | Why | Where it is kept |
|---|---|---|---|---|
| Precise location | Yes, while a drive runs | Broadcast to nearby phones inside signed safety messages under a pseudonym that changes every 5–10 minutes; not sent to the server in Step 5 | App functionality: collision warnings | Phone memory; own raw GPS trail deleted after 24 h (Step 8) |
| Device or other IDs | A device public key and pseudonym certificates | Pseudonyms on the radio; the server (Step 9) knows which certificates belong to which install, never to a person | Security: only genuine phones can send warnings | Keys in the Android Keystore, never exported |
| App activity (driving behaviour) | Scores of nearby vehicles, on the phone only | No | App functionality | Deleted 10 min after the vehicle leaves range |
| Diagnostics (field log) | Debug builds only: delivery %, signal strength, battery % per minute | Only if the tester sends the file | Field testing | Phone storage until deleted |

No names, phone numbers, plates, contacts, photos or advertising IDs are collected. The app makes no network connections in Step 5 (debug builds have Flutter's INTERNET permission for hot reload only).

Android permissions: Bluetooth scan and advertise (no connections), fine location, foreground service (connected device + location), notifications.
