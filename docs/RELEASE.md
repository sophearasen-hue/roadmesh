# Release checklist

Steps that run before every release, not on every pull request. Each one must pass before a build goes to a store or a pilot.

## Security gates

1. **24-hour fuzz run (AUDIT S-10).** Run each target in `fuzz/` for 24 hours with zero crashes. See `fuzz/README.md` for the commands.

   ```bash
   cd fuzz
   for t in rsm_frame receive_pipeline pki_parsers; do
     cargo +nightly-2026-11-01 fuzz run -O "$t" -- -max_total_time=86400
   done
   ```

   Record the run counts and coverage in the release notes.
2. **Secret scan (AUDIT S-11).** gitleaks over the full history must find nothing. CI runs it on every push.
3. **No test keys in production code (CLAUDE.md).** Software key stores and CA keys exist only in `sim/`, `fuzz/` and test code. Check that `grep -rn "SigningKey" core/src server/src` finds hits only inside `#[cfg(test)]` modules.
4. **Trust anchors.** The CA public keys built into the app match the production KMS/HSM keys (Step 9).

More items join this list as later steps add them (store forms, privacy manifests, golden screenshots).
