# RoadMesh — Technical Specification v0.1

Nov 22, 2026 · @Sopheara Sen

## 1. Overview

RoadMesh turns any smartphone into a vehicle safety radio. Phones broadcast their position and motion to nearby phones, relay messages for each other, and warn drivers and riders 2 to 6 seconds before a likely conflict. It works for any car, motorbike or tuk-tuk, of any brand, with or without CarPlay.

**Problem.** Factory V2V radios exist only in a few new cars and will take 15 to 20 years to spread. In Cambodia, motorbikes are about 85% of vehicles and most road deaths, and they will never get factory radios.

**Goals (v1)**

- Show every RoadMesh user within mesh range on a live map, with speed, distance and link type.
- Warn of four conflicts: crossing traffic at a junction, hard braking ahead, a vehicle overtaking you, a fast vehicle closing from behind. Also warn about your own speeding.
- Record driving behaviour for the user and for observed peers: speeding, harsh acceleration, harsh braking, overtaking. Turn it into a 0 to 100 score.
- Full English and Khmer interface, switchable at any time.
- Work phone to phone with no internet, and use 4G when it is available.

**Non-goals (v1)**

- No control of brakes or steering. Warnings only.
- No last-second crash avoidance (under 1 s). Phone radios and GPS are not fast or precise enough.
- No identity, plate number or face data, for any user.

**Target users**

| Segment | Device | Why they adopt first |
| --- | --- | --- |
| Ride-hailing and delivery fleets | Phone + optional long-range dongle | One contract equips thousands of vehicles in one city |
| Motorbike riders | Phone in pocket or mount, earbud audio | The highest-risk group, unserved by factory V2V |
| Car drivers | Phone mount or CarPlay | Familiar with navigation apps |
| Pedestrians (later) | Phone | Protects children near schools |

