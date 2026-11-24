//! Pseudonym certificates, the signed-message envelope, the revocation list and
//! misbehaviour reports (SPEC §10, AUDIT S-01 to S-06).
//!
//! The core never holds a private key. Signing goes through [`KeyStore`], which the
//! platform implements with keys generated in the Secure Enclave / Android Keystore
//! (AUDIT S-02). Verification is pure computation and lives here.
//!
//! # Certificate (107 bytes, big-endian)
//!
//! | off | bytes | field |
//! |----:|----:|---|
//! | 0 | 1 | version = 1 |
//! | 1 | 1 | ca_key_id (which CA key signed it; allows CA key rotation) |
//! | 2 | 4 | valid_from, Unix seconds |
//! | 6 | 4 | valid_until, Unix seconds |
//! | 10 | 33 | pseudonym public key, SEC1 compressed P-256 point |
//! | 43 | 64 | CA signature (ECDSA P-256 / SHA-256, r‖s) over `DOMAIN_CERT ‖ bytes 0..43` |
//!
//! The certificate digest is the first 8 bytes of SHA-256 over all 107 bytes; the
//! message `temp_id` is the first 4 bytes of the digest, so a pseudonym is bound
//! to exactly one certificate.
//!
//! # Message envelope (the frame's `auth` field)
//!
//! `[kind:1][signature:64][digest:8 | certificate:107]`, kind 1 = digest (73 bytes),
//! kind 2 = full certificate (172 bytes). The signature covers
//! `DOMAIN_RSM ‖ hour:4 ‖ signing_bytes(body)`, where `hour` is the sender's
//! Unix hour. The body carries only milliseconds since the hour; binding the hour
//! into the signature means a recorded message replayed an hour (or a week) later
//! no longer verifies (AUDIT S-03).
//!
//! # Revocation list
//!
//! `[version:1][ca_key_id:1][serial:4][issued_at:4][next_update:4][count:4][digest:8]*count[signature:64]`,
//! digests strictly ascending; the CA signs `DOMAIN_CRL ‖ everything before the signature`.
//!
//! # Misbehaviour report
//!
//! `[version:1][reason:1][observed_at:4][n:1]([len:1][frame:len])*n[reporter certificate:107][signature:64]`,
//! signed by the reporter's current pseudonym key over `DOMAIN_REPORT ‖ everything before the signature`.

use std::collections::HashMap;

use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::auth::{AuthError, Signer, Verifier};
use crate::config::{Config, Security};
use crate::geo::GeoOrigin;
use crate::ldm::{Check, Ldm, Misbehaviour, PeerMsg};
use crate::rsm::{self, BODY_LEN, Frame, MS_PER_HOUR, Rsm, TempId};

/// First 8 bytes of SHA-256 over an encoded certificate.
pub type CertDigest = [u8; 8];

/// Certificate format version.
pub const CERT_VERSION: u8 = 1;
/// Encoded certificate length.
pub const CERT_LEN: usize = 107;
/// SEC1 compressed P-256 public key length.
pub const PUBLIC_KEY_LEN: usize = 33;
/// Raw ECDSA P-256 signature length (r‖s).
pub const SIG_LEN: usize = 64;
const CERT_TBS_LEN: usize = 43;

/// Envelope kind: signature + certificate digest.
pub const AUTH_KIND_DIGEST: u8 = 1;
/// Envelope kind: signature + full certificate.
pub const AUTH_KIND_FULL: u8 = 2;
/// Envelope length with a digest.
pub const AUTH_DIGEST_LEN: usize = 1 + SIG_LEN + 8;
/// Envelope length with a full certificate.
pub const AUTH_FULL_LEN: usize = 1 + SIG_LEN + CERT_LEN;

/// Domain separation: what a signature is for. A key that signs messages can
/// never be tricked into producing a valid certificate, list or report.
pub const DOMAIN_RSM: &[u8; 16] = b"RoadMesh RSM v0\0";
/// See [`DOMAIN_RSM`].
pub const DOMAIN_CERT: &[u8; 16] = b"RoadMesh CERT v1";
/// See [`DOMAIN_RSM`].
pub const DOMAIN_CRL: &[u8; 16] = b"RoadMesh CRL v1\0";
/// See [`DOMAIN_RSM`].
pub const DOMAIN_REPORT: &[u8; 16] = b"RoadMesh RPT v1\0";
/// See [`DOMAIN_RSM`]. Used by the server's issuance request.
pub const DOMAIN_ISSUE: &[u8; 16] = b"RoadMesh ISS v1\0";

const CRL_VERSION: u8 = 1;
const CRL_HEADER_LEN: usize = 18;
const REPORT_VERSION: u8 = 1;

/// Why certificate material, a revocation list or a report was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkiError {
    /// Wrong length or truncated.
    Length,
    /// Unknown format version.
    Version,
    /// Not a valid P-256 public key.
    BadKey,
    /// Signature does not verify.
    BadSignature,
    /// Signed by a CA key this phone does not trust.
    UnknownCa,
    /// Revocation list entries not strictly ascending.
    NotSorted,
    /// More entries than allowed.
    TooMany,
    /// Revocation list older than the one already loaded.
    Rollback,
    /// Evidence is empty, from several senders, or shows no misbehaviour.
    BadEvidence,
}

/// SHA-256.
pub fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// Verify an ECDSA P-256 / SHA-256 signature (r‖s) over `message`.
pub fn verify_signature(key: &VerifyingKey, message: &[u8], sig: &[u8; SIG_LEN]) -> bool {
    match Signature::from_slice(sig) {
        Ok(s) => key.verify(message, &s).is_ok(),
        Err(_) => false,
    }
}

/// Parse a SEC1 compressed P-256 public key.
pub fn parse_public_key(bytes: &[u8; PUBLIC_KEY_LEN]) -> Result<VerifyingKey, PkiError> {
    if bytes[0] != 0x02 && bytes[0] != 0x03 {
        return Err(PkiError::BadKey);
    }
    VerifyingKey::from_sec1_bytes(bytes).map_err(|_| PkiError::BadKey)
}

// ---------------------------------------------------------------- certificate

/// A pseudonym certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Certificate {
    /// Which CA key signed it.
    pub ca_key_id: u8,
    /// Start of validity (Unix s).
    pub valid_from: u32,
    /// End of validity (Unix s).
    pub valid_until: u32,
    /// Pseudonym public key, SEC1 compressed.
    pub public_key: [u8; PUBLIC_KEY_LEN],
    /// CA signature over [`Certificate::to_be_signed`].
    pub ca_signature: [u8; SIG_LEN],
}

impl Certificate {
    fn tbs_fields(&self) -> [u8; CERT_TBS_LEN] {
        let mut b = [0u8; CERT_TBS_LEN];
        b[0] = CERT_VERSION;
        b[1] = self.ca_key_id;
        b[2..6].copy_from_slice(&self.valid_from.to_be_bytes());
        b[6..10].copy_from_slice(&self.valid_until.to_be_bytes());
        b[10..43].copy_from_slice(&self.public_key);
        b
    }

    /// The bytes the CA signs.
    pub fn to_be_signed(&self) -> Vec<u8> {
        [DOMAIN_CERT.as_slice(), &self.tbs_fields()].concat()
    }

    /// Encode (107 bytes).
    pub fn encode(&self) -> [u8; CERT_LEN] {
        let mut b = [0u8; CERT_LEN];
        b[..CERT_TBS_LEN].copy_from_slice(&self.tbs_fields());
        b[CERT_TBS_LEN..].copy_from_slice(&self.ca_signature);
        b
    }

