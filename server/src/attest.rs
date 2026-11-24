//! Device attestation before certificates are issued (SPEC §10, AUDIT S-16): Apple App
//! Attest on iPhones and Google Play Integrity on Android. Both prove that the request
//! comes from the genuine RoadMesh app on a real device, and both are bound to the
//! exact certificate request (its signed bytes are the challenge), so evidence cannot
//! be replayed for another device key.
//!
//! Evidence formats (first byte):
//! * `1`: App Attest attestation (first enrolment): `[1][key id 32][attestation object]`.
//! * `2`: App Attest assertion (later renewals): `[2][key id 32][assertion]`.
//! * `3`: Play Integrity token (classic request): `[3][JWE compact, ASCII]`.
//!
//! Each App Attest key may vouch for one device key only, so one genuine iPhone cannot
//! mint many identities. The Play Integrity token is decrypted and verified locally
//! with the keys from the Play Console (Google's documented option).

use std::collections::HashMap;
use std::sync::Mutex;

use aes_kw::KeyInit;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ciborium::Value;
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use roadmesh_core::pki::PUBLIC_KEY_LEN;
use sha2::{Digest, Sha256};
use x509_parser::prelude::{ASN1Time, FromDer, X509Certificate};

use crate::ca::Attestation;

/// Apple App Attestation Root CA (published at apple.com/certificateauthority).
/// SHA-256 fingerprint 1C:B9:82:3B:A2:8B:A6:AD:2D:33:A0:06:94:1D:E2:AE:4F:51:3E:F1:D4:E8:31:B9:F7:E0:FA:7B:62:42:C9:32.
pub const APPLE_ROOT_PEM: &str = include_str!("apple_app_attestation_root_ca.pem");

const OID_APPLE_NONCE: &str = "1.2.840.113635.100.8.2";

/// Evidence kinds.
pub mod kind {
    /// App Attest attestation.
    pub const APPLE_ATTESTATION: u8 = 1;
    /// App Attest assertion.
    pub const APPLE_ASSERTION: u8 = 2;
    /// Play Integrity token.
    pub const PLAY_INTEGRITY: u8 = 3;
}

/// The iOS app.
#[derive(Debug, Clone)]
pub struct AppleConfig {
    /// "TEAMID.bundle.id".
    pub app_id: String,
    /// Root certificate (DER): Apple's in production.
    pub root_der: Vec<u8>,
    /// Production App Attest environment (false: development builds).
    pub production: bool,
}

/// The Android app and the Play Console keys.
#[derive(Clone)]
pub struct PlayConfig {
    /// Package name.
    pub package: String,
    /// Base64 SHA-256 digests of the app signing certificate (as Play reports them).
    pub cert_digests: Vec<String>,
    /// Response decryption key (AES-256), from the Play Console.
    pub decryption_key: [u8; 32],
    /// Response verification key (EC P-256), from the Play Console.
    pub verification_key: VerifyingKey,
    /// Accepted age of a verdict (ms).
    pub max_age_ms: i64,
}

impl std::fmt::Debug for PlayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayConfig")
            .field("package", &self.package)
            .finish_non_exhaustive() // never print the keys
    }
}

struct AppleKey {
    public: VerifyingKey,
    counter: u32,
    device_key: [u8; PUBLIC_KEY_LEN],
}

/// Checks App Attest and Play Integrity evidence.
pub struct DeviceAttestation {
    apple: Option<AppleConfig>,
    play: Option<PlayConfig>,
    keys: Mutex<HashMap<[u8; 32], AppleKey>>,
    clock_ms: Box<dyn Fn() -> i64 + Send + Sync>,
}

/// Parse the PEM of a single certificate to DER.
pub fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .concat();
    STANDARD.decode(body.trim()).ok()
}

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

fn map_get<'a>(m: &'a [(Value, Value)], key: &str) -> Option<&'a Value> {
    m.iter()
        .find(|(k, _)| k.as_text() == Some(key))
        .map(|(_, v)| v)
}

impl DeviceAttestation {
    /// `clock_ms`: Unix time in milliseconds.
    pub fn new(
        apple: Option<AppleConfig>,
        play: Option<PlayConfig>,
        clock_ms: Box<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        Self {
            apple,
            play,
            keys: Mutex::new(HashMap::new()),
            clock_ms,
        }
    }

    fn keys(&self) -> std::sync::MutexGuard<'_, HashMap<[u8; 32], AppleKey>> {
        self.keys.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------------------------------------------------------------- Apple