The working browser prototype is at [RoadMesh prototype](https://claude.ai/artifact/X8tzCQ1aPVamqbQuSou7Eb). This spec turns its logic into a native app.

## 2. System architecture

Each phone runs the full safety stack itself, so alerts work with no internet. The edge server only joins up groups of phones that cannot hear each other and spreads hazard events further.

> **Diagram not included in this export:** RoadMesh architecture · phone stack, peers, dongle, edge, cloud. See the original spec document.

The safety core is one shared library. The same code runs on iPhone, Android and the traffic simulator, so a scenario that passes in simulation behaves the same on the road.

**Data flow for one alert.** A peer's sensors produce an RSM, which is signed and broadcast. Your phone receives it by radio (or 4G), verifies it, updates the LDM, predicts both paths and raises the alert. Target end to end: under 150 ms for a direct link, under 400 ms over 3 hops.

## 3. RoadMesh Safety Message (RSM)

Every device broadcasts one compact binary RSM. It is 31 bytes before the signature (the fields below), so it travels in one Bluetooth LE extended advertisement or one LoRa packet. The field set follows SAE J2735's Basic Safety Message, reduced to what a phone can measure.

| Field | Type | Bytes | Notes |
| --- | --- | --- | --- |
| version + msg\_type | uint8 | 1 | 4 bits each. Types: 0 heartbeat, 1 event, 2 relay |
| temp\_id | bytes | 4 | Random pseudonym, rotates every 5 min (section 10) |
| msg\_id | uint16 | 2 | Counter per temp\_id, used for duplicate detection |
| timestamp | uint32 | 4 | ms since the hour, GPS-disciplined clock |
| lat, lon | int32 × 2 | 8 | 1e-7 degrees (about 1 cm resolution) |
| speed | uint16 | 2 | 0.02 m/s units |
| heading | uint16 | 2 | 0.01 degree units |
| accel\_long | int8 | 1 | 0.1 m/s² units, negative = braking |
| yaw\_rate | int8 | 1 | 0.5 deg/s units |
| vehicle\_type | uint4 | 0.5 | car, motorbike, tuk-tuk, truck, bus, pedestrian, bicycle |
| flags | uint4 | 0.5 | hard-brake active, overtaking, hazard lights, emergency |
| pos\_accuracy | uint8 | 1 | Estimated horizontal error in 0.25 m units |
| hop\_count + ttl | uint8 | 1 | 4 bits each (section 5) |
| origin\_pos\_age | uint16 | 2 | ms since the origin sent it (relayed messages only) |
| event\_code | uint8 | 1 | Event messages only: 1 crash, 2 hard brake, 3 wrong-way, 4 hazard, 5 stopped traffic (0 = none) |
| signature | bytes | 73 or 172 | Security envelope (section 10): ECDSA P-256 signature (64) + certificate digest (8), or + full certificate (107). Can be replaced by a 16-byte MAC on LoRa |

**Send rates**

| Situation | Rate |
| --- | --- |
| Moving, within 150 m of a junction or with 3+ peers within 100 m | 10 Hz |
| Moving, open road | 4 Hz |
| Stopped for more than 30 s | 1 Hz |
| Event (hard brake, crash) | Immediately, 3 repeats within 300 ms |

Encoding is a fixed binary layout, not JSON, to keep airtime short. Version 0 is frozen once the pilot starts; new fields go in version 1.

**Relay-mutable fields and the radio frame (Step 2).** Relays change only hop\_count, ttl and origin\_pos\_age. The originator signs the body with those three fields set to zero, so the signature still verifies after relaying while any other change breaks it. The radio frame around the RSM carries the last relay's position (needed for the distance back-off in section 5); it is outside the signature, like the ETSI GeoNetworking forwarding header. The security envelope (Step 3) follows the body in the frame's auth field, at most 176 bytes; a whole frame is at most 218 bytes and fits one Bluetooth LE extended advertisement.

**Event messages (Step 2).** A hard-brake event is sent as soon as deceleration reaches 4 m/s² and repeated on the next 3 messages. A vehicle stopped for 5 s away from a junction sends a stopped-traffic event (code 5) once a second. Events use TTL 5, so a queue in fog or around a bend is announced well beyond one radio hop.

## 4. Radio and transport layer

The app uses whichever links are available at the same moment, behind one transport interface. Bluetooth LE is the baseline because it is the only radio both iPhone and Android can use with each other.

| Link | Range per hop (approx.) | Latency per hop | Platforms | Role |
| --- | --- | --- | --- | --- |
| Bluetooth LE extended advertising (Coded PHY where supported) | 50 to 150 m, more with Coded PHY | 20 to 50 ms | iOS + Android | Baseline peer-to-peer, all users |
| Wi-Fi Aware (NAN) | 100 to 300 m | about 10 ms | Android (check current iOS support) | Faster, longer range between capable phones |
| LoRa add-on dongle (BLE to phone) | 0.5 to 2 km | 100 to 300 ms | Any phone | Fleets, rural roads, relay backbone |
| 4G/5G via edge server | Unlimited | 50 to 300 ms, varies by network | All | Fallback, crowd hazards, wide-area view |

All ranges are planning estimates. Measure them in the Phase 0 field test (section 12) before setting thresholds.

**Platform constraints to design around**

- **iOS background limits.** iOS restricts Bluetooth advertising when the app is in the background. Mitigation: the CarPlay session and an active "Drive mode" keep the app in the foreground state. Background mode becomes receive-only plus 4G.
- **iPhone ↔ Android.** Apple's Multipeer Connectivity is iPhone-only and Wi-Fi Direct is Android-only. So cross-platform traffic always uses Bluetooth LE or 4G.
- **Android battery optimisation.** The app runs as a foreground service with a persistent notification while Drive mode is on.
- **Battery budget.** Target under 8% per hour while driving with the screen off. Rates adapt (section 3), and scanning duty-cycles when no peers are heard for 60 s.
- **Radio approval.** The LoRa dongle needs type approval from the Telecommunication Regulator of Cambodia (TRC) and a permitted band. Confirm the band before ordering hardware.

**Link arbitration.** Each incoming RSM is tagged with its link. If the same msg\_id arrives by several links, the first copy is used and the others are dropped. The HUD shows the link type (direct radio, relayed, or cellular).

*Implementation note (Step 5, Android).* Each frame (up to 218 bytes) travels in one non-connectable Bluetooth LE extended advertisement as service data under the 128-bit RoadMesh UUID `4b6e7a10-2d3c-4e5f-8a9b-0c1d2e3f4a5b`, every 100 ms, on Coded PHY where the phone supports it (otherwise 1M). One long-running scan with a filter on that UUID hears everyone. No connections or pairing. Phones without extended advertising are receive-only. The safety engine runs in a foreground service (type `connectedDevice` + `location`). The advertising address is Android's random private address, renewed by restarting the advertising set whenever the pseudonym changes; pseudonyms change only when stopped or at a junction, at most 5 minutes late (AUDIT S-13).

*Implementation note (Step 6, iOS).* iOS advertises only service UUIDs, so an iPhone advertises the RoadMesh UUID and sends each frame as GATT notifications on characteristic `4b6e7a11-2d3c-4e5f-8a9b-0c1d2e3f4a5b`, cut into `[frame_no][index][count][payload]` fragments to fit the link (MTU 247 requested). Android phones and other iPhones connect (at most 6 links) and subscribe; links unheard for 5 s are dropped. iPhones hear Android phones' extended advertisements where the hardware reports them. In the background, iOS advertises only to other iPhones, so the app keeps Drive mode on screen (CarPlay, Step 10); sending over 4G (Step 9) is the fallback.

## 5. Mesh relay protocol

Phones re-broadcast what they hear, so a warning can travel several hops beyond one radio's range. The rules below keep that from flooding the channel.

1. **Hop limit.** Every RSM carries a TTL. Heartbeats: TTL 2. Event messages (hard brake, crash, wrong-way): TTL 5. Each relay decrements it and stops at 0.
2. **Duplicate cache.** Each phone keeps (temp\_id, msg\_id) pairs for 5 s and never relays the same message twice.
3. **Relay selection by distance.** On hearing a message, wait a back-off time inversely proportional to your distance from the sender (0 to 60 ms). If another phone relays it first, cancel. The farthest phones relay first, so each hop covers the most new ground.
4. **Direction filter.** Relay heartbeats only along the road the sender is on, and only if you are moving away from it. Event messages relay in all directions.
5. **Geographic cut-off.** Drop any relayed message whose origin is more than 1.5 km away. Information that far out cannot change a driver's next 6 seconds.
6. **Congestion control.** If a phone hears more than 60 distinct peers, it halves its heartbeat rate and relays only event messages.
7. **Freshness.** Discard a relayed heartbeat older than 1 s, and an event older than 10 s.

*Implementation note (Step 7).* All seven rules run in the core (`relay_rules`, product profile only; the reference profile keeps its immediate relaying so the golden tests stay exact). Distances in rules 3 and 4 are measured from the last hop's position in the radio frame. A relay still needs at least 100 m of progress from the last hop; the back-off then falls from 60 ms at 100 m to 0 ms at 150 m (one radio range). A pending relay is cancelled only by a copy whose signed bytes and signature equal the original, so a forged copy cannot silence a relay. Rule 4 counts a phone as on the sender's road within 15 m sideways and ±37° of its heading; a stopped phone counts as moving away. Rule 5 applies to relayed messages (hop count above 0); the peer is dropped from the map. Rule 6 counts peers in the local map. The platform may call `due_relays` between transmit cycles for finer back-off timing.

**Cellular bridge.** A phone with 4G uploads its own RSM and any event messages it hears to the regional edge server. The server pushes events to subscribed phones in the same map cells. This joins up mesh islands that cannot hear each other by radio.

## 6. Local Dynamic Map, tracking and prediction

Each phone keeps a Local Dynamic Map (LDM): a small live table of every peer it has heard in the last 2 s, placed on the road map. Every alert and score reads from it.

**LDM layers on the phone**

| Layer | Content | Source | Refresh |
| --- | --- | --- | --- |
| 1. Static | Roads, lanes, junctions, speed limits | OpenStreetMap extract for Cambodia, cached offline | Monthly |
| 2. Semi-static | Crash hotspots, school zones, roadworks | Edge server | Daily |
| 3. Transient | Crash, flooding, wrong-way events | RSM events + server | Seconds to minutes |
| 4. Dynamic | Peer tracks | RSM heartbeats | 4 to 10 Hz |

**Tracking pipeline (per incoming RSM)**

1. Verify the signature and check plausibility (section 10). Reject on failure.
2. Associate it with a track by temp\_id. When an ID rotates, link the new ID to an existing track if position and velocity match within 3 m and 2 m/s.
3. Update a constant-turn-rate-and-velocity Kalman filter. This smooths phone GPS noise (typically 2 to 10 m) and estimates true speed, heading and acceleration.
4. Map-match to a road segment and lane direction using heading and the road graph. Store the match confidence.
5. Expire the track 2 s after its last update.

**Prediction.** Every 100 ms, project each track and the user 6 s ahead in 100 ms steps. Two tracks conflict when their predicted positions come within a conflict radius at the same step. The radius is 2.5 m plus both position uncertainties, so noisy GPS widens the radius rather than causing false certainty.

*Implementation note (Step 2).* The core predicts vehicle outlines (rotated rectangles), not points. Each outline is lengthened at both ends by that vehicle's reported position accuracy. It is not widened sideways: on a two-lane road the lanes are only 3.5 m apart, and sideways widening made every oncoming vehicle a false conflict. Separating adjacent lanes is the job of map matching (Step 4).

*Implementation note (Step 4).* Step 3 of the pipeline is a Kalman filter on position whose prediction uses the reported speed, heading, acceleration and turn rate (more precise than GPS position); it removes fix-to-fix jitter but not slowly drifting GPS bias. Step 4 keeps a matched track on its own side of the road (between the lane centres of its direction) and aligns straight-driving tracks with the road; it does not pick a lane, because phone GPS cannot tell adjacent same-direction lanes apart. For same-direction vehicles whose combined position error exceeds 1.5 m the lateral error is added to the conflict check, and such a rear-end conflict is a Warning with braking advice until it is under 2 s away, then Danger. Map-match confidence treats the reported accuracy as a 68 % radius. Inside junction areas only vehicles aligned within 15° of a road and not turning are kept on their side; turning vehicles keep their measured path. Results: `docs/SIM_REPORT.md`.

**Confidence gate.** No Warning or Danger alert is raised from a track with map-match confidence under 0.7 or position accuracy worse than 8 m. Such tracks still appear on the map as faded icons. If such a track is on a collision course, the driver gets a Caution "possible conflict, ease off" so they are never left without a hint (decision of 2026-11-23); it never becomes Danger, so it cannot cause false emergency alarms.

## 7. Behaviour detection and scoring

The same detector runs on the user's own motion and on every peer track. Peer events are recorded only while that peer is in the user's mesh, so a score reflects what was actually observed.

| Event | Trigger (starting values, tune in pilot) | Ends when | Score penalty |
| --- | --- | --- | --- |
| Speeding | Speed above limit + 10 km/h for 3 s | Speed falls below limit + 5 km/h | −6 |
| Harsh braking | Longitudinal deceleration above 4.5 m/s² | Deceleration under 2 m/s² | −5 |
| Harsh acceleration | Acceleration above 3.0 m/s² (motorbikes: 3.5) | Under 1.5 m/s² | −3 |
| Overtake | Passes a same-direction track by more than 10 m after a lateral shift | Completed | −1 (−4 if within 50 m of a junction or across a solid line) |
| Tailgating (v1.1) | Time gap under 0.8 s for 5 s above 30 km/h | Gap above 1.2 s | −2 |
| Wrong-way (v1.1) | Heading opposite to a one-way segment for 3 s | Heading corrected | −10 + event broadcast |

**Score.** Each trip starts at 100. Penalties apply once per episode; an episode re-arms only after its end condition. The score recovers 1.2 points per minute of clean driving, capped at 100. Bands: Safe 85 to 100, Caution 65 to 84, Risky under 65.

**Per-trip record (user only, stored on the phone)**

- Distance, duration, average and top speed, speed profile at 1 Hz
- Event list: time, type, speed, road segment (no raw GPS trail kept after 24 h unless the user opts in)
- Final score and band

**Peer record (stored on the phone only).** temp\_id, vehicle type, current score, event counts and a 2-minute speed history. It is deleted when the track expires, plus 10 minutes. It is never uploaded with any identifier.

## 8. Alert logic

Only one alert shows at a time: the highest-priority active one. Each alert has an entry threshold and a looser exit threshold (hysteresis), so the banner does not flicker.

| Priority | Alert | Enter when | Exit when | Level |
| --- | --- | --- | --- | --- |
| 1 | Crossing conflict | Both reach the same junction within 7 s, arrival times within 2.2 s, both above 2 m/s | Either has passed the junction, or the time gap grows above 3 s | Danger |
| 2 | Hard braking ahead | Same road and direction, 0 to 200 m ahead, peer deceleration above 4 m/s² or hard-brake flag set | Peer deceleration under 1.5 m/s² for 1 s | Danger |
| 3 | Overtaking you | Peer overtaking flag set, within 25 m beside or behind | Peer 10 m ahead or flag cleared | Warning |
| 4 | Fast approach from behind | Same lane, 0 to 80 m behind, closing faster than 5 m/s (18 km/h) | Closing speed under 2 m/s | Warning |
| 5 | Own speeding | Above limit + 5 km/h | At or below the limit | Warning |

**Additions (Step 2).** These alerts come from the prediction and right-of-way engine and sit between priorities 1 and 2:

| Alert | Enter when | Exit when | Level |
| --- | --- | --- | --- |
| Lost contact | The vehicle I am giving way to stops being heard | It is heard again and has passed, or 3 s with no conflict | Danger (fail-safe: keep slowing) |
| Not yielding | A vehicle that must give way is on a collision course within 2 s and is not slowing | Conflict gone | Danger, for the vehicle with priority |
| Vehicle ahead / oncoming | Predicted collision within 4 s with a slow or stopped vehicle ahead, or an oncoming one | The vehicle has cleared | Danger |
| Hazard ahead (stopped traffic) | Stopped vehicle in my lane ahead, heard directly or relayed (up to 3 km) | Gone, or 8 s after the last report | Warning |

"Hard braking ahead" is not shown separately for a vehicle that already has one of these Danger alerts.

**Danger needs a confirmed conflict (decision of 2026-11-24).** A crossing conflict is Danger only when the collision prediction confirms that the two paths meet (or miss by less than 1 s at the crossing point); otherwise it is a Warning with the same braking advice. Hard braking ahead is Danger only when the braking vehicle is inside my stopping distance (1 s reaction plus braking at 4.5 m/s²) or on a predicted collision course; otherwise a Warning. A stopped or crawling vehicle (under 2 m/s) in my path is always a "vehicle ahead" Danger, whatever its direction. "Not yielding" applies to crossing traffic too, when the vehicle that must give way is braking less than 1 m/s².

**Driver instruction.** Every Danger and Warning alert carries an instruction worked out from the gentlest deceleration that avoids the conflict: *Ease off*, *Brake gently* (up to 2 m/s²), *Brake firmly* (up to 4.5 m/s²) or *Brake hard now*. The app shows and speaks it. It is advice for the driver; the app never brakes (section 1, non-goals).

**Right of way.** For a crossing conflict, the vehicle that must yield gets the Danger alert. Yield follows the road hierarchy (minor road yields to major road), then traffic from the right. The vehicle with priority gets a Caution alert. This avoids the symmetric deadlock found in the prototype, where both sides braked for each other.

**Warning style**

| Level | Screen | Sound | Motorbike mode |
| --- | --- | --- | --- |
| Danger | Full-width red banner, target ringed on map | Two-tone chime + spoken alert | Spoken alert in earbud + 3 short vibrations |
| Warning | Amber banner | Single soft tone | Spoken alert only |
| Caution | Map icon highlight only | None | None |

**Spoken alerts** use pre-recorded Khmer and English clips (for example "Motorbike crossing from the right"), not text-to-speech, for clarity at speed. The same alert is not repeated within 8 s unless its level rises.

**False alarm budget.** Target fewer than 1 Danger alert per 100 km that the driver rates as unnecessary (rated by a one-tap thumbs-down after the trip).

## 9. App screens and UX

The screens match the [prototype](https://claude.ai/artifact/X8tzCQ1aPVamqbQuSou7Eb). The rule for every screen while moving is: one glance, under 2 seconds, no typing.

| Screen | Content | Interaction while moving |
| --- | --- | --- |
| Onboarding | Language choice first (English / ខ្មែរ), vehicle type, permissions (location, Bluetooth, notifications), privacy summary | Blocked while moving |
| Drive (home) | Map centred on user, peers coloured by risk band, speed labels, radio links, alert banner, speedometer + limit sign, peer count, mesh reach, trip score | View only; large Network/Nearby toggle |
| Peers | List sorted by distance: type, temp\_id, speed, distance, link (direct / relayed n times / cellular), road, risk badge | Blocked above 10 km/h (shows Drive instead) |
| Peer detail | Speed, acceleration, link latency, 2-min speed graph, event counts and list | Blocked while moving |
| Behaviour | Trip score ring, distance, average and top speed, event chips, speed graph, event timeline (All / Me / Others), weekly trend | Blocked while moving |
| Settings | Language, vehicle type, radio options, long-range dongle pairing, alert volume and voice, motorbike mode, data and privacy controls, delete my data | Blocked while moving |

**Motorbike mode.** The screen may be off. All alerts are audio (earbud) plus phone vibration. An optional single-button Bluetooth remote can mute alerts.

**CarPlay (Driving Task category).** Apple's CarPlay rules for this category allow simple templates, not custom maps. The CarPlay screen therefore shows a list or grid template: current alert, own speed against the limit, the nearest 3 to 5 vehicles with speed and distance, and a mute control. The full map stays on the phone. Apple must approve the entitlement before launch, so apply in Phase 1.

**Localisation (English and Khmer)**

- All strings in resource files (Flutter ARB), keyed. No hard-coded text. The prototype's string table is the starting set.
- The language switch applies instantly with no restart, and is stored per device.
- Font: Kantumruy Pro or Noto Sans Khmer, bundled in the app (not downloaded). Test Khmer stacked consonants and vowel signs at 11 to 40 pt.
- Khmer text runs 20 to 40% longer than English. Every label must allow 2 lines without overlapping.
- Numbers in Western digits by default, with an option for Khmer digits. Units: km/h (គ.ម/ម៉ោង), metres.
- A native-speaking reviewer signs off every string and every recorded voice clip before release.

## 10. Security and privacy

A fake "hard braking" message could cause a crash, and position data could be used to track people. Both risks are designed out from v1.

**Message authenticity**

- Each install generates an ECDSA P-256 *device key* in the phone's secure hardware (Secure Enclave / Android Keystore, StrongBox where available). It identifies the install to the server only and is never broadcast.
- Each pseudonym has its own key pair, also generated in secure hardware; a shared key would make the pseudonyms linkable. The phone sends the 20 public keys, signed by the device key and with App Attest / Play Integrity evidence, and the server issues at most 20 pseudonym certificates per device in any rolling 7 days, each valid for one week. The server knows the device; peers see only rotating pseudonyms.
- **Certificate (107 bytes):** version, CA key id, valid from, valid until (Unix seconds), compressed public key (33), CA signature (64). The temp\_id is the first 4 bytes of SHA-256 over the certificate, so a pseudonym belongs to exactly one certificate.
- Every RSM is signed with the current pseudonym key. The signature also covers the sender's Unix hour (the body carries only ms since the hour), so a recorded message cannot be replayed an hour or a week later.
- **Security envelope:** most messages carry the signature and an 8-byte certificate digest (73 bytes). The full certificate (172 bytes) is attached at least every 0.3 s (every third message at 10 Hz), on every event message, and for 2 s after each pseudonym change, so a phone that hears a new vehicle can trust it within 0.3 s. (Decision of 2026-11-24: with once a second, the Step 4 simulation missed 7.2 % of conflicts against 4.1 % without signatures, mostly vehicles first heard coming round a blind corner; every 0.3 s matched no delay. Average envelope about 106 bytes.) Receivers cache verified certificates until they expire.
- **Verification (v1):** every message is verified. Sampling heartbeats 1 in 4 on busy channels (over 40 peers) exists as a setting, off by default; it is decided in Step 5 with CPU measurements from real phones. Copies of a message already verified (relays) are dropped before the signature check.
- **Revocation list:** the CA signs a list with a serial number, issue time, next-update time and the sorted 8-byte digests of revoked certificates that have not yet expired. Phones refresh it daily and on each connection, and never accept a list with a lower serial. (Chosen over a compact filter: exact, and small because certificates expire within a week.)
- On LoRa, the signature is replaced by a 16-byte MAC under a rotating group key, because full signatures cost too much airtime.

**Misbehaviour detection.** Reject and report a message when any of these hold:

- Speed above 200 km/h, or acceleration above 12 m/s²
- Position jumps more than speed × time + max(15 m, 3 × the reported accuracy, counted up to 20 m) between messages (decision of 2026-11-24: phones at about 10 m accuracy jump more than 15 m between fixes; the reference's speed-ratio rule blocked honest phones with ordinary GPS jitter)
- Claimed position not on or near a road (more than 50 m, or 3 × its accuracy, outside every road, within the map's coverage)
- The sender claims to be within 30 m but its signal strength is impossible for that distance

The reporting phone blocks the sender locally at once and sends the server a report signed with its own pseudonym key, carrying the offending signed messages as evidence. The server re-checks the signatures and the evidence, and revokes **all** certificates of a device once 3 different devices have filed confirmed reports (several pseudonyms of one phone count once). A revoked device gets no new certificates. Each device may file at most 50 reports a day. The "not near a road" check arrives with map matching (Step 4) and the signal-strength check with the Bluetooth transport (Step 5).

**Privacy rules**

| Data | Where kept | How long | Leaves the phone? |
| --- | --- | --- | --- |
| Own trips (score, events, speed profile) | Phone | Until the user deletes | Only if the user opts in (e.g. fleet or insurer programme) |
| Own raw GPS trail | Phone | 24 h | No |
| Peer tracks and peer scores | Phone | Track expiry + 10 min | Never with an identifier |
| Hazard events (crash, hard-brake clusters) | Edge server | 30 days | Yes, anonymised and snapped to road segments |
| Device certificate identity | Server | Account life | Server only, never broadcast |

- No names, phone numbers or plate numbers are ever broadcast.
- "Delete my data" removes everything on the phone and the server in one step.
- The privacy summary is shown in both languages at onboarding, before any permission prompt.
- Get a legal review of data handling under Cambodian law before the public launch.

## 11. Tech stack and project structure

One Flutter codebase for the screens, with native modules for everything that touches radios, sensors or CarPlay. The safety logic lives in one shared core so iPhone and Android behave identically.

| Layer | Technology | Why |
| --- | --- | --- |
| UI | Flutter (Dart), flutter\_map with offline vector tiles | One UI for iOS and Android; works without internet |
| Safety core (LDM, tracking, prediction, alerts, scoring) | Rust library, called from Dart via FFI and from Swift/Kotlin | Fast, deterministic, unit-testable; the same code everywhere, including the simulator |
| Radio: iOS | Swift: CoreBluetooth, CarPlay framework | Platform APIs |
| Radio: Android | Kotlin: BLE extended advertising, Wi-Fi Aware, foreground service | Platform APIs |
| Sensors | Native GNSS (raw dual-frequency where available), accelerometer, gyroscope | Better speed and braking estimates than GPS alone |
| Long-range dongle | ESP32 + LoRa module, BLE link to phone, open firmware | Low cost, small, fleet-installable |
| Edge server | Go or Rust service, MQTT over TLS, Redis for live map cells (geohash level 7) | Low latency, scales by city |
| Cloud | PostgreSQL + PostGIS, hosted in-region; certificate authority service | Hazard history, accounts, certificates |
| Localisation | Flutter ARB files: `en`, `km` | Instant language switch |
| Testing | Rust unit tests, SUMO traffic simulator driving the safety core, field test rig | Repeatable scenarios before road tests |

**Repository layout**

```
roadmesh/
  core/            Rust safety core: rsm codec, ldm, tracker, predict, alerts, scoring
  app/             Flutter app: screens, l10n (en, km), state
  ios/             Swift: BLE transport, CarPlay scene, keychain
  android/         Kotlin: BLE + Wi-Fi Aware transport, foreground service
  firmware/        ESP32 + LoRa dongle
  server/          Edge relay, certificate authority, hazard service
  sim/             SUMO scenarios + harness feeding the core
  docs/            This spec, protocol versions, test reports
```

**Team for the MVP (about 6 months):** 1 mobile lead (Flutter), 1 iOS and 1 Android radio engineer, 1 Rust/core engineer, 1 backend engineer, a part-time UX designer, a part-time Khmer localisation reviewer and a field test coordinator.

## 12. MVP scope, milestones, testing and KPIs

The MVP is the smallest app a fleet can pilot: Bluetooth LE mesh, the four peer alerts plus own speeding, behaviour scoring, English and Khmer, and the cellular bridge. Wi-Fi Aware, the LoRa dongle, tailgating and wrong-way detection follow in v1.1.

> **Diagram not included in this export:** MVP roadmap · 4 phases, 3 gates. See the original spec document.

Each gate is a go or no-go decision. Gate 1 matters most: if phones cannot reach about 100 m reliably by Bluetooth in real traffic, the design shifts toward 4G plus dongles before more is built.

**Testing plan**

1. **Unit tests** for the RSM codec, tracker, prediction and every alert rule in the Rust core.
2. **Simulation.** SUMO scenarios (blind junction, hard brake in a platoon, overtake, motorbike filtering, dense rush hour) feed the core with GPS noise of 2 to 10 m and 0 to 40% packet loss.
3. **Closed-course test.** 10 phones on cars and motorbikes, staged conflicts at walking and city speeds.
4. **Fleet pilot.** Real traffic, with drivers rating alerts after each trip.

**KPIs and targets**

| KPI | Target | Measured in |
| --- | --- | --- |
| Direct Bluetooth range, 90% of packets delivered | at least 100 m | Phase 0 |
| Battery use in Drive mode, screen off | under 8% per hour | Phase 0 |
| Alert lead time before conflict point | at least 3 s (median) | Simulation, pilot |
| Missed conflicts in simulation scenarios | under 2% | Phase 1 |
| Unnecessary Danger alerts (driver-rated) | under 1 per 100 km | Phase 2 |
| End-to-end latency, direct / 3 hops | under 150 ms / under 400 ms | Phase 0–2 |
| Weekly active drivers in pilot fleet | above 70% | Phase 2 |
| Hard-braking events per 100 km in pilot fleet | falling month over month | Phase 2 |

## 13. Risks and open questions

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Too few users nearby (network effect) | Peer alerts rarely fire, users leave | Launch with fleets; solo features (hotspot, speed, crash detection) give value from day one |
| Phone GPS error causes false alarms | Drivers ignore or uninstall | Kalman filter, map matching, confidence gate, conservative thresholds, false-alarm KPI |
| iOS background Bluetooth limits | iPhone users broadcast less | Drive mode / CarPlay keep foreground; 4G fallback; measure in field test |
| Battery drain | Uninstalls | Adaptive rates, duty-cycled scanning, 8%/h budget tested on low-end Android phones |
| Spoofed messages | Fake alerts, loss of trust | Signatures, plausibility checks, revocation |
| Driver distraction from the app itself | Causes the crashes it should prevent | Glance-only Drive screen, audio-first alerts, screens locked while moving |
| Liability after a missed or wrong warning | Legal exposure | "Driver assistance, not a guarantee" terms in both languages; legal review |
| Apple refuses the CarPlay entitlement | No in-car screen on iPhone | Phone-mount mode stays fully usable; reapply with a narrower scope |

**Open questions**

- [ ] Which LoRa band and power level does TRC permit, and what does type approval require?
- [ ] Which fleet partner pilots first, and can its drivers' phones run the app (Android version, Bluetooth 5 support)?
- [ ] Does the pilot city's 4G latency support the cellular bridge? Measure it in Phase 0.
- [ ] Which OpenStreetMap coverage gaps in Phnom Penh need fixing before map matching works (lanes, one-way streets)?
- [ ] Business model for v1: free consumer app plus paid fleet dashboard, or insurer and lender partnerships?
- [ ] Who owns Khmer voice recording and string review?
