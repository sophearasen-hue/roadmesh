//! The cellular bridge's gate (SPEC §5 "Cellular bridge", CLAUDE.md "verify before
//! you trust"): every frame a phone uploads is checked before anything is passed on.
//! The certificate must be one this CA issued, unexpired and not revoked; the
//! signature must verify; the message must be fresh and not a duplicate. Only hazard
//! *events* are pushed to other phones, to the map cell where they happened; position
//! heartbeats are checked and dropped (the server keeps no positions).
//!
//! Phones run every pushed frame through their own full receive pipeline again
//! (signature, revocation list, plausibility), so this gate stops floods and forgeries
//! early, and the phones remain the final judges.

use std::collections::HashMap;

use roadmesh_core::config::Config;
use roadmesh_core::geohash;
use roadmesh_core::pki::{self, AuthEnvelope, CertRef};
use roadmesh_core::rsm::{self, Frame, MsgType, Rsm, TempId};

use crate::ca::{Attestation, CaKey, CertAuthority};

/// Why an upload was not passed on (counted in the metrics; never logged with content).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reject {
    /// Not a frame, or not an RSM.
    Malformed,
    /// Signed with a certificate this CA did not issue.
    UnknownCertificate,
    /// A full certificate that differs from the one issued under its digest.
    ForgedCertificate,
    /// The certificate has expired or is not yet valid.
    Expired,
    /// The certificate is revoked.
    Revoked,
    /// The signature does not verify.
    BadSignature,
    /// Older than SPEC §5 rule 7 allows (1 s heartbeats, 10 s events), or from the future.
    Stale,
    /// Seen in the last few seconds.
    Duplicate,
}

/// What to do with an upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upload {
    /// A verified hazard event: push `frame` to phones subscribed to `cell`.
    Event {
        /// Geohash cell where it happened.
        cell: String,
        /// The frame to push (relay position removed).
        frame: Vec<u8>,
    },
    /// A verified heartbeat: nothing to pass on.
    Heartbeat,
    /// Not passed on.
    Rejected(Reject),
}

/// Duplicate cache and checks.
#[derive(Debug, Clone)]
pub struct HazardGate {
    cfg: Config,
    seen: HashMap<(TempId, u16), f64>,
    last_prune: f64,
}

impl HazardGate {
    /// A gate with the product profile's freshness limits.
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            seen: HashMap::new(),
            last_prune: 0.0,
        }
    }

    /// Check one uploaded frame at Unix time `now` (s).
    pub fn check<K: CaKey, A: Attestation>(
        &mut self,
        ca: &CertAuthority<K, A>,
        bytes: &[u8],
        now: f64,
    ) -> Upload {
        match self.verify(ca, bytes, now) {
            Ok(u) => u,
            Err(r) => Upload::Rejected(r),
        }
    }

    fn verify<K: CaKey, A: Attestation>(
        &mut self,
        ca: &CertAuthority<K, A>,
        bytes: &[u8],
        now: f64,
    ) -> Result<Upload, Reject> {
        let frame = Frame::decode(bytes).map_err(|_| Reject::Malformed)?;
        let msg = Rsm::decode(&frame.body).map_err(|_| Reject::Malformed)?;
        let env = AuthEnvelope::decode(&frame.auth).map_err(|_| Reject::Malformed)?;
        // the certificate: one we issued (a full copy must be byte-identical)
        let digest = env.cert.digest();
        let cert = ca.certificate(&digest).ok_or(Reject::UnknownCertificate)?;
        if let CertRef::Full(c) = &env.cert
            && c != cert
        {
            return Err(Reject::ForgedCertificate);
        }
        if pki::temp_id_of(&digest) != msg.temp_id {
            return Err(Reject::ForgedCertificate);
        }
        if !cert.valid_at(now, self.cfg.security.clock_skew_s) {
            return Err(Reject::Expired);
        }
        if ca.is_revoked(&digest) {
            return Err(Reject::Revoked);
        }
        // freshness (SPEC §5 rule 7) before the costly signature check
        let age = rsm::age_s(rsm::ms_of_hour(now), msg.timestamp_ms);
        let max_age = if msg.msg_type == MsgType::Event {
            self.cfg.event_max_age_s
        } else {
            self.cfg.far_max_age_s
        };
        if !(-self.cfg.security.clock_skew_s..=max_age).contains(&age) {
            return Err(Reject::Stale);
        }
        let key = pki::parse_public_key(&cert.public_key).map_err(|_| Reject::Malformed)?;
        if !pki::verify_rsm_signature(&key, now, &frame.body, &env.signature) {
            return Err(Reject::BadSignature);
        }
        // duplicates: the same message uploaded by several phones
        self.prune(now);
        let k = (msg.temp_id, msg.msg_id);
        if self.seen.contains_key(&k) {
            return Err(Reject::Duplicate);
        }
        self.seen.insert(k, now);
        if msg.msg_type != MsgType::Event {
            return Ok(Upload::Heartbeat);
        }
        let cell =
            geohash::encode(msg.lat_deg, msg.lon_deg, geohash::LEVEL).ok_or(Reject::Malformed)?;
        let out = Frame {
            last_hop: None,
            body: frame.body,
            auth: frame.auth,
        };
        Ok(Upload::Event {
            cell,
            frame: out.encode().ok_or(Reject::Malformed)?,
        })
    }

    fn prune(&mut self, now: f64) {
        if now - self.last_prune < 1.0 {
            return;
        }
        self.last_prune = now;
        let keep = self.cfg.duplicate_cache_s.max(self.cfg.event_max_age_s);
        self.seen.retain(|_, t| now - *t <= keep);
    }

    /// Entries in the duplicate cache.
    pub fn cached(&self) -> usize {
        self.seen.len()
    }
}
