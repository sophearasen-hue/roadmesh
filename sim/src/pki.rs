//! Demo PKI for the SIMULATOR ONLY: certificate authority, signer and verifier.
//!
//! Port of `reference/core-python/v2v/security.py`. Signatures are HMAC-SHA256
//! with keys looked up at the CA, which is fine for a simulator but must never
//! ship (GAP_REPORT F-3, AUDIT S-01, CLAUDE.md). This module lives in `sim/`
//! so it cannot be linked into the product core. Real ECDSA P-256 pseudonym
//! certificates arrive in Step 3.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use hmac::{Hmac, KeyInit, Mac};
use roadmesh_core::auth::{AuthError, Signer, Verifier};
use roadmesh_core::rsm::{BODY_LEN, TempId};
use sha2::Sha256;

use crate::rng::SimRng;

type HmacSha256 = Hmac<Sha256>;

/// A pseudonym certificate: id (= temp_id on the air) and its demo key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Credential {
    /// Certificate id, used as the message temp_id.
    pub cert_id: TempId,
    /// Demo HMAC key.
    pub key: [u8; 32],
}

/// Demo certificate authority with misbehaviour-driven revocation.
#[derive(Debug)]
pub struct Ca {
    rng: SimRng,
    keys: HashMap<TempId, [u8; 32]>,
    /// Revoked certificate ids.
    pub revoked: HashSet<TempId>,
    reports: HashMap<TempId, HashSet<String>>,
    /// Independent reporters needed to revoke (3, CLAUDE.md).
    pub revoke_after: usize,
}

impl Ca {
    /// New CA with a seeded generator.
    pub fn new(seed: u64, revoke_after: usize) -> Self {
        Self {
            rng: SimRng::new(seed),
            keys: HashMap::new(),
            revoked: HashSet::new(),
            reports: HashMap::new(),
            revoke_after,
        }
    }

    /// Issue `count` certificates.
    pub fn issue(&mut self, count: usize) -> Vec<Credential> {
        (0..count)
            .map(|_| {
                let cred = Credential {
                    cert_id: self.rng.bytes::<4>(),
                    key: self.rng.bytes::<32>(),
                };
                self.keys.insert(cred.cert_id, cred.key);
                cred
            })
            .collect()
    }

    /// Key of a valid, unrevoked certificate.
    pub fn lookup(&self, cert_id: &TempId) -> Option<[u8; 32]> {
        if self.revoked.contains(cert_id) {
            None
        } else {
            self.keys.get(cert_id).copied()
        }
    }

    /// Misbehaviour report. Returns true if this report caused a revocation.
    pub fn report(&mut self, cert_id: &TempId, reporter: &str) -> bool {
        if !self.keys.contains_key(cert_id) || self.revoked.contains(cert_id) {
            return false;
        }
        let who = self.reports.entry(*cert_id).or_default();
        who.insert(reporter.to_owned());
        if who.len() >= self.revoke_after {
            self.revoked.insert(*cert_id);
            return true;
        }
        false
    }
}

/// HMAC over the signing bytes with a credential key.
pub fn demo_mac(key: &[u8; 32], signed: &[u8; BODY_LEN]) -> Vec<u8> {
    match HmacSha256::new_from_slice(key) {
        Ok(mut m) => {
            m.update(signed);
            m.finalize().into_bytes().to_vec()
        }
        Err(_) => Vec::new(), // unreachable: HMAC accepts any key length
    }
}

/// Signer holding a batch of pseudonym certificates, rotating every `rotate_every` s.
#[derive(Debug, Clone)]
pub struct DemoSigner {
    creds: Vec<Credential>,
    rotate_every: f64,
}

impl DemoSigner {
    /// Signer over a batch of credentials.
    pub fn new(creds: Vec<Credential>, rotate_every: f64) -> Self {
        Self {
            creds,
            rotate_every,
        }
    }

    /// All credentials (for mapping pseudonyms back to vehicles in reports).
    pub fn creds(&self) -> &[Credential] {
        &self.creds
    }
}

impl Signer for DemoSigner {
    fn current_temp_id(&mut self, now: f64) -> TempId {
        if self.creds.is_empty() {
            return [0; 4];
        }
        let i = ((now / self.rotate_every).floor().max(0.0) as usize) % self.creds.len();
        self.creds[i].cert_id
    }

    fn sign(&mut self, temp_id: &TempId, signed: &[u8; BODY_LEN]) -> Vec<u8> {
        match self.creds.iter().find(|c| &c.cert_id == temp_id) {
            Some(c) => demo_mac(&c.key, signed),
            None => Vec::new(),
        }
    }
}

/// Verifier backed by the shared demo CA. `enabled = false` accepts everything
/// (the "signature checks off" comparison runs).
#[derive(Debug, Clone)]
pub struct DemoVerifier {
    ca: Rc<RefCell<Ca>>,
    enabled: bool,
}

impl DemoVerifier {
    /// Verifier over a shared CA.
    pub fn new(ca: Rc<RefCell<Ca>>, enabled: bool) -> Self {
        Self { ca, enabled }
    }
}

impl Verifier for DemoVerifier {
    fn verify(
        &self,
        temp_id: &TempId,
        signed: &[u8; BODY_LEN],
        auth: &[u8],
    ) -> Result<(), AuthError> {
        if !self.enabled {
            return Ok(());
        }
        let ca = self.ca.borrow();
        let Some(key) = ca.lookup(temp_id) else {
            return Err(if ca.revoked.contains(temp_id) {
                AuthError::RevokedCertificate
            } else {
                AuthError::UnknownCertificate
            });
        };
        let Ok(mut m) = HmacSha256::new_from_slice(&key) else {
            return Err(AuthError::MalformedAuth);
        };
        m.update(signed);
        m.verify_slice(auth).map_err(|_| AuthError::BadSignature) // constant-time compare
    }
}