    /// Parse untrusted bytes (exactly one certificate). Does not check the CA signature.
    pub fn decode(b: &[u8]) -> Result<Self, PkiError> {
        if b.len() != CERT_LEN {
            return Err(PkiError::Length);
        }
        if b[0] != CERT_VERSION {
            return Err(PkiError::Version);
        }
        let u32_at = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let mut public_key = [0u8; PUBLIC_KEY_LEN];
        public_key.copy_from_slice(&b[10..43]);
        let mut ca_signature = [0u8; SIG_LEN];
        ca_signature.copy_from_slice(&b[43..]);
        Ok(Self {
            ca_key_id: b[1],
            valid_from: u32_at(2),
            valid_until: u32_at(6),
            public_key,
            ca_signature,
        })
    }

    /// First 8 bytes of SHA-256 over the encoded certificate.
    pub fn digest(&self) -> CertDigest {
        let h = sha256(&[&self.encode()]);
        let mut d = [0u8; 8];
        d.copy_from_slice(&h[..8]);
        d
    }

    /// The pseudonym this certificate stands for (first 4 digest bytes).
    pub fn temp_id(&self) -> TempId {
        temp_id_of(&self.digest())
    }

    /// Valid at `now` (Unix s), allowing `skew` seconds of clock difference.
    pub fn valid_at(&self, now: f64, skew: f64) -> bool {
        f64::from(self.valid_from) - skew <= now && now <= f64::from(self.valid_until) + skew
    }

    /// Check the CA signature and the public key.
    pub fn verify_issuer(&self, ca: &VerifyingKey) -> Result<VerifyingKey, PkiError> {
        if !verify_signature(ca, &self.to_be_signed(), &self.ca_signature) {
            return Err(PkiError::BadSignature);
        }
        parse_public_key(&self.public_key)
    }
}

/// The pseudonym of a certificate digest.
pub fn temp_id_of(d: &CertDigest) -> TempId {
    [d[0], d[1], d[2], d[3]]
}

/// A CA public key the app trusts (shipped with the app; public, not a secret).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustAnchor {
    /// CA key id, as in certificates.
    pub key_id: u8,
    /// SEC1 compressed public key.
    pub public_key: [u8; PUBLIC_KEY_LEN],
}

// ---------------------------------------------------------------- message envelope

/// How the envelope names the signing certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertRef {
    /// Digest of a certificate the receiver should already have.
    Digest(CertDigest),
    /// The full certificate.
    Full(Certificate),
}

impl CertRef {
    /// Digest of the referenced certificate.
    pub fn digest(&self) -> CertDigest {
        match self {
            Self::Digest(d) => *d,
            Self::Full(c) => c.digest(),
        }
    }
}

/// The frame's `auth` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthEnvelope {
    /// ECDSA signature (r‖s) over [`rsm_to_be_signed`].
    pub signature: [u8; SIG_LEN],
    /// Signing certificate.
    pub cert: CertRef,
}

impl AuthEnvelope {
    /// Encode.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(AUTH_FULL_LEN);
        match &self.cert {
            CertRef::Digest(d) => {
                out.push(AUTH_KIND_DIGEST);
                out.extend_from_slice(&self.signature);
                out.extend_from_slice(d);
            }
            CertRef::Full(c) => {
                out.push(AUTH_KIND_FULL);
                out.extend_from_slice(&self.signature);
                out.extend_from_slice(&c.encode());
            }
        }
        out
    }

    /// Parse untrusted bytes.
    pub fn decode(b: &[u8]) -> Result<Self, AuthError> {
        let (&kind, rest) = b.split_first().ok_or(AuthError::MalformedAuth)?;
        if rest.len() < SIG_LEN {
            return Err(AuthError::MalformedAuth);
        }
        let (sig, tail) = rest.split_at(SIG_LEN);
        let mut signature = [0u8; SIG_LEN];
        signature.copy_from_slice(sig);
        let cert = match (kind, tail.len()) {
            (AUTH_KIND_DIGEST, 8) => {
                let mut d = [0u8; 8];
                d.copy_from_slice(tail);
                CertRef::Digest(d)
            }
            (AUTH_KIND_FULL, CERT_LEN) => {
                CertRef::Full(Certificate::decode(tail).map_err(|_| AuthError::MalformedAuth)?)
            }
            _ => return Err(AuthError::MalformedAuth),
        };
        Ok(Self { signature, cert })
    }
}

/// The sender's Unix hour for a message with `timestamp_ms` (ms since the hour)
/// received at `now_s` (Unix s): the hour that puts the timestamp nearest to now.
pub fn sender_hour(now_s: f64, timestamp_ms: u32) -> u32 {
    let now_ms = (now_s * 1000.0).round() as i64;
    let age_ms = (rsm::age_s(rsm::ms_of_hour(now_s), timestamp_ms) * 1000.0).round() as i64;
    let sent_ms = (now_ms - age_ms).max(0);
    (sent_ms / i64::from(MS_PER_HOUR)).min(i64::from(u32::MAX)) as u32
}

/// Timestamp field of an encoded body.
fn body_timestamp_ms(body: &[u8; BODY_LEN]) -> u32 {
    u32::from_be_bytes([body[7], body[8], body[9], body[10]])
}

/// Temp_id field of an encoded body.
pub fn body_temp_id(body: &[u8; BODY_LEN]) -> TempId {
    [body[1], body[2], body[3], body[4]]
}

/// What the originator signs: `DOMAIN_RSM ‖ hour ‖ signing_bytes(body)`.
pub fn rsm_to_be_signed(hour: u32, body: &[u8; BODY_LEN]) -> [u8; 16 + 4 + BODY_LEN] {
    let mut b = [0u8; 16 + 4 + BODY_LEN];
    b[..16].copy_from_slice(DOMAIN_RSM);
    b[16..20].copy_from_slice(&hour.to_be_bytes());
    b[20..].copy_from_slice(&rsm::signing_bytes(body));
    b
}

/// Verify a message signature with a known certificate key.
pub fn verify_rsm_signature(
    key: &VerifyingKey,
    now: f64,
    body: &[u8; BODY_LEN],
    signature: &[u8; SIG_LEN],
) -> bool {
    let hour = sender_hour(now, body_timestamp_ms(body));
    verify_signature(key, &rsm_to_be_signed(hour, body), signature)
}

// ---------------------------------------------------------------- signing (phone side)

/// The phone's secure key storage. Implemented by the platform with hardware keys
/// (Secure Enclave / Android Keystore, StrongBox where available); the private key
/// never leaves it (AUDIT S-02). Simulator and tests use software keys.
pub trait KeyStore {
    /// ECDSA P-256 / SHA-256 signature (r‖s) over `message` with the private key
    /// of the certificate `digest`. `None` if that key is not available.
    fn sign(&mut self, digest: &CertDigest, message: &[u8]) -> Option<[u8; SIG_LEN]>;
}

/// Rotates through this phone's pseudonym certificates and signs with the
/// matching hardware key. Decides when the full certificate is attached.
#[derive(Debug, Clone)]
pub struct PseudonymSigner<K> {
    certs: Vec<Certificate>,
    keys: K,
    sec: Security,
    current: Option<CertDigest>,
    rotated_at: f64,
    last_full: f64,
    rotation_allowed: bool,
}

impl<K: KeyStore> PseudonymSigner<K> {
    /// Signer over a batch of certificates whose private keys are in `keys`.
    pub fn new(certs: Vec<Certificate>, keys: K, sec: &Security) -> Self {
        let mut s = Self {
            certs: Vec::new(),
            keys,
            sec: sec.clone(),
            current: None,
            rotation_allowed: true,
            rotated_at: f64::NEG_INFINITY,
            last_full: f64::NEG_INFINITY,
        };
        s.add_certificates(certs);
        s
    }

