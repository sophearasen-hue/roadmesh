# Incident response (AUDIT S-27)

**Goal:** stop harm to drivers first, then fix and explain. Keep a written timeline of every incident, kept for one year with the audit logs.

## Roles

| Role | Who | Decides |
|---|---|---|
| Incident lead | Project owner (or a named deputy) | Severity, fleet-wide revocation, forced update, public statement |
| Server on-call | Whoever holds the AWS on-call role | Contain on the server: rate limits, block devices, roll back a deploy |
| Second approver | A second named person | Required for any CA key-policy change (AUDIT S-19, two-person rule) |

Fill in names and phone numbers before the pilot. Keep the list in the team's password manager, not in this repository.

## Severity

| Level | Examples | Response time |
|---|---|---|
| **Critical** | False Danger alerts spread by forged or relayed messages; CA or device key compromise; a flaw that lets a phone send messages under others' pseudonyms; personal data exposed | Start within 1 hour; fix within 7 days (S-26) |
| **High** | A crash anyone in radio range can trigger; server down for more than 1 hour; a misbehaviour-report flood | Same day |
| **Medium / Low** | Everything else | Next release |

The alerts that start an incident:
- **Logged alerts:** CloudWatch `ALERT` lines (docs/SERVER.md), such as abnormal certificate requests, report spikes and rejected uploads.
- **Reports from outside:** reports to the security contact (SECURITY.md).
- **Crash spikes:** crash-reporter spikes after a release.

## Playbooks

**1. Revoke one device or a group of devices.**
- **Normal path:** misbehaviour reports from 3 devices revoke a device automatically (SPEC §10). Then the server pushes the new revocation list to every connected phone; phones offline get it at their next connection, and the list is re-signed at least every 30 minutes.
- **Manual revocation:** the incident lead revokes by device id (the 8-byte reference in the audit log).
  - Run `CertAuthority::revoke_device` through the admin task, with two people present.
  - Record the reason.

**2. Revoke every certificate (CA compromise or a flaw in issued certificates).**
1. **Pick the next key id.** Choose a new CA key id: `RM_CA_KEY_ID`, one higher than the current one.
2. **Create a new key.** Create a new KMS key under the two-person rule.
3. **Restart the servers on it.** The new trust anchor is printed at start-up.
4. **Stop the old key.** Disable the old KMS key. Phones refuse messages signed under the old key once they install the update below. Until then, revoke all old certificates through the revocation list (playbook 1, for all devices).
5. **Ship the new anchor.** Release an app update with the new anchor (playbook 3). Phones then re-enrol automatically.

**3. Force an update.**
- **Store releases.** Publish the fixed build on both stores with expedited review: Apple's "Request an Expedited App Review" and Google Play's managed publishing.
- **Server-side block.**
  - The enrolment port refuses requests from old app versions. Play Integrity reports `versionCode`; App Attest renewals go through the new build.
  - Phones that cannot renew lose their certificates within a week. They then stop sending but still receive and warn (fail-safe).

**4. Server compromise.**
- Isolate the instance, keeping disk and memory for analysis.
- Deploy from a clean image.
- Rotate the server's TLS key to the backup pin. The app ships two pins, so phones accept the backup at once.
- Review the KMS CloudTrail for `Sign` calls not made by the server role.

**5. Personal data exposure.** Within 72 hours, tell the affected users and the authorities required by Cambodian law. What the server holds is limited (docs/SERVER.md): device certificates and report counts, no positions.

## After every incident

Within 2 weeks:
- **Review:** a blameless review: timeline, cause, what detected it, what slowed the response.
- **Prevention:** add a test or a fuzz target so the same flaw cannot return.
- **Records:** update this plan and AUDIT_REPORT.md.
