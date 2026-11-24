# RoadMesh edge server

The edge server (BUILD_PLAN Step 9) does four things:

1. **Certificates.** It issues device and pseudonym certificates once a phone proves it runs the genuine app.
2. **Cellular bridge.** It passes verified hazard events between phones that cannot hear each other by radio.
3. **Revocation.** It collects misbehaviour reports and publishes the revocation list.
4. **Delete my data.** It handles the server half of "Delete my data".

Phones never depend on it for safety: alerts work with no internet (SPEC §2).

Code: `server/` (Rust, tokio, rustls). Codec: `core/src/mqtt.rs`. Map cells: `core/src/geohash.rs`.

## Ports and protocol

| Port | Use | TLS | Client authentication |
|---|---|---|---|
| 8883 | MQTT: uploads, events, revocation list, reports, delete | TLS 1.3 only | Device certificate (mutual TLS) |
| 8884 | Enrolment: certificate requests | TLS 1.3 only | App Attest / Play Integrity evidence inside the request |

- **TLS version.** rustls is built without TLS 1.2, so older clients fail the handshake.
- **ALPN.** Both ports use ALPN `mqtt`.
- **Pinning.** Phones pin the server key's SPKI hash, with one backup pin (AUDIT S-17).
- **MQTT subset.** MQTT 3.1.1 at QoS 0 only. There are no retained messages, wills, user names or passwords. Anything else closes the session.