    fn apple_attestation(
        &self,
        device_key: &[u8; PUBLIC_KEY_LEN],
        challenge: &[u8],
        ev: &[u8],
    ) -> Option<()> {
        let cfg = self.apple.as_ref()?;
        let key_id: [u8; 32] = ev.get(..32)?.try_into().ok()?;
        let obj: Value = ciborium::from_reader(ev.get(32..)?).ok()?;
        let m = obj.as_map()?;
        if map_get(m, "fmt")?.as_text()? != "apple-appattest" {
            return None;
        }
        let auth_data = map_get(m, "authData")?.as_bytes()?;
        let stmt = map_get(m, "attStmt")?.as_map()?;
        let x5c = map_get(stmt, "x5c")?.as_array()?;
        let leaf_der = x5c.first()?.as_bytes()?;
        let inter_der = x5c.get(1)?.as_bytes()?;
        let (_, leaf) = X509Certificate::from_der(leaf_der).ok()?;
        let (_, inter) = X509Certificate::from_der(inter_der).ok()?;
        let (_, root) = X509Certificate::from_der(&cfg.root_der).ok()?;
        // 1. chain to the pinned root, all certificates valid now
        let now = ASN1Time::from_timestamp((self.clock_ms)() / 1000).ok()?;
        for c in [&leaf, &inter] {
            if !c.validity().is_valid_at(now) {
                return None;
            }
        }
        leaf.verify_signature(Some(inter.public_key())).ok()?;
        inter.verify_signature(Some(root.public_key())).ok()?;
        // 2–4. nonce = SHA-256(authData ‖ SHA-256(challenge)) in the leaf's extension
        let nonce = sha256(&[auth_data, &sha256(&[challenge])]);
        let ext = leaf
            .extensions()
            .iter()
            .find(|e| e.oid.to_id_string() == OID_APPLE_NONCE)?;
        let mut expect = vec![0x30, 0x24, 0xa1, 0x22, 0x04, 0x20];
        expect.extend_from_slice(&nonce);
        if ext.value != expect.as_slice() {
            return None;
        }
        // 5. the key id is the SHA-256 of the credential's public key
        let point = leaf.public_key().subject_public_key.data.as_ref();
        if sha256(&[point]) != key_id {
            return None;
        }
        let public = VerifyingKey::from_sec1_bytes(point).ok()?;
        // 6–9. authenticator data: app id, counter 0, environment, credential id
        let (rp, counter) = apple_auth_data(auth_data)?;
        if rp != sha256(&[cfg.app_id.as_bytes()]) || counter != 0 {
            return None;
        }
        let aaguid = auth_data.get(37..53)?;
        let want: &[u8] = if cfg.production {
            b"appattest\0\0\0\0\0\0\0"
        } else {
            b"appattestdevelop"
        };
        if aaguid != want {
            return None;
        }
        let n = usize::from(u16::from_be_bytes([
            *auth_data.get(53)?,
            *auth_data.get(54)?,
        ]));
        if n != 32 || auth_data.get(55..87)? != key_id {
            return None;
        }
        // one App Attest key, one device key
        let mut keys = self.keys();
        if let Some(k) = keys.get(&key_id)
            && k.device_key != *device_key
        {
            return None;
        }
        keys.insert(
            key_id,
            AppleKey {
                public,
                counter: 0,
                device_key: *device_key,
            },
        );
        Some(())
    }

    fn apple_assertion(
        &self,
        device_key: &[u8; PUBLIC_KEY_LEN],
        challenge: &[u8],
        ev: &[u8],
    ) -> Option<()> {
        let cfg = self.apple.as_ref()?;
        let key_id: [u8; 32] = ev.get(..32)?.try_into().ok()?;
        let obj: Value = ciborium::from_reader(ev.get(32..)?).ok()?;
        let m = obj.as_map()?;
        let sig = Signature::from_der(map_get(m, "signature")?.as_bytes()?).ok()?;
        let auth_data = map_get(m, "authenticatorData")?.as_bytes()?;
        let mut keys = self.keys();
        let k = keys.get_mut(&key_id)?;
        if k.device_key != *device_key {
            return None;
        }
        let nonce = sha256(&[auth_data, &sha256(&[challenge])]);
        k.public.verify(&nonce, &sig).ok()?;
        let (rp, counter) = apple_auth_data(auth_data)?;
        if rp != sha256(&[cfg.app_id.as_bytes()]) || counter <= k.counter {
            return None;
        }
        k.counter = counter;
        Some(())
    }

    // ---------------------------------------------------------------- Google

