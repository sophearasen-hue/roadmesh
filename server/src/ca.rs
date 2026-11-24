//! Certificate authority logic (SPEC §10, AUDIT S-04, S-05, S-06): pseudonym
//! certificate issuance with a weekly limit, misbehaviour reports, revocation
//! after reports from 3 independent devices, and the signed revocation list.
//!
//! Logic only: no network and no key material. The CA private key is reached
//! through [`CaKey`], implemented in Step 9 with a cloud KMS/HSM; device
//! genuineness through [`Attestation`] (Apple App Attest / Google Play Integrity,
//! Step 9). Until then [`RejectAll`] is the only attestation in this crate, so
//! nothing can be issued in production by accident.

use std::collections::{BTreeSet, HashMap, HashSet};

use roadmesh_core::config::{Config, Security};
use roadmesh_core::pki::{
    self, AuthEnvelope, CertDigest, Certificate, MisbehaviourReport, PUBLIC_KEY_LEN,
    RevocationList, SIG_LEN,
};

/// Identifies a device: SHA-256 of its device public key. Server-side only, never broadcast.
pub type DeviceId = [u8; 32];

/// The CA signing key. Production: a KMS/HSM key that never leaves the HSM (Step 9).
pub trait CaKey {
    /// Key id written into certificates and revocation lists.
    fn key_id(&self) -> u8;
    /// SEC1 compressed public key (shipped in the app as a trust anchor).
    fn public_key(&self) -> [u8; PUBLIC_KEY_LEN];
    /// ECDSA P-256 / SHA-256 signature (r‖s) over `message`.
    fn sign(&self, message: &[u8]) -> Result<[u8; SIG_LEN], CaError>;
}

/// Proof that a request comes from the genuine RoadMesh app on a real device.
pub trait Attestation {
    /// True if `evidence` proves `device_key` belongs to a genuine app install.
    fn verify(&self, device_key: &[u8; PUBLIC_KEY_LEN], evidence: &[u8]) -> bool;
}

/// Accepts no device. The default until App Attest / Play Integrity are built (Step 9).
#[derive(Debug, Clone, Copy, Default)]
pub struct RejectAll;

impl Attestation for RejectAll {
    fn verify(&self, _: &[u8; PUBLIC_KEY_LEN], _: &[u8]) -> bool {
        false
    }
}

/// The CA key could not sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaError {
    /// KMS/HSM unavailable.
    Unavailable,
}

/// A phone asks for pseudonym certificates. Every pseudonym key is generated in the
/// phone's secure hardware; the request is signed by the device key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRequest {
    /// The install's device public key.
    pub device_key: [u8; PUBLIC_KEY_LEN],
    /// App Attest / Play Integrity evidence for `device_key`.
    pub attestation: Vec<u8>,
    /// One public key per requested certificate.
    pub pseudonym_keys: Vec<[u8; PUBLIC_KEY_LEN]>,
    /// When the phone made the request (Unix s).
    pub requested_at: u32,
    /// Device-key signature over [`IssueRequest::to_be_signed`].
    pub signature: [u8; SIG_LEN],
}

impl IssueRequest {
    /// What the device key signs.
    pub fn to_be_signed(&self) -> Vec<u8> {
        pki::issue_tbs(&self.device_key, self.requested_at, &self.pseudonym_keys)
    }

    /// Parse a request received from a phone (untrusted bytes, wire format of
    /// [`pki::IssueRequestWire`]).
    pub fn decode(b: &[u8]) -> Result<Self, IssueError> {
        let w = pki::IssueRequestWire::decode(b).map_err(|_| IssueError::BadRequest)?;
        Ok(Self {
            device_key: w.device_key,
            attestation: w.attestation,
            pseudonym_keys: w.pseudonym_keys,
            requested_at: w.requested_at,
            signature: w.signature,
        })
    }
}

/// Why certificates were not issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueError {
    /// No keys, too many keys, or an invalid key.
    BadRequest,
    /// The device signature does not verify.
    BadSignature,
    /// Request time too far from the server clock (replayed request).
    StaleRequest,
    /// The device could not prove it is genuine.
    AttestationFailed,
    /// The device was revoked for misbehaviour.
    Banned,
    /// More than the weekly limit; this many may still be issued now.
    QuotaExceeded {
        /// Certificates still available in the window.
        remaining: usize,
    },
    /// A pseudonym key was used before (or equals the device key): pseudonyms would be linkable.
    ReusedKey,
    /// The CA key could not sign.
    Ca(CaError),
}

/// Why a report was not counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportError {
    /// Bytes did not parse.
    Malformed,
    /// The reporter's certificate is not one this CA issued, or is revoked or expired.
    UnknownReporter,
    /// The reporter's signature does not verify.
    BadSignature,
    /// An evidence frame is not validly signed by a certificate this CA issued,
    /// or the frames come from different certificates.
    BadEvidence,
    /// The evidence does not show the misbehaviour.
    Unconfirmed,
    /// A device reporting itself.
    SelfReport,
    /// The reporter has sent too many reports today.
    RateLimited,
}