| Topic | Direction | Payload |
|---|---|---|
| `rm/v1/up` | phone → server | one radio frame (the phone's own message or an event it heard) |
| `rm/v1/ev/<geohash6>` | server → phones | a verified hazard event in that cell (about 1.2 × 0.6 km) |
| `rm/v1/crl` | server → phones | signed revocation list, sent on subscribe and when it changes |
| `rm/v1/report` | phone → server | signed misbehaviour report |
| `rm/v1/delete` | phone → server | "delete my data" |
| `rm/v1/reply/report`, `rm/v1/reply/delete` | server → that phone | one result byte (see `broker::code`) |
| `rm/v1/enrol` | phone → server (port 8884) | certificate request (`pki::IssueRequestWire`) |
| `rm/v1/enrol/reply` | server → that phone | `[0][u16 len][device certificate DER][20 pseudonym certificates]` or `[error code]` |

A phone subscribes to its own cell and the 8 around it (`geohash::neighbourhood`).

- **No wildcards.** Wildcard filters are refused, as is anything beyond 9 cells per session. Nobody can follow events across a whole city.
- **What gets forwarded.** Only events are forwarded. Position heartbeats are verified and dropped, and the server keeps no positions.

### What the server checks

| Input | Check |
|---|---|
| Certificate request | Device-key signature, request at most 5 min old, App Attest or Play Integrity bound to the request, banned devices refused, at most 20 pseudonyms per device per week, keys never reused, and 5 attempts per minute per source address |
| App Attest | Attestation: chain to Apple's App Attestation Root (pinned in the binary), nonce extension, key id, app id, counter 0, environment. Renewals use assertions with a growing counter. One App Attest key may vouch for one device key only. |
| Play Integrity | The token is decrypted and verified locally with the Play Console keys. Nonce = SHA-256 of the request, package and signing certificate match, `PLAY_RECOGNIZED`, `MEETS_DEVICE_INTEGRITY` or stronger, verdict at most 10 min old. |
| Upload | Certificate issued by this CA (a full copy must be byte-identical), unexpired, not revoked, signature valid, fresh (1 s heartbeats, 10 s events), not a duplicate, and at most 25 per second per session with a burst of 50. Sessions that keep exceeding the rate are closed. |
| Report | Verified by the CA. A device is revoked after reports from 3 different devices; each device may file at most 50 reports a day. |

## Configuration

| Variable | Meaning |
|---|---|
| `RM_TLS_CERT`, `RM_TLS_KEY` | PEM server certificate chain and key (e.g. ACM-exported or Let's Encrypt). Mount from AWS Secrets Manager. |
| `RM_CA_KMS_KEY`, `RM_CA_KEY_ID` | Pseudonym CA key (ARN or alias) and its one-byte id in certificates |
| `RM_DEVICE_CA_KMS_KEY` | Device CA key (ARN or alias) |
| `RM_APPLE_APP_ID` | `TEAMID.com.roadmesh.app`; `RM_APPLE_DEVELOPMENT=1` accepts development App Attest keys (staging only) |
| `RM_PLAY_PACKAGE`, `RM_PLAY_CERT_DIGESTS` | Package name; comma-separated base64 SHA-256 digests of the app signing certificate, as Play reports them |
| `RM_PLAY_DECRYPTION_KEY_FILE`, `RM_PLAY_VERIFICATION_KEY_FILE` | Files holding the Play Console response keys (base64). Mount from Secrets Manager; never put them in variables or images. |
| `RM_MQTT_ADDR`, `RM_ENROL_ADDR` | Listen addresses (defaults `0.0.0.0:8883`, `0.0.0.0:8884`) |

Build the production binary with the `aws-kms` feature: `cargo build --release -p roadmesh-server --features aws-kms`. Without it, the server refuses to start. There is no software CA key in production code.

At start-up the server logs the pseudonym CA trust anchor (base64). Build that anchor into the app.

`roadmesh-server pins server.pem` prints the SPKI pin of each certificate in a PEM file, for the app configuration. Always ship the current pin and a backup pin (a key held offline for rotation).

## AWS set-up (Singapore, ap-southeast-1)

**KMS (AUDIT S-19).**
- **Keys.** Create two customer-managed keys, both with `KeySpec ECC_NIST_P256` and `KeyUsage SIGN_VERIFY`: `alias/roadmesh-ca` and `alias/roadmesh-device-ca`.
- **Server role.** The server's IAM role gets `kms:Sign` and `kms:GetPublicKey` on these two keys only.
- **Who can change the keys.** Key-policy changes, deletion and disabling are allowed only through a role that needs approval from two named people. Use IAM Identity Center with a permission set behind an approval workflow, plus a CloudTrail alarm on `PutKeyPolicy`, `ScheduleKeyDeletion` and `DisableKey`.

**Network (AUDIT S-18).**
- **Load balancer.** A Network Load Balancer passes TCP through to the server, which terminates TLS itself; mutual TLS needs that.
- **Security group.** Allow only ports 8883 and 8884 from the internet, plus outbound HTTPS to KMS through a VPC endpoint.
- **Rate limits.** AWS Shield Standard sits in front. Rate limits are built into the server.

**Containers.** Run as a distroless, non-root image on ECS Fargate with automatic platform updates. Image scanning runs in ECR.

**Logs (AUDIT S-21).**
- **Format.** Standard output is JSON lines, sent to CloudWatch Logs with **365-day retention**:
  - **Audit lines:** `{"t":…,"audit":"enrolled"|"enrol_refused"|"revoked"|"delete"|"ALERT",…}`.
  - **Metrics:** one line a minute, `{"metrics":"minute",…}`.
- **Alarms.** Create metric filters and alarms on `"audit":"ALERT"`, which covers abnormal certificate requests, refused requests, misbehaviour-report spikes and rejected uploads (thresholds in `monitor::Thresholds`). Also alarm on API errors (`SessionsRefused`).
- **No personal data in logs.** Lines never contain coordinates, pseudonyms, keys or message contents. A device appears only as the first 8 bytes of its device id.

## Known gaps (tracked for Step 11)

- **No persistent store yet.** The CA state (issued certificates, bans, report counts) lives in memory. A restart forgets it, and every phone must enrol again; bans are lost.
  - Before the pilot, add an encrypted store: DynamoDB with KMS encryption at rest (AUDIT S-20).
  - Store the App Attest key registry there too.
  - With several server instances, share subscriptions through the store (SPEC §11 names Redis for live map cells).
- **Delete my data.** The server forgets a device's report history at once. It keeps which certificates the device holds, and any ban, only until its last certificate expires (at most one week). Otherwise a device could escape revocation by deleting its data. The privacy statement and the Data safety form must say so.
- **Hazard-event storage.** SPEC §10 asks for hazard events kept 30 days, anonymised and snapped to road segments. That is not stored yet; events are only passed on.
- **Android key attestation.** Play Integrity proves a genuine app and device. It does not prove the device key is hardware-backed. Checking the Android Keystore key-attestation chain is planned.
