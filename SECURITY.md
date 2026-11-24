# Security policy

RoadMesh warns drivers about collisions, so a security flaw can put people at risk. Please tell us privately.

## Reporting a vulnerability

- **Email:** `<security contact: set by the project owner before release>`. Encrypt with the PGP key published in the project's `/.well-known/security.txt`.
- **GitHub:** alternatively, open a private advisory under Security → Report a vulnerability.
- **What to include:** what you found, how to reproduce it, and the app or server version.
- **What not to include:** do not test on public roads, and do not send messages that could cause a real driver to brake.

We answer within 2 working days. Critical issues are fixed within 7 days (AUDIT S-26). We credit reporters who want credit once the fix is out.

## In scope

- The iPhone and Android apps.
- The Bluetooth messages and the relay protocol (SPEC §3–§5).
- The edge server and the certificate authority (docs/SERVER.md).
- This repository.

## Out of scope

- Denial of service by radio jamming. Jamming is physically always possible, and the app fails safe: no warnings, never false braking.
- Social engineering of staff.
- Findings that need a rooted or jailbroken phone to read that phone's own data.

## How the system protects itself

The design is in docs/SPEC.md §10, and the checklist is section B of docs/AUDIT.md. In short:

- **Signing:** every message is signed with a hardware key.
- **Certificates:** they rotate every 5 minutes and are issued only to genuine apps.
- **Revocation:** a sender is revoked after 3 independent reports.
- **Parsing:** every received byte is parsed in memory-safe, fuzzed Rust.
- **Server traffic:** TLS 1.3 with pinning and mutual TLS.