/// What a counted report did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportOutcome {
    /// Counted; this many independent devices have now reported the accused device.
    Counted(usize),
    /// This report reached the threshold: every certificate of the device is revoked.
    Revoked,
    /// The device was already revoked.
    AlreadyRevoked,
}

#[derive(Debug, Clone, Default)]
struct Device {
    issued: Vec<(u32, CertDigest)>,
    banned: bool,
    reports_sent: Vec<u32>,
}

#[derive(Debug, Clone)]
struct Issued {
    device: DeviceId,
    cert: Certificate,
}

/// The certificate authority.
#[derive(Debug)]
pub struct CertAuthority<K, A> {
    key: K,
    attestation: A,
    cfg: Config,
    devices: HashMap<DeviceId, Device>,
    certs: HashMap<CertDigest, Issued>,
    used_keys: HashSet<[u8; PUBLIC_KEY_LEN]>,
    reporters: HashMap<DeviceId, HashSet<DeviceId>>,
    revoked: BTreeSet<CertDigest>,
    crl_serial: u32,
}

fn device_id(key: &[u8; PUBLIC_KEY_LEN]) -> DeviceId {
    pki::sha256(&[key])
}

impl<K: CaKey, A: Attestation> CertAuthority<K, A> {
    /// New CA. `cfg` supplies the misbehaviour thresholds and `cfg.security` the limits.
    pub fn new(key: K, attestation: A, cfg: Config) -> Self {
        Self {
            key,
            attestation,
            cfg,
            devices: HashMap::new(),
            certs: HashMap::new(),
            used_keys: HashSet::new(),
            reporters: HashMap::new(),
            revoked: BTreeSet::new(),
            crl_serial: 0,
        }
    }

    fn sec(&self) -> &Security {
        &self.cfg.security
    }

    /// The trust anchor phones ship with.
    pub fn trust_anchor(&self) -> pki::TrustAnchor {
        pki::TrustAnchor {
            key_id: self.key.key_id(),
            public_key: self.key.public_key(),
        }
    }

    /// Issue pseudonym certificates (at most `certs_per_window` per device per week).
    pub fn issue(&mut self, req: &IssueRequest, now: u32) -> Result<Vec<Certificate>, IssueError> {
        let sec = self.sec().clone();
        let n = req.pseudonym_keys.len();
        if n == 0 || n > sec.certs_per_window {
            return Err(IssueError::BadRequest);
        }
        if now.abs_diff(req.requested_at) > sec.issue_request_max_age_s {
            return Err(IssueError::StaleRequest);
        }
        let dk = pki::parse_public_key(&req.device_key).map_err(|_| IssueError::BadRequest)?;
        if !pki::verify_signature(&dk, &req.to_be_signed(), &req.signature) {
            return Err(IssueError::BadSignature);
        }
        if !self.attestation.verify(&req.device_key, &req.attestation) {
            return Err(IssueError::AttestationFailed);
        }
        let id = device_id(&req.device_key);
        let dev = self.devices.entry(id).or_default();
        if dev.banned {
            return Err(IssueError::Banned);
        }
        let window_start = now.saturating_sub(sec.issue_window_s);
        dev.issued.retain(|(t, _)| *t > window_start);
        let remaining = sec.certs_per_window.saturating_sub(dev.issued.len());
        if n > remaining {
            return Err(IssueError::QuotaExceeded { remaining });
        }
        let mut seen = HashSet::new();
        for k in &req.pseudonym_keys {
            pki::parse_public_key(k).map_err(|_| IssueError::BadRequest)?;
            if *k == req.device_key || self.used_keys.contains(k) || !seen.insert(*k) {
                return Err(IssueError::ReusedKey);
            }
        }
        let mut out = Vec::with_capacity(n);
        for k in &req.pseudonym_keys {
            let mut c = Certificate {
                ca_key_id: self.key.key_id(),
                valid_from: now,
                valid_until: now.saturating_add(sec.cert_validity_s),
                public_key: *k,
                ca_signature: [0; SIG_LEN],
            };
            c.ca_signature = self.key.sign(&c.to_be_signed()).map_err(IssueError::Ca)?;
            out.push(c);
        }
        let dev = self.devices.entry(id).or_default();
        for c in &out {
            let d = c.digest();
            dev.issued.push((now, d));
            self.used_keys.insert(c.public_key);
            self.certs.insert(
                d,
                Issued {
                    device: id,
                    cert: *c,
                },
            );
        }
        Ok(out)
    }

