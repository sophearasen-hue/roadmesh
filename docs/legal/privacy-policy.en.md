# RoadMesh privacy policy

*Draft for legal review (AUDIT L-20). Effective date: to be set at publication.*

RoadMesh warns drivers about nearby vehicles. It is built to need as little data as possible.

**What we never collect.** Your name, phone number, email address, number plate, contacts, photos, or advertising identifiers. There are no accounts.

**What the app sends, and to whom.**
- **Your position, speed and heading**, while a drive you started is running, about ten times a second, by Bluetooth to nearby phones running RoadMesh.
  - These messages carry a random identifier that changes about every five minutes, never anything that names you.
  - Other phones delete what they heard about your vehicle 10 minutes after you leave their range.
- **Hazard warnings** (for example "hard braking"), with their position, to the RoadMesh server over an encrypted connection.
  - The server passes them on at once to phones nearby, and does not store them.
- **A security key created in your phone's secure hardware**, and the certificates the server issues for it.
  - These let the server check that warnings come from the genuine app, and block devices that send false warnings.
  - The server does not know who you are.
- **Proof that the app is genuine**, from Apple App Attest or Google Play Integrity, when your phone asks for certificates.

**What stays on your phone.** Your trip score, speeds and driving events, and the scores your phone gives nearby vehicles. They are never uploaded, and are not included in phone backups.

**Deleting your data.** Settings → Delete my data removes your trip, your driving records and every vehicle your phone heard.
- It also asks the server to forget your device.
- To stop misuse, the server keeps which certificates your device holds, and any block, until they expire, at most one week later.
- Uninstalling the app also removes everything on the phone.

**Security.**
- Messages are digitally signed.
- Server connections use TLS 1.3 with certificate pinning.
- Keys never leave your phone's secure hardware.

**Children.** RoadMesh is meant for drivers and is not directed at children.

**Changes and contact.** We will post changes here. Questions: `<privacy contact to be set by the project owner>`.
