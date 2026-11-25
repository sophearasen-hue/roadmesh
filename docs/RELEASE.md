# Release checklist

Steps that run before every release, not on every pull request. Each one must pass before a build goes to a store or a pilot.

## Security gates

1. **24-hour fuzz run (AUDIT S-10).** Run each target in `fuzz/` for 24 hours with zero crashes. See `fuzz/README.md` for the commands.

   ```bash
   cd fuzz
   for t in rsm_frame receive_pipeline pki_parsers fragments mqtt server_inputs; do
     cargo +nightly-2026-11-01 fuzz run -O "$t" -- -max_total_time=86400
   done
   ```

   Record the run counts and coverage in the release notes.
2. **Secret scan (AUDIT S-11).** gitleaks over the full history must find nothing. CI runs it on every push.
3. **No test keys in production code (CLAUDE.md).** Software key stores and CA keys exist only in `sim/`, `fuzz/` and test code. Check that `grep -rn "SigningKey" core/src server/src` finds hits only inside `#[cfg(test)]` modules.
4. **Trust anchors.** The CA public keys built into the app match the production KMS/HSM keys (Step 9).

5. **Obfuscated, stripped builds (AUDIT S-12).**
   - Android: R8 is on for release builds.
   - iOS: `Flutter/Release.xcconfig` strips Swift and local symbols.
   - Rust: release builds drop debug info.
   - Build Flutter with obfuscation and keep the symbol files (needed to read crash reports) in the release vault, not in the repo:

   ```bash
   flutter build appbundle --release --obfuscate --split-debug-info=build/symbols/android
   ```

   ```bash
   flutter build ipa --release --obfuscate --split-debug-info=build/symbols/ios
   ```
6. **MobSF.**
   - CI's `mobsf` job (the MobSF rules on the sources) must show no high or warning findings.
   - Also scan the final APK/AAB and IPA in a MobSF server instance before submission: no high findings (AUDIT S-25).
7. **SBOM (AUDIT S-24).**
   - **Rust:** attach CI's `sbom-rust` artifact (CycloneDX).
   - **Flutter:** run `flutter pub deps --json > sbom-flutter.json`.
   - **Native:** list the Android and iOS libraries from the Gradle and SPM lockfiles.
8. **Dependencies.** Merge or decline every open Dependabot pull request; `cargo audit` must be green.
9. **Penetration test (AUDIT S-25).** Before public launch, an independent test of the app, the radio and the server. Keep the report on file.

More items join this list as later steps add them (store forms, privacy manifests, golden screenshots).