    /// Handle a misbehaviour report (untrusted bytes). The device is revoked once
    /// `revoke_after_reporters` different devices have filed confirmed reports.
    pub fn report(&mut self, bytes: &[u8], now: u32) -> Result<ReportOutcome, ReportError> {
        let sec = self.sec().clone();
        let r = MisbehaviourReport::decode(bytes, sec.report_max_evidence)
            .map_err(|_| ReportError::Malformed)?;
        // the reporter: a certificate we issued, not revoked, valid now
        let rd = r.reporter.digest();
        let reporter = match self.certs.get(&rd) {
            Some(i) if i.cert == r.reporter && !self.revoked.contains(&rd) => i.device,
            _ => return Err(ReportError::UnknownReporter),
        };
        if !r.reporter.valid_at(f64::from(now), sec.clock_skew_s) {
            return Err(ReportError::UnknownReporter);
        }
        let rkey =
            pki::parse_public_key(&r.reporter.public_key).map_err(|_| ReportError::BadSignature)?;
        let tbs = r.to_be_signed().ok_or(ReportError::Malformed)?;
        if !pki::verify_signature(&rkey, &tbs, &r.signature) {
            return Err(ReportError::BadSignature);
        }
        if self.devices.get(&reporter).is_some_and(|d| d.banned) {
            return Err(ReportError::UnknownReporter);
        }
        // evidence: every frame validly signed by one certificate we issued
        let observed = f64::from(r.observed_at);
        let mut accused_digest = None;
        for f in &r.evidence {
            let env = AuthEnvelope::decode(&f.auth).map_err(|_| ReportError::BadEvidence)?;
            let d = env.cert.digest();
            if accused_digest.is_some_and(|a| a != d) {
                return Err(ReportError::BadEvidence);
            }
            accused_digest = Some(d);
            let issued = self.certs.get(&d).ok_or(ReportError::BadEvidence)?;
            let key = pki::parse_public_key(&issued.cert.public_key)
                .map_err(|_| ReportError::BadEvidence)?;
            if pki::body_temp_id(&f.body) != pki::temp_id_of(&d)
                || !pki::verify_rsm_signature(&key, observed, &f.body, &env.signature)
            {
                return Err(ReportError::BadEvidence);
            }
        }
        let d = accused_digest.ok_or(ReportError::BadEvidence)?;
        let accused = self
            .certs
            .get(&d)
            .map(|i| i.device)
            .ok_or(ReportError::BadEvidence)?;
        if accused == reporter {
            return Err(ReportError::SelfReport);
        }
        if r.reason.checkable_from_evidence() {
            match pki::evidence_shows(&r.evidence, observed, &self.cfg) {
                Ok(Some(_)) => {}
                _ => return Err(ReportError::Unconfirmed),
            }
        }
        // rate limit per reporting device (counted only for reports that passed all checks)
        let day_start = now.saturating_sub(86_400);
        let rdev = self.devices.entry(reporter).or_default();
        rdev.reports_sent.retain(|t| *t > day_start);
        if rdev.reports_sent.len() >= sec.reports_per_device_per_day {
            return Err(ReportError::RateLimited);
        }
        rdev.reports_sent.push(now);
        if self.devices.get(&accused).is_some_and(|d| d.banned) {
            return Ok(ReportOutcome::AlreadyRevoked);
        }
        let who = self.reporters.entry(accused).or_default();
        who.insert(reporter);
        let count = who.len();
        if count >= sec.revoke_after_reporters {
            self.revoke_device(&accused);
            return Ok(ReportOutcome::Revoked);
        }
        Ok(ReportOutcome::Counted(count))
    }

    /// Revoke every certificate of a device and refuse it new ones.
    pub fn revoke_device(&mut self, device: &DeviceId) {
        let dev = self.devices.entry(*device).or_default();
        dev.banned = true;
        let mut changed = false;
        for (d, i) in &self.certs {
            if i.device == *device {
                changed |= self.revoked.insert(*d);
            }
        }
        if changed {
            self.crl_serial = self.crl_serial.saturating_add(1);
        }
    }

    /// Device behind a certificate (server-side only).
    pub fn device_of(&self, d: &CertDigest) -> Option<DeviceId> {
        self.certs.get(d).map(|i| i.device)
    }

    /// Is this certificate revoked?
    pub fn is_revoked(&self, d: &CertDigest) -> bool {
        self.revoked.contains(d)
    }

    /// The signed revocation list: revoked certificates that have not expired yet.
    pub fn revocation_list(&self, now: u32, next_update: u32) -> Result<Vec<u8>, CaError> {
        let revoked: Vec<CertDigest> = self
            .revoked
            .iter()
            .filter(|d| {
                self.certs
                    .get(*d)
                    .is_some_and(|i| i.cert.valid_until >= now)
            })
            .copied()
            .collect(); // BTreeSet: already strictly ascending
        let list = RevocationList {
            ca_key_id: self.key.key_id(),
            serial: self.crl_serial,
            issued_at: now,
            next_update,
            revoked,
        };
        let sig = self.key.sign(&list.to_be_signed())?;
        Ok(list.encode_signed(&sig))
    }
}