    /// Add newly issued certificates (keeps a stable order).
    pub fn add_certificates(&mut self, certs: Vec<Certificate>) {
        self.certs.extend(certs);
        self.certs.sort_by_key(|c| (c.valid_from, c.digest()));
        self.certs.dedup();
    }

    /// Forget expired certificates (their keys should be deleted from the key store too).
    pub fn remove_expired(&mut self, now: f64) -> Vec<CertDigest> {
        let (old, keep): (Vec<_>, Vec<_>) = self
            .certs
            .iter()
            .partition(|c| f64::from(c.valid_until) < now);
        self.certs = keep;
        old.iter().map(|c| c.digest()).collect()
    }

    /// Certificates held.
    pub fn certificates(&self) -> &[Certificate] {
        &self.certs
    }

    /// The key store.
    pub fn keys_mut(&mut self) -> &mut K {
        &mut self.keys
    }

    /// The certificate in use at `now`: rotates every `pseudonym_rotate_s` through the valid ones.
    pub fn certificate_at(&self, now: f64) -> Option<Certificate> {
        let valid: Vec<&Certificate> = self.certs.iter().filter(|c| c.valid_at(now, 0.0)).collect();
        if valid.is_empty() {
            return None;
        }
        let slot = (now / self.sec.pseudonym_rotate_s).floor().max(0.0) as u64;
        valid.get((slot % valid.len() as u64) as usize).map(|c| **c)
    }

    /// AUDIT S-13: whether switching pseudonym is allowed now (the platform sets it: the
    /// vehicle is stopped or at a junction). While not allowed, the pseudonym in use is kept,
    /// for at most `rotation_max_delay_s` beyond its scheduled end.
    pub fn set_rotation_allowed(&mut self, allowed: bool) {
        self.rotation_allowed = allowed;
    }

    /// The certificate actually used at `now`: the scheduled one, unless the switch must wait.
    pub fn active_certificate(&self, now: f64) -> Option<Certificate> {
        let scheduled = self.certificate_at(now)?;
        if let Some(d) = self.current
            && d != scheduled.digest()
            && !self.rotation_allowed
            && let Some(cur) = self
                .certs
                .iter()
                .find(|c| c.digest() == d && c.valid_at(now, 0.0))
            && now - self.rotated_at
                < self.sec.pseudonym_rotate_s + self.sec.rotation_max_delay_s - 1e-9
        {
            return Some(*cur);
        }
        Some(scheduled)
    }

    /// Sign `payload` (already domain-prefixed) with the current pseudonym key.
    pub fn sign_with_current(
        &mut self,
        now: f64,
        payload: &[u8],
    ) -> Option<(Certificate, [u8; SIG_LEN])> {
        let cert = self.active_certificate(now)?;
        let sig = self.keys.sign(&cert.digest(), payload)?;
        Some((cert, sig))
    }
}

impl<K: KeyStore> Signer for PseudonymSigner<K> {
    fn current_temp_id(&mut self, now: f64) -> Option<TempId> {
        let cert = self.active_certificate(now)?;
        let d = cert.digest();
        if self.current != Some(d) {
            self.current = Some(d);
            self.rotated_at = now;
        }
        Some(temp_id_of(&d))
    }

    fn sign(&mut self, now: f64, is_event: bool, body: &[u8; BODY_LEN]) -> Option<Vec<u8>> {
        let cert = self.active_certificate(now)?;
        let d = cert.digest();
        if body_temp_id(body) != temp_id_of(&d) {
            return None; // never sign a pseudonym that is not ours
        }
        let hour = sender_hour(now, body_timestamp_ms(body));
        let signature = self.keys.sign(&d, &rsm_to_be_signed(hour, body))?;
        let full = is_event
            || now - self.rotated_at < self.sec.full_cert_after_rotation_s - 1e-9
            || now - self.last_full >= self.sec.full_cert_interval_s - 1e-9;
        let cert_ref = if full {
            self.last_full = now;
            CertRef::Full(cert)
        } else {
            CertRef::Digest(d)
        };
        Some(
            AuthEnvelope {
                signature,
                cert: cert_ref,
            }
            .encode(),
        )
    }
}

// ---------------------------------------------------------------- revocation list

/// A CA-signed list of revoked certificate digests (AUDIT S-05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevocationList {
    /// CA key that signs it.
    pub ca_key_id: u8,
    /// Increases with every change; phones never go back to a lower serial.
    pub serial: u32,
    /// Issue time (Unix s).
    pub issued_at: u32,
    /// Phones should fetch a new list by this time (Unix s).
    pub next_update: u32,
    /// Revoked certificate digests, strictly ascending.
    pub revoked: Vec<CertDigest>,
}

impl RevocationList {
    fn body(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(CRL_HEADER_LEN + self.revoked.len() * 8);
        b.push(CRL_VERSION);
        b.push(self.ca_key_id);
        b.extend_from_slice(&self.serial.to_be_bytes());
        b.extend_from_slice(&self.issued_at.to_be_bytes());
        b.extend_from_slice(&self.next_update.to_be_bytes());
        b.extend_from_slice(&(self.revoked.len() as u32).to_be_bytes());
        for d in &self.revoked {
            b.extend_from_slice(d);
        }
        b
    }

    /// The bytes the CA signs.
    pub fn to_be_signed(&self) -> Vec<u8> {
        [DOMAIN_CRL.as_slice(), &self.body()].concat()
    }

    /// Encode with the CA signature appended.
    pub fn encode_signed(&self, signature: &[u8; SIG_LEN]) -> Vec<u8> {
        let mut b = self.body();
        b.extend_from_slice(signature);
        b
    }

    /// Parse untrusted bytes. Does not check the signature.
    pub fn decode(b: &[u8], max_entries: usize) -> Result<(Self, [u8; SIG_LEN]), PkiError> {
        if b.len() < CRL_HEADER_LEN + SIG_LEN {
            return Err(PkiError::Length);
        }
        if b[0] != CRL_VERSION {
            return Err(PkiError::Version);
        }
        let u32_at = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let count = u32_at(14) as usize;
        if count > max_entries {
            return Err(PkiError::TooMany);
        }
        let end = count
            .checked_mul(8)
            .and_then(|n| n.checked_add(CRL_HEADER_LEN))
            .ok_or(PkiError::Length)?;
        if b.len() != end + SIG_LEN {
            return Err(PkiError::Length);
        }
        let mut revoked = Vec::with_capacity(count);
        for chunk in b[CRL_HEADER_LEN..end].chunks_exact(8) {
            let mut d = [0u8; 8];
            d.copy_from_slice(chunk);
            if revoked.last().is_some_and(|prev| *prev >= d) {
                return Err(PkiError::NotSorted);
            }
            revoked.push(d);
        }
        let mut sig = [0u8; SIG_LEN];
        sig.copy_from_slice(&b[end..]);
        Ok((
            Self {
                ca_key_id: b[1],
                serial: u32_at(2),
                issued_at: u32_at(6),
                next_update: u32_at(10),
                revoked,
            },
            sig,
        ))
    }

    /// Is this certificate revoked?
    pub fn contains(&self, d: &CertDigest) -> bool {
        self.revoked.binary_search(d).is_ok()
    }
}

/// Result of loading a revocation list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrlUpdate {
    /// A newer list was installed.
    Updated(u32),
    /// Same serial as the list already held.
    Unchanged,
}

// ---------------------------------------------------------------- verification (phone side)

