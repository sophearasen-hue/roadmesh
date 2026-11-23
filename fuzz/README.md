# Fuzzing (AUDIT S-09, S-10)

Every parser of untrusted bytes is fuzzed with [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer).

| Target | What it feeds | What it checks |
|---|---|---|
| `rsm_frame` | Raw radio bytes to `Frame::decode` and `Rsm::decode` | Never panics; every accepted frame re-encodes and decodes to the same body and auth |
| `receive_pipeline` | Raw bytes, or mutations of a validly signed frame, to `Obu::receive_bytes`, then `Obu::decide` | Decode → signature → checks → LDM → relay queue → alerts never panic |
| `pki_parsers` | Raw bytes, or mutations of a valid report / revocation list, to the certificate, envelope, revocation-list and report parsers and to the server's `CertAuthority::report` | Never panics; certificates and envelopes round-trip |

Keys used by the fixtures are generated from the OS random source at start-up. None are stored.

cargo-fuzz needs a nightly compiler. This folder uses a date-pinned nightly and is outside the Cargo workspace, so the product keeps building on stable 1.97.0.

```bash
rustup toolchain install nightly-2026-11-01 --profile minimal
cargo install cargo-fuzz --locked --version 0.13.2
cd fuzz
cargo +nightly-2026-11-01 fuzz run -O receive_pipeline -- -max_total_time=3600
```

- **Every PR:** CI runs each target for 5 minutes (`fuzz` job).
- **Before every release:** each target runs for 24 hours with zero crashes (`docs/RELEASE.md`).
- **A crash:** libFuzzer writes the input to `fuzz/artifacts/<target>/`. Add it as a unit test next to the parser before fixing the bug.