    fn play_integrity(&self, challenge: &[u8], ev: &[u8]) -> Option<()> {
        let cfg = self.play.as_ref()?;
        let token = std::str::from_utf8(ev).ok()?;
        let payload = decrypt_and_verify(cfg, token)?;
        let v: serde_json::Value = serde_json::from_slice(&payload).ok()?;
        let req = v.get("requestDetails")?;
        if req.get("requestPackageName")?.as_str()? != cfg.package {
            return None;
        }
        if req.get("nonce")?.as_str()? != URL_SAFE_NO_PAD.encode(sha256(&[challenge])) {
            return None;
        }
        let ts: i64 = req.get("timestampMillis")?.as_str()?.parse().ok()?;
        if ((self.clock_ms)() - ts).abs() > cfg.max_age_ms {
            return None;
        }
        let app = v.get("appIntegrity")?;
        if app.get("appRecognitionVerdict")?.as_str()? != "PLAY_RECOGNIZED"
            || app.get("packageName")?.as_str()? != cfg.package
        {
            return None;
        }
        let digests = app.get("certificateSha256Digest")?.as_array()?;
        if !digests
            .iter()
            .filter_map(|d| d.as_str())
            .any(|d| cfg.cert_digests.iter().any(|c| c == d))
        {
            return None;
        }
        let device = v
            .get("deviceIntegrity")?
            .get("deviceRecognitionVerdict")?
            .as_array()?;
        if !device.iter().any(|d| {
            d.as_str() == Some("MEETS_DEVICE_INTEGRITY")
                || d.as_str() == Some("MEETS_STRONG_INTEGRITY")
        }) {
            return None;
        }
        Some(())
    }
}

/// (rpIdHash, counter) from App Attest authenticator data.
fn apple_auth_data(a: &[u8]) -> Option<([u8; 32], u32)> {
    let rp: [u8; 32] = a.get(..32)?.try_into().ok()?;
    let c = a.get(33..37)?;
    Some((rp, u32::from_be_bytes([c[0], c[1], c[2], c[3]])))
}

/// Play Integrity response: JWE (A256KW, A256GCM) around a JWS (ES256).
fn decrypt_and_verify(cfg: &PlayConfig, token: &str) -> Option<Vec<u8>> {
    let parts: Vec<&str> = token.split('.').collect();
    let [header, ekey, iv, ct, tag] = parts.as_slice() else {
        return None;
    };
    let h: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header).ok()?).ok()?;
    if h.get("alg")?.as_str()? != "A256KW" || h.get("enc")?.as_str()? != "A256GCM" {
        return None;
    }
    let wrapped = URL_SAFE_NO_PAD.decode(ekey).ok()?;
    let kw = aes_kw::KwAes256::new_from_slice(&cfg.decryption_key).ok()?;
    let mut cek = [0u8; 32];
    let cek = kw.unwrap_key(&wrapped, &mut cek).ok()?;
    let iv: [u8; 12] = URL_SAFE_NO_PAD.decode(iv).ok()?.try_into().ok()?;
    let mut data = URL_SAFE_NO_PAD.decode(ct).ok()?;
    data.extend_from_slice(&URL_SAFE_NO_PAD.decode(tag).ok()?);
    let key = ring::aead::LessSafeKey::new(
        ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, cek).ok()?,
    );
    let plain = key
        .open_in_place(
            ring::aead::Nonce::assume_unique_for_key(iv),
            ring::aead::Aad::from(header.as_bytes()),
            &mut data,
        )
        .ok()?;
    let jws = std::str::from_utf8(plain).ok()?;
    let (signed, sig) = jws.rsplit_once('.')?;
    let (jh, payload) = signed.split_once('.')?;
    let jh: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(jh).ok()?).ok()?;
    if jh.get("alg")?.as_str()? != "ES256" {
        return None;
    }
    let sig = Signature::from_slice(&URL_SAFE_NO_PAD.decode(sig).ok()?).ok()?;
    cfg.verification_key.verify(signed.as_bytes(), &sig).ok()?;
    URL_SAFE_NO_PAD.decode(payload).ok()
}

impl Attestation for DeviceAttestation {
    fn verify(&self, device_key: &[u8; PUBLIC_KEY_LEN], challenge: &[u8], evidence: &[u8]) -> bool {
        let Some((&k, rest)) = evidence.split_first() else {
            return false;
        };
        match k {
            kind::APPLE_ATTESTATION => self.apple_attestation(device_key, challenge, rest),
            kind::APPLE_ASSERTION => self.apple_assertion(device_key, challenge, rest),
            kind::PLAY_INTEGRITY => self.play_integrity(challenge, rest),
            _ => None,
        }
        .is_some()
    }
}