#[derive(Debug, Clone)]
struct Cached {
    cert: Certificate,
    key: VerifyingKey,
}

/// Verifies messages against pseudonym certificates, the trusted CA keys and the
/// revocation list. One per phone; caches verified certificates.
#[derive(Debug, Clone)]
pub struct CertVerifier {
    anchors: Vec<(u8, VerifyingKey)>,
    cache: HashMap<CertDigest, Cached>,
    crl: Option<RevocationList>,
    sec: Security,
}

impl CertVerifier {
    /// Verifier trusting `anchors` (the CA public keys shipped with the app).
    pub fn new(anchors: &[TrustAnchor], sec: &Security) -> Result<Self, PkiError> {
        let anchors = anchors
            .iter()
            .map(|a| Ok((a.key_id, parse_public_key(&a.public_key)?)))
            .collect::<Result<Vec<_>, PkiError>>()?;
        Ok(Self {
            anchors,
            cache: HashMap::new(),
            crl: None,
            sec: sec.clone(),
        })
    }

    fn anchor(&self, key_id: u8) -> Option<&VerifyingKey> {
        self.anchors
            .iter()
            .find(|(id, _)| *id == key_id)
            .map(|(_, k)| k)
    }

    /// Install a revocation list fetched from the server (untrusted bytes).
    /// Rejects bad signatures and lists older than the one held (rollback).
    pub fn load_revocation_list(&mut self, bytes: &[u8]) -> Result<CrlUpdate, PkiError> {
        let (list, sig) = RevocationList::decode(bytes, self.sec.crl_max_entries)?;
        let ca = self.anchor(list.ca_key_id).ok_or(PkiError::UnknownCa)?;
        if !verify_signature(ca, &list.to_be_signed(), &sig) {
            return Err(PkiError::BadSignature);
        }
        match &self.crl {
            Some(cur) if list.serial < cur.serial => return Err(PkiError::Rollback),
            Some(cur) if list.serial == cur.serial => return Ok(CrlUpdate::Unchanged),
            _ => {}
        }
        self.cache.retain(|d, _| !list.contains(d));
        let serial = list.serial;
        self.crl = Some(list);
        Ok(CrlUpdate::Updated(serial))
    }

    /// Serial of the revocation list held.
    pub fn revocation_serial(&self) -> Option<u32> {
        self.crl.as_ref().map(|l| l.serial)
    }

    /// True when no list is held or it is past its `next_update`: the platform should refresh.
    pub fn revocation_list_due(&self, now: f64) -> bool {
        self.crl
            .as_ref()
            .is_none_or(|l| now >= f64::from(l.next_update))
    }

    /// Is this certificate on the revocation list?
    pub fn is_revoked(&self, d: &CertDigest) -> bool {
        self.crl.as_ref().is_some_and(|l| l.contains(d))
    }

    /// Certificates in the cache.
    pub fn cached(&self) -> usize {
        self.cache.len()
    }

    /// Check a certificate (CA signature, key, validity, revocation) and cache it.
    pub fn check_certificate(
        &mut self,
        cert: &Certificate,
        now: f64,
    ) -> Result<VerifyingKey, AuthError> {
        let d = cert.digest();
        if self.is_revoked(&d) {
            return Err(AuthError::RevokedCertificate);
        }
        if let Some(c) = self.cache.get(&d) {
            if c.cert != *cert {
                return Err(AuthError::BadCertificate); // digest collision attempt
            }
        } else {
            let ca = self
                .anchor(cert.ca_key_id)
                .ok_or(AuthError::BadCertificate)?;
            let key = cert
                .verify_issuer(ca)
                .map_err(|_| AuthError::BadCertificate)?;
            if !cert.valid_at(now, self.sec.clock_skew_s) {
                return Err(AuthError::ExpiredCertificate);
            }
            self.insert(d, Cached { cert: *cert, key }, now);
        }
        self.cached_key(&d, now)
    }

    fn insert(&mut self, d: CertDigest, c: Cached, now: f64) {
        if self.cache.len() >= self.sec.cert_cache_max {
            let skew = self.sec.clock_skew_s;
            self.cache.retain(|_, c| c.cert.valid_at(now, skew));
        }
        while self.cache.len() >= self.sec.cert_cache_max.max(1) {
            let Some(oldest) = self
                .cache
                .iter()
                .min_by_key(|(k, c)| (c.cert.valid_until, **k))
                .map(|(k, _)| *k)
            else {
                break;
            };
            self.cache.remove(&oldest);
        }
        self.cache.insert(d, c);
    }

    fn cached_key(&self, d: &CertDigest, now: f64) -> Result<VerifyingKey, AuthError> {
        let c = self.cache.get(d).ok_or(AuthError::UnknownCertificate)?;
        if !c.cert.valid_at(now, self.sec.clock_skew_s) {
            return Err(AuthError::ExpiredCertificate);
        }
        Ok(c.key)
    }

    fn resolve(
        &mut self,
        now: f64,
        body: &[u8; BODY_LEN],
        env: &AuthEnvelope,
    ) -> Result<VerifyingKey, AuthError> {
        let d = env.cert.digest();
        if body_temp_id(body) != temp_id_of(&d) {
            return Err(AuthError::TempIdMismatch);
        }
        if self.is_revoked(&d) {
            return Err(AuthError::RevokedCertificate);
        }
        match &env.cert {
            CertRef::Full(c) => self.check_certificate(c, now),
            CertRef::Digest(d) => self.cached_key(d, now),
        }
    }
}

impl Verifier for CertVerifier {
    fn verify(&mut self, now: f64, body: &[u8; BODY_LEN], auth: &[u8]) -> Result<(), AuthError> {
        let env = AuthEnvelope::decode(auth)?;
        let key = self.resolve(now, body, &env)?;
        if verify_rsm_signature(&key, now, body, &env.signature) {
            Ok(())
        } else {
            Err(AuthError::BadSignature)
        }
    }

    fn verify_certificate_only(
        &mut self,
        now: f64,
        body: &[u8; BODY_LEN],
        auth: &[u8],
    ) -> Result<(), AuthError> {
        let env = AuthEnvelope::decode(auth)?;
        self.resolve(now, body, &env).map(|_| ())
    }
}

// ---------------------------------------------------------------- misbehaviour reports

/// Reason codes in a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportReason {
    /// Speed outside the plausible range.
    ImplausibleSpeed = 1,
    /// Acceleration outside the plausible range.
    ImplausibleAccel = 2,
    /// Timestamp in the future.
    FutureTimestamp = 3,
    /// Message flood.
    RateTooHigh = 4,
    /// Impossible position jump.
    PositionJump = 5,
    /// Claimed position far from any road.
    OffRoad = 6,
}

impl ReportReason {
    /// From a byte.
    pub fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            1 => Self::ImplausibleSpeed,
            2 => Self::ImplausibleAccel,
            3 => Self::FutureTimestamp,
            4 => Self::RateTooHigh,
            5 => Self::PositionJump,
            6 => Self::OffRoad,
            _ => return None,
        })
    }

    /// Can the server confirm this reason from the signed evidence alone?
    /// (A future timestamp depends on when the reporter received the message;
    /// off-road needs the map, which the server checks once it has one, Step 9.)
    pub fn checkable_from_evidence(self) -> bool {
        !matches!(self, Self::FutureTimestamp | Self::OffRoad)
    }
}

