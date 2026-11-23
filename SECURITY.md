# Security policy

RoadMesh is a road safety system. A security flaw can put people in danger, so we treat reports as urgent.

## Reporting a vulnerability

- Email: security@[your-domain] (set this up before the public launch)
- Please include: what you found, steps to reproduce, and the app version.
- Do not test against other people's phones or on public roads.
- We will confirm receipt within 2 working days.

## Our commitments

| Severity | Example | Fix target |
|---|---|---|
| Critical | Forged alerts accepted, key extraction, remote code execution | 7 days |
| High | Tracking of a user across pseudonyms, certificate abuse | 30 days |
| Medium / Low | Other issues | Next release |

We credit reporters who wish to be named, and we will not take legal action against good-faith research that follows this policy.

## Scope

In scope: the iOS and Android apps, the radio message protocol, the edge and certificate servers.
Out of scope: the `reference/` simulators (they use demo-only cryptography by design).