impl From<Misbehaviour> for ReportReason {
    fn from(m: Misbehaviour) -> Self {
        match m {
            Misbehaviour::ImplausibleSpeed(_) => Self::ImplausibleSpeed,
            Misbehaviour::ImplausibleAccel(_) => Self::ImplausibleAccel,
            Misbehaviour::FutureTimestamp => Self::FutureTimestamp,
            Misbehaviour::RateTooHigh => Self::RateTooHigh,
            Misbehaviour::PositionJump(_) => Self::PositionJump,
            Misbehaviour::OffRoad(_) => Self::OffRoad,
        }
    }
}

/// A misbehaviour report a phone sends to the server.
#[derive(Debug, Clone, PartialEq)]
pub struct MisbehaviourReport {
    /// Why.
    pub reason: ReportReason,
    /// When the reporter saw it (Unix s).
    pub observed_at: u32,
    /// The accused sender's signed frames, as received.
    pub evidence: Vec<Frame>,
    /// Reporter's pseudonym certificate.
    pub reporter: Certificate,
    /// Reporter's signature over [`MisbehaviourReport::to_be_signed`].
    pub signature: [u8; SIG_LEN],
}

impl MisbehaviourReport {
    fn body(
        reason: ReportReason,
        observed_at: u32,
        evidence: &[Frame],
        reporter: &Certificate,
    ) -> Option<Vec<u8>> {
        let mut b = vec![REPORT_VERSION, reason as u8];
        b.extend_from_slice(&observed_at.to_be_bytes());
        b.push(u8::try_from(evidence.len()).ok()?);
        for f in evidence {
            let e = f.encode()?;
            b.push(u8::try_from(e.len()).ok()?);
            b.extend_from_slice(&e);
        }
        b.extend_from_slice(&reporter.encode());
        Some(b)
    }

    /// The bytes the reporter signs.
    pub fn to_be_signed(&self) -> Option<Vec<u8>> {
        let body = Self::body(
            self.reason,
            self.observed_at,
            &self.evidence,
            &self.reporter,
        )?;
        Some([DOMAIN_REPORT.as_slice(), &body].concat())
    }

    /// Build and sign a report with the reporter's current pseudonym key.
    pub fn build<K: KeyStore>(
        signer: &mut PseudonymSigner<K>,
        now: f64,
        reason: ReportReason,
        evidence: Vec<Frame>,
    ) -> Option<Self> {
        let reporter = signer.active_certificate(now)?;
        let observed_at = now.max(0.0).min(f64::from(u32::MAX)) as u32;
        let body = Self::body(reason, observed_at, &evidence, &reporter)?;
        let payload = [DOMAIN_REPORT.as_slice(), &body].concat();
        let (cert, signature) = signer.sign_with_current(now, &payload)?;
        if cert != reporter {
            return None;
        }
        Some(Self {
            reason,
            observed_at,
            evidence,
            reporter,
            signature,
        })
    }

    /// Encode.
    pub fn encode(&self) -> Option<Vec<u8>> {
        let mut b = Self::body(
            self.reason,
            self.observed_at,
            &self.evidence,
            &self.reporter,
        )?;
        b.extend_from_slice(&self.signature);
        Some(b)
    }

    /// Parse untrusted bytes. Does not check any signature.
    pub fn decode(b: &[u8], max_evidence: usize) -> Result<Self, PkiError> {
        let mut i = 0usize;
        let take = |i: &mut usize, n: usize| -> Result<&[u8], PkiError> {
            let end = i.checked_add(n).ok_or(PkiError::Length)?;
            let s = b.get(*i..end).ok_or(PkiError::Length)?;
            *i = end;
            Ok(s)
        };
        let head = take(&mut i, 7)?;
        if head[0] != REPORT_VERSION {
            return Err(PkiError::Version);
        }
        let reason = ReportReason::from_u8(head[1]).ok_or(PkiError::Version)?;
        let observed_at = u32::from_be_bytes([head[2], head[3], head[4], head[5]]);
        let n = usize::from(head[6]);
        if n == 0 || n > max_evidence {
            return Err(PkiError::TooMany);
        }
        let mut evidence = Vec::with_capacity(n);
        for _ in 0..n {
            let len = usize::from(take(&mut i, 1)?[0]);
            let f = Frame::decode(take(&mut i, len)?).map_err(|_| PkiError::BadEvidence)?;
            evidence.push(f);
        }
        let reporter = Certificate::decode(take(&mut i, CERT_LEN)?)?;
        let mut signature = [0u8; SIG_LEN];
        signature.copy_from_slice(take(&mut i, SIG_LEN)?);
        if i != b.len() {
            return Err(PkiError::Length);
        }
        Ok(Self {
            reason,
            observed_at,
            evidence,
            reporter,
            signature,
        })
    }
}

/// Re-run the misbehaviour checks on evidence frames from one sender, in the
/// order they were sent. Returns what the evidence shows (server side, AUDIT S-06).
/// Signatures are NOT checked here; the caller verifies them first.
pub fn evidence_shows(
    frames: &[Frame],
    observed_at: f64,
    cfg: &Config,
) -> Result<Option<Misbehaviour>, PkiError> {
    let mut msgs = Vec::with_capacity(frames.len());
    let mut sender = None;
    for f in frames {
        let r = Rsm::decode(&f.body).map_err(|_| PkiError::BadEvidence)?;
        if sender.is_some_and(|s| s != r.temp_id) {
            return Err(PkiError::BadEvidence);
        }
        sender = Some(r.temp_id);
        msgs.push(r);
    }
    let first = msgs.first().ok_or(PkiError::BadEvidence)?;
    let origin = GeoOrigin::new(first.lat_deg, first.lon_deg);
    let mut peer: Vec<PeerMsg> = msgs
        .iter()
        .map(|r| PeerMsg::from_rsm(r, &origin, observed_at))
        .collect();
    peer.sort_by_key(|m| m.t_ms);
    let mut ldm = Ldm::new();
    for m in peer {
        // judge each message at the moment it was sent: freshness is not evidence
        match ldm.check(&m, m.t, cfg) {
            Check::Misbehaviour(Misbehaviour::FutureTimestamp) | Check::Stale => {}
            Check::Misbehaviour(x) => return Ok(Some(x)),
            Check::Duplicate => continue,
            Check::Ok => {}
        }
        ldm.update(m, m.t, 0);
    }
    Ok(None)
}

// ---------------------------------------------------------------- issuance messages

const ISSUE_VERSION: u8 = 1;

/// The bytes a phone's device key signs to request pseudonym certificates (SPEC §10).
pub fn issue_tbs(
    device_key: &[u8; PUBLIC_KEY_LEN],
    requested_at: u32,
    pseudonym_keys: &[[u8; PUBLIC_KEY_LEN]],
) -> Vec<u8> {
    let mut b = DOMAIN_ISSUE.to_vec();
    b.extend_from_slice(device_key);
    b.extend_from_slice(&requested_at.to_be_bytes());
    b.extend_from_slice(&(pseudonym_keys.len() as u32).to_be_bytes());
    for k in pseudonym_keys {
        b.extend_from_slice(k);
    }
    b
}

/// A certificate request on the wire:
/// `[version:1][device_key:33][requested_at:4][n:1][key:33]*n[attestation_len:2][attestation][signature:64]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRequestWire {
    /// The install's device public key.
    pub device_key: [u8; PUBLIC_KEY_LEN],
    /// When the phone made the request (Unix s).
    pub requested_at: u32,
    /// One public key per requested certificate.
    pub pseudonym_keys: Vec<[u8; PUBLIC_KEY_LEN]>,
    /// App Attest / Play Integrity evidence.
    pub attestation: Vec<u8>,
    /// Device-key signature over [`issue_tbs`].
    pub signature: [u8; SIG_LEN],
}

impl IssueRequestWire {
    /// Encode. `None` if more than 255 keys or attestation over 65 535 bytes.
    pub fn encode(&self) -> Option<Vec<u8>> {
        let mut b = vec![ISSUE_VERSION];
        b.extend_from_slice(&self.device_key);
        b.extend_from_slice(&self.requested_at.to_be_bytes());
        b.push(u8::try_from(self.pseudonym_keys.len()).ok()?);
        for k in &self.pseudonym_keys {
            b.extend_from_slice(k);
        }
        b.extend_from_slice(&u16::try_from(self.attestation.len()).ok()?.to_be_bytes());
        b.extend_from_slice(&self.attestation);
        b.extend_from_slice(&self.signature);
        Some(b)
    }

    /// Parse untrusted bytes. Does not check the signature.
    pub fn decode(b: &[u8]) -> Result<Self, PkiError> {
        let mut i = 0usize;
        let take = |i: &mut usize, n: usize| -> Result<&[u8], PkiError> {
            let end = i.checked_add(n).ok_or(PkiError::Length)?;
            let s = b.get(*i..end).ok_or(PkiError::Length)?;
            *i = end;
            Ok(s)
        };
        if take(&mut i, 1)?[0] != ISSUE_VERSION {
            return Err(PkiError::Version);
        }
        let mut device_key = [0u8; PUBLIC_KEY_LEN];
        device_key.copy_from_slice(take(&mut i, PUBLIC_KEY_LEN)?);
        let t = take(&mut i, 4)?;
        let requested_at = u32::from_be_bytes([t[0], t[1], t[2], t[3]]);
        let n = usize::from(take(&mut i, 1)?[0]);
        let mut pseudonym_keys = Vec::with_capacity(n);
        for _ in 0..n {
            let mut k = [0u8; PUBLIC_KEY_LEN];
            k.copy_from_slice(take(&mut i, PUBLIC_KEY_LEN)?);
            pseudonym_keys.push(k);
        }
        let l = take(&mut i, 2)?;
        let attestation = take(&mut i, usize::from(u16::from_be_bytes([l[0], l[1]])))?.to_vec();
        let mut signature = [0u8; SIG_LEN];
        signature.copy_from_slice(take(&mut i, SIG_LEN)?);
        if i != b.len() {
            return Err(PkiError::Length);
        }
        Ok(Self {
            device_key,
            requested_at,
            pseudonym_keys,
            attestation,
            signature,
        })
    }
}

/// Certificates on the wire: back to back, 107 bytes each.
pub fn encode_certificates(certs: &[Certificate]) -> Vec<u8> {
    certs.iter().flat_map(|c| c.encode()).collect()
}

/// Parse a certificate batch (untrusted bytes; CA signatures checked separately).
pub fn decode_certificates(b: &[u8]) -> Result<Vec<Certificate>, PkiError> {
    if !b.len().is_multiple_of(CERT_LEN) {
        return Err(PkiError::Length);
    }
    b.chunks_exact(CERT_LEN).map(Certificate::decode).collect()
}

#[cfg(test)]
mod tests {
    //! Keys here are generated from the OS random source at run time and thrown
    //! away; nothing is hard-coded (CLAUDE.md security rules).
    #![allow(clippy::panic)]

    use super::*;
    use crate::rsm::{EventCode, MsgType, VehicleType};
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer as _;
    use p256::elliptic_curve::Generate;

    const T0: u32 = 1_800_000_000; // a Unix time in 2027

    fn sign_raw(k: &SigningKey, msg: &[u8]) -> [u8; SIG_LEN] {
        let s: Signature = k.sign(msg);
        let mut out = [0u8; SIG_LEN];
        out.copy_from_slice(&s.to_bytes());
        out
    }

    fn pubkey(k: &SigningKey) -> [u8; PUBLIC_KEY_LEN] {
        let mut out = [0u8; PUBLIC_KEY_LEN];
        out.copy_from_slice(k.verifying_key().to_sec1_point(true).as_bytes());
        out
    }

    struct TestCa {
        key: SigningKey,
        id: u8,
    }

    impl TestCa {
        fn new(id: u8) -> Self {
            Self {
                key: SigningKey::generate(),
                id,
            }
        }
        fn anchor(&self) -> TrustAnchor {
            TrustAnchor {
                key_id: self.id,
                public_key: pubkey(&self.key),
            }
        }
        fn issue(&self, pseudonym: &SigningKey, from: u32, until: u32) -> Certificate {
            let mut c = Certificate {
                ca_key_id: self.id,
                valid_from: from,
                valid_until: until,
                public_key: pubkey(pseudonym),
                ca_signature: [0; SIG_LEN],
            };
            c.ca_signature = sign_raw(&self.key, &c.to_be_signed());
            c
        }
        fn crl(&self, serial: u32, mut revoked: Vec<CertDigest>) -> Vec<u8> {
            revoked.sort();
            let l = RevocationList {
                ca_key_id: self.id,
                serial,
                issued_at: T0,
                next_update: T0 + 86_400,
                revoked,
            };
            l.encode_signed(&sign_raw(&self.key, &l.to_be_signed()))
        }
    }

    #[derive(Default)]
    struct SoftKeys(HashMap<CertDigest, SigningKey>);
    impl KeyStore for SoftKeys {
        fn sign(&mut self, d: &CertDigest, msg: &[u8]) -> Option<[u8; SIG_LEN]> {
            self.0.get(d).map(|k| sign_raw(k, msg))
        }
    }

    fn device(ca: &TestCa, n: usize) -> PseudonymSigner<SoftKeys> {
        let mut keys = SoftKeys::default();
        let mut certs = Vec::new();
        for _ in 0..n {
            let k = SigningKey::generate();
            let c = ca.issue(&k, T0 - 60, T0 + 7 * 86_400);
            keys.0.insert(c.digest(), k);
            certs.push(c);
        }
        PseudonymSigner::new(certs, keys, &Security::v0())
    }

    fn body(temp_id: TempId, now: f64, msg_id: u16) -> [u8; BODY_LEN] {
        Rsm {
            msg_type: MsgType::Heartbeat,
            temp_id,
            msg_id,
            timestamp_ms: rsm::ms_of_hour(now),
            lat_deg: 11.5564,
            lon_deg: 104.9282,
            speed: 10.0,
            heading_deg: 90.0,
            accel: 0.0,
            yaw_rate: 0.0,
            vehicle_type: VehicleType::Car,
            flags: 0,
            pos_accuracy_m: 1.0,
            hop_count: 0,
            ttl: 2,
            origin_pos_age_ms: 0,
            event: EventCode::None,
        }
        .encode()
    }

    fn signed(
        dev: &mut PseudonymSigner<SoftKeys>,
        now: f64,
        msg_id: u16,
    ) -> ([u8; BODY_LEN], Vec<u8>) {
        let id = dev
            .current_temp_id(now)
            .unwrap_or_else(|| panic!("no cert"));
        let b = body(id, now, msg_id);
        let auth = dev.sign(now, false, &b).unwrap_or_else(|| panic!("no sig"));
        (b, auth)
    }

    fn verifier(ca: &TestCa) -> CertVerifier {
        CertVerifier::new(&[ca.anchor()], &Security::v0()).unwrap_or_else(|_| panic!("anchor"))
    }

    fn now0() -> f64 {
        f64::from(T0)
    }

    #[test]
    fn certificate_round_trip_and_binding() {
        let ca = TestCa::new(1);
        let c = ca.issue(&SigningKey::generate(), T0, T0 + 10);
        assert_eq!(Certificate::decode(&c.encode()), Ok(c));
        assert_eq!(c.temp_id(), temp_id_of(&c.digest()));
        assert!(c.verify_issuer(ca.key.verifying_key()).is_ok());
        let mut bad = c;
        bad.valid_until += 1; // any change breaks the CA signature
        assert_eq!(
            bad.verify_issuer(ca.key.verifying_key()).err(),
            Some(PkiError::BadSignature)
        );
        assert_eq!(Certificate::decode(&[1; 106]), Err(PkiError::Length));
        let mut v2 = c.encode();
        v2[0] = 2;
        assert_eq!(Certificate::decode(&v2), Err(PkiError::Version));
    }

    #[test]
    fn valid_signature_verifies_and_digest_works_after_full_cert() {
        let ca = TestCa::new(1);
        let mut dev = device(&ca, 1);
        let mut v = verifier(&ca);
        let now = now0();
        let (b, auth) = signed(&mut dev, now, 1);
        assert_eq!(auth.len(), AUTH_FULL_LEN); // first message after a rotation: full certificate
        assert_eq!(v.verify(now, &b, &auth), Ok(()));
        let (b2, auth2) = signed(&mut dev, now + 2.5, 2);
        assert_eq!(auth2.len(), AUTH_FULL_LEN); // 1 s since the last full certificate
        assert_eq!(v.verify(now + 2.5, &b2, &auth2), Ok(()));
        let (b3, auth3) = signed(&mut dev, now + 2.6, 3);
        assert_eq!(auth3.len(), AUTH_DIGEST_LEN); // digest only in between
        assert_eq!(v.verify(now + 2.6, &b3, &auth3), Ok(()));
    }

    #[test]
    fn digest_from_unknown_sender_waits_for_full_certificate() {
        let ca = TestCa::new(1);
        let mut dev = device(&ca, 1);
        let now = now0();
        let _ = signed(&mut dev, now, 1);
        let _ = signed(&mut dev, now + 2.5, 2);
        let (b, auth) = signed(&mut dev, now + 2.6, 3);
        assert_eq!(auth.len(), AUTH_DIGEST_LEN);
        assert_eq!(
            verifier(&ca).verify(now + 2.6, &b, &auth),
            Err(AuthError::UnknownCertificate)
        );
    }

    #[test]
    fn events_always_carry_the_full_certificate() {
        let ca = TestCa::new(1);
        let mut dev = device(&ca, 1);
        let now = now0();
        let _ = signed(&mut dev, now, 1);
        let _ = signed(&mut dev, now + 2.5, 2);
        let id = dev
            .current_temp_id(now + 2.6)
            .unwrap_or_else(|| panic!("id"));
        let auth = dev
            .sign(now + 2.6, true, &body(id, now + 2.6, 3))
            .unwrap_or_else(|| panic!("sig"));
        assert_eq!(auth.len(), AUTH_FULL_LEN);
        assert!(auth.len() <= rsm::MAX_AUTH_LEN);
    }

    #[test]
    fn tampered_body_or_wrong_key_fails() {
        let ca = TestCa::new(1);
        let mut dev = device(&ca, 1);
        let mut v = verifier(&ca);
        let now = now0();
        let (b, auth) = signed(&mut dev, now, 1);
        let mut t = b;
        t[20] ^= 1; // speed
        assert_eq!(v.verify(now, &t, &auth), Err(AuthError::BadSignature));
        let mut sig_bad = auth.clone();
        sig_bad[5] ^= 1;
        assert_eq!(v.verify(now, &b, &sig_bad), Err(AuthError::BadSignature));
        // relay-mutable fields may change
        let mut relayed = b;
        relayed[27] = 0x11;
        relayed[28] = 0x01;
        assert_eq!(v.verify(now, &relayed, &auth), Ok(()));
    }

    #[test]
    fn certificate_from_another_ca_is_rejected() {
        let ca = TestCa::new(1);
        let rogue = TestCa::new(1); // same key id, different key
        let mut dev = device(&rogue, 1);
        let now = now0();
        let (b, auth) = signed(&mut dev, now, 1);
        assert_eq!(
            verifier(&ca).verify(now, &b, &auth),
            Err(AuthError::BadCertificate)
        );
        let unknown_id = TestCa::new(9);
        let mut dev9 = device(&unknown_id, 1);
        let (b9, a9) = signed(&mut dev9, now, 1);
        assert_eq!(
            verifier(&ca).verify(now, &b9, &a9),
            Err(AuthError::BadCertificate)
        );
    }

    #[test]
    fn temp_id_must_match_certificate() {
        let ca = TestCa::new(1);
        let mut mine = device(&ca, 1);
        let victim = device(&ca, 1).certificates()[0];
        let now = now0();
        let (_, my_auth) = signed(&mut mine, now, 1);
        // claim the victim's pseudonym with my certificate and signature
        let claimed = body(victim.temp_id(), now, 1);
        assert_eq!(
            verifier(&ca).verify(now, &claimed, &my_auth),
            Err(AuthError::TempIdMismatch)
        );
        // and my signer refuses to sign a body that is not mine
        assert!(mine.sign(now, false, &claimed).is_none());
    }

    #[test]
    fn expired_or_not_yet_valid_certificate_is_rejected() {
        let ca = TestCa::new(1);
        let k = SigningKey::generate();
        let c = ca.issue(&k, T0, T0 + 100);
        let mut keys = SoftKeys::default();
        keys.0.insert(c.digest(), k);
        let mut dev = PseudonymSigner::new(vec![c], keys, &Security::v0());
        let (b, auth) = signed(&mut dev, now0() + 50.0, 1);
        let mut v = verifier(&ca);
        assert_eq!(v.verify(now0() + 50.0, &b, &auth), Ok(()));
        assert_eq!(
            v.verify(now0() + 200.0, &b, &auth),
            Err(AuthError::ExpiredCertificate)
        );
        assert_eq!(
            verifier(&ca).verify(now0() - 100.0, &b, &auth),
            Err(AuthError::ExpiredCertificate)
        );
        // a phone with only expired certificates goes receive-only
        assert!(dev.current_temp_id(now0() + 200.0).is_none());
    }

    #[test]
    fn replay_an_hour_later_fails() {
        // The body carries only ms-since-the-hour; the signed hour stops replays.
        let ca = TestCa::new(1);
        let mut dev = device(&ca, 1);
        let mut v = verifier(&ca);
        let now = now0();
        let (b, auth) = signed(&mut dev, now, 1);
        assert_eq!(v.verify(now, &b, &auth), Ok(()));
        for later in [3600.0, 7200.0, 86_400.0] {
            assert_eq!(
                v.verify(now + later, &b, &auth),
                Err(AuthError::BadSignature),
                "replayed {later} s later"
            );
        }
    }

    #[test]
    fn revocation_list_rejects_revoked_certificates() {
        let ca = TestCa::new(1);
        let mut dev = device(&ca, 1);
        let mut v = verifier(&ca);
        let now = now0();
        let (b, auth) = signed(&mut dev, now, 1);
        assert_eq!(v.verify(now, &b, &auth), Ok(()));
        let d = dev.certificates()[0].digest();
        assert_eq!(
            v.load_revocation_list(&ca.crl(1, vec![d])),
            Ok(CrlUpdate::Updated(1))
        );
        assert_eq!(v.verify(now, &b, &auth), Err(AuthError::RevokedCertificate));
        assert_eq!(v.cached(), 0); // dropped from the cache
    }

    #[test]
    fn revocation_list_must_be_signed_newer_and_sorted() {
        let ca = TestCa::new(1);
        let rogue = TestCa::new(1);
        let mut v = verifier(&ca);
        assert_eq!(
            v.load_revocation_list(&rogue.crl(5, vec![[1; 8]])),
            Err(PkiError::BadSignature)
        );
        assert_eq!(
            v.load_revocation_list(&ca.crl(5, vec![[1; 8]])),
            Ok(CrlUpdate::Updated(5))
        );
        assert_eq!(
            v.load_revocation_list(&ca.crl(5, vec![])),
            Ok(CrlUpdate::Unchanged)
        );
        assert_eq!(
            v.load_revocation_list(&ca.crl(4, vec![])),
            Err(PkiError::Rollback)
        );
        assert!(v.is_revoked(&[1; 8]));
        // unsorted
        let l = RevocationList {
            ca_key_id: 1,
            serial: 9,
            issued_at: T0,
            next_update: T0,
            revoked: vec![[2; 8], [1; 8]],
        };
        let bytes = l.encode_signed(&sign_raw(&ca.key, &l.to_be_signed()));
        assert_eq!(v.load_revocation_list(&bytes), Err(PkiError::NotSorted));
        // truncated / oversize count
        assert_eq!(v.load_revocation_list(&bytes[..30]), Err(PkiError::Length));
        let mut huge = bytes.clone();
        huge[14..18].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(v.load_revocation_list(&huge), Err(PkiError::TooMany));
        assert!(!v.revocation_list_due(f64::from(T0)));
        assert!(v.revocation_list_due(f64::from(T0 + 86_400))); // past next_update: refresh
    }

    #[test]
    fn rotation_switches_pseudonym_and_certificate() {
        let ca = TestCa::new(1);
        let mut dev = device(&ca, 3);
        let now = now0();
        let a = dev.current_temp_id(now);
        let b = dev.current_temp_id(now + 300.0);
        assert!(a.is_some() && b.is_some() && a != b);
        // the first messages after a switch carry the full certificate
        let (_, auth) = signed(&mut dev, now + 300.5, 7);
        assert_eq!(auth.len(), AUTH_FULL_LEN);
    }

    #[test]
    fn issue_request_and_certificate_batch_round_trip() {
        let r = IssueRequestWire {
            device_key: [2; PUBLIC_KEY_LEN],
            requested_at: T0,
            pseudonym_keys: vec![[3; PUBLIC_KEY_LEN], [2; PUBLIC_KEY_LEN]],
            attestation: b"att".to_vec(),
            signature: [9; SIG_LEN],
        };
        let b = r.encode().unwrap_or_else(|| panic!("encode"));
        assert_eq!(IssueRequestWire::decode(&b), Ok(r));
        assert!(IssueRequestWire::decode(&b[..b.len() - 1]).is_err());
        let ca = TestCa::new(1);
        let certs = vec![ca.issue(&SigningKey::generate(), T0, T0 + 1); 2];
        assert_eq!(decode_certificates(&encode_certificates(&certs)), Ok(certs));
        assert_eq!(decode_certificates(&[0; 10]), Err(PkiError::Length));
    }

    #[test]
    fn rotation_waits_for_a_stop_or_junction_but_not_forever() {
        // AUDIT S-13
        let ca = TestCa::new(1);
        let mut dev = device(&ca, 3);
        let now = now0();
        dev.set_rotation_allowed(true);
        let first = dev.current_temp_id(now);
        dev.set_rotation_allowed(false); // driving between junctions
        assert_eq!(
            dev.current_temp_id(now + 310.0),
            first,
            "kept while driving"
        );
        let (b, a) = signed(&mut dev, now + 310.0, 2);
        assert_eq!(body_temp_id(&b), first.unwrap_or([0; 4]));
        assert!(verifier(&ca).verify(now + 310.0, &b, &a).is_ok());
        assert_ne!(
            dev.current_temp_id(now + 601.0),
            first,
            "forced after the maximum delay"
        );
        let mut dev2 = device(&ca, 3);
        let f2 = dev2.current_temp_id(now);
        dev2.set_rotation_allowed(false);
        let _ = dev2.current_temp_id(now + 310.0);
        dev2.set_rotation_allowed(true); // stopped at a junction
        assert_ne!(dev2.current_temp_id(now + 311.0), f2);
    }

    #[test]
    fn envelope_decoder_rejects_garbage() {
        for bad in [
            &[][..],
            &[1][..],
            &[1; 72][..],
            &[1; 74][..],
            &[2; 171][..],
            &[3; 73][..],
        ] {
            assert_eq!(AuthEnvelope::decode(bad), Err(AuthError::MalformedAuth));
        }
    }

    #[test]
    fn cache_is_bounded() {
        let ca = TestCa::new(1);
        let mut sec = Security::v0();
        sec.cert_cache_max = 3;
        let mut v = CertVerifier::new(&[ca.anchor()], &sec).unwrap_or_else(|_| panic!("anchor"));
        for _ in 0..6 {
            let c = ca.issue(&SigningKey::generate(), T0, T0 + 1000);
            assert!(v.check_certificate(&c, now0()).is_ok());
        }
        assert_eq!(v.cached(), 3);
    }

    #[test]
    fn report_round_trip_and_signature() {
        let ca = TestCa::new(1);
        let mut reporter = device(&ca, 1);
        let mut evil = device(&ca, 1);
        let now = now0();
        let (b, auth) = signed(&mut evil, now, 1);
        let frame = Frame {
            last_hop: None,
            body: b,
            auth,
        };
        let r = MisbehaviourReport::build(
            &mut reporter,
            now,
            ReportReason::ImplausibleSpeed,
            vec![frame],
        )
        .unwrap_or_else(|| panic!("report"));
        let bytes = r.encode().unwrap_or_else(|| panic!("encode"));
        let back = MisbehaviourReport::decode(&bytes, 16).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(back, r);
        let key = parse_public_key(&back.reporter.public_key).unwrap_or_else(|_| panic!("key"));
        let tbs = back.to_be_signed().unwrap_or_else(|| panic!("tbs"));
        assert!(verify_signature(&key, &tbs, &back.signature));
        assert!(MisbehaviourReport::decode(&bytes[..bytes.len() - 1], 16).is_err());
        assert!(MisbehaviourReport::decode(&bytes, 0).is_err());
    }

    #[test]
    fn evidence_check_confirms_speed_and_jump_only_when_true() {
        let cfg = Config::spec_v0();
        let mk = |speed: f64, lat: f64, t: f64, id: u16| {
            let mut r = Rsm::decode(&body(*b"EVIL", t, id)).unwrap_or_else(|_| panic!("decode"));
            r.speed = speed;
            r.lat_deg = lat;
            Frame {
                last_hop: None,
                body: r.encode(),
                auth: Vec::new(),
            }
        };
        let t = now0();
        assert_eq!(evidence_shows(&[mk(10.0, 11.5, t, 1)], t, &cfg), Ok(None));
        assert!(matches!(
            evidence_shows(&[mk(95.0, 11.5, t, 1)], t, &cfg),
            Ok(Some(Misbehaviour::ImplausibleSpeed(_)))
        ));
        // 1 km in 0.1 s
        let jump = [mk(10.0, 11.5, t, 1), mk(10.0, 11.509, t + 0.1, 2)];
        assert!(matches!(
            evidence_shows(&jump, t + 0.1, &cfg),
            Ok(Some(Misbehaviour::PositionJump(_)))
        ));
        let mixed = [mk(10.0, 11.5, t, 1), {
            let mut f = mk(10.0, 11.5, t, 2);
            f.body[1] = b'X';
            f
        }];
        assert_eq!(evidence_shows(&mixed, t, &cfg), Err(PkiError::BadEvidence));
        assert_eq!(evidence_shows(&[], t, &cfg), Err(PkiError::BadEvidence));
    }
}
