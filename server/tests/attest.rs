//! App Attest and Play Integrity verification (AUDIT S-16). The evidence is built here
//! the way Apple and Google build it, with throwaway keys standing in for theirs.
#![allow(clippy::panic, clippy::unwrap_used)]

use aes_kw::KeyInit;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ciborium::Value;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::Generate;
use rcgen::{
    BasicConstraints, CertificateParams, CustomExtension, IsCa, Issuer, KeyPair,
    PKCS_ECDSA_P256_SHA256, PKCS_ECDSA_P384_SHA384,
};
use roadmesh_core::pki::PUBLIC_KEY_LEN;
use roadmesh_server::attest::{
    APPLE_ROOT_PEM, AppleConfig, DeviceAttestation, PlayConfig, kind, pem_to_der,
};
use roadmesh_server::ca::Attestation;
use sha2::{Digest, Sha256};

const APP_ID: &str = "ABCDE12345.com.roadmesh.app";
const NOW_MS: i64 = 1_790_000_000_000;

fn sha(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

fn device_key() -> [u8; PUBLIC_KEY_LEN] {
    let k = SigningKey::generate();
    let mut o = [0u8; PUBLIC_KEY_LEN];
    o.copy_from_slice(k.verifying_key().to_sec1_point(true).as_bytes());
    o
}

#[test]
fn embedded_apple_root_is_apples() {
    let der = pem_to_der(APPLE_ROOT_PEM).unwrap();
    let fp: String = sha(&[&der]).iter().map(|b| format!("{b:02X}")).collect();
    assert_eq!(
        fp,
        "1CB9823BA28BA6AD2D33A006941DE2AE4F513EF1D4E831B9F7E0FA7B6242C932"
    );
}

/// A stand-in for Apple's App Attest service.
struct FakeApple {
    root_der: Vec<u8>,
    inter: Issuer<'static, KeyPair>,
}

impl FakeApple {
    /// Attestation object for a new App Attest key over `challenge`.
    fn attest(&self, challenge: &[u8], counter: u32, app_id: &str) -> (Vec<u8>, KeyPair, Vec<u8>) {
        let cred = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let point = cred.public_key_raw().to_vec();
        let key_id = sha(&[&point]);
        let mut auth = sha(&[app_id.as_bytes()]).to_vec();
        auth.push(0x40);
        auth.extend_from_slice(&counter.to_be_bytes());
        auth.extend_from_slice(b"appattestdevelop");
        auth.extend_from_slice(&32u16.to_be_bytes());
        auth.extend_from_slice(&key_id);
        let nonce = sha(&[&auth, &sha(&[challenge])]);
        let mut ext = vec![0x30, 0x24, 0xa1, 0x22, 0x04, 0x20];
        ext.extend_from_slice(&nonce);
        let mut lp = CertificateParams::new(Vec::<String>::new()).unwrap();
        lp.custom_extensions = vec![CustomExtension::from_oid_content(
            &[1, 2, 840, 113635, 100, 8, 2],
            ext,
        )];
        let leaf = lp.signed_by(&cred, &self.inter).unwrap().der().to_vec();
        let obj = Value::Map(vec![
            (
                Value::Text("fmt".into()),
                Value::Text("apple-appattest".into()),
            ),
            (
                Value::Text("attStmt".into()),
                Value::Map(vec![
                    (
                        Value::Text("x5c".into()),
                        Value::Array(vec![Value::Bytes(leaf), Value::Bytes(self.inter_der())]),
                    ),
                    (Value::Text("receipt".into()), Value::Bytes(vec![1, 2, 3])),
                ]),
            ),
            (Value::Text("authData".into()), Value::Bytes(auth)),
        ]);
        let mut out = vec![kind::APPLE_ATTESTATION];
        out.extend_from_slice(&key_id);
        ciborium::into_writer(&obj, &mut out).unwrap();
        (out, cred, key_id.to_vec())
    }

    /// The intermediate certificate as Apple sends it (made once in `fake_apple`).
    fn inter_der(&self) -> Vec<u8> {
        INTER.with(|c| c.borrow().clone())
    }
}

thread_local! {
    static INTER: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn fake_apple() -> FakeApple {
    let mut rp = CertificateParams::new(Vec::<String>::new()).unwrap();
    rp.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let rk = KeyPair::generate_for(&PKCS_ECDSA_P384_SHA384).unwrap();
    let root_der = rp.self_signed(&rk).unwrap().der().to_vec();
    let root = Issuer::new(rp, rk);
    let mut ip = CertificateParams::new(Vec::<String>::new()).unwrap();
    ip.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    let ik = KeyPair::generate_for(&PKCS_ECDSA_P384_SHA384).unwrap();
    let inter_der = ip.signed_by(&ik, &root).unwrap().der().to_vec();
    INTER.with(|c| *c.borrow_mut() = inter_der);
    FakeApple {
        root_der,
        inter: Issuer::new(ip, ik),
    }
}

fn now_real_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn apple_verifier(root_der: Vec<u8>) -> DeviceAttestation {
    DeviceAttestation::new(
        Some(AppleConfig {
            app_id: APP_ID.into(),
            root_der,
            production: false,
        }),
        None,
        Box::new(now_real_ms),
    )
}

fn assertion(cred: &KeyPair, key_id: &[u8], challenge: &[u8], counter: u32) -> Vec<u8> {
    let mut auth = sha(&[APP_ID.as_bytes()]).to_vec();
    auth.push(0);
    auth.extend_from_slice(&counter.to_be_bytes());
    let nonce = sha(&[&auth, &sha(&[challenge])]);
    let sk = SigningKey::from_slice(&cred_secret(cred)).unwrap();
    let sig: Signature = sk.sign(&nonce);
    let obj = Value::Map(vec![
        (
            Value::Text("signature".into()),
            Value::Bytes(sig.to_der().as_bytes().to_vec()),
        ),
        (Value::Text("authenticatorData".into()), Value::Bytes(auth)),
    ]);
    let mut out = vec![kind::APPLE_ASSERTION];
    out.extend_from_slice(key_id);
    ciborium::into_writer(&obj, &mut out).unwrap();
    out
}

/// The private scalar of an rcgen P-256 key (from its PKCS#8 DER).
fn cred_secret(k: &KeyPair) -> Vec<u8> {
    use p256::pkcs8::DecodePrivateKey;
    let sk = p256::SecretKey::from_pkcs8_der(&k.serialize_der()).unwrap();
    sk.to_bytes().to_vec()
}

#[test]
fn app_attest_attestation_then_assertions() {
    let apple = fake_apple();
    let v = apple_verifier(apple.root_der.clone());
    let dk = device_key();
    let challenge = b"issue request bytes";
    let (ev, cred, key_id) = apple.attest(challenge, 0, APP_ID);
    assert!(v.verify(&dk, challenge, &ev));
    // bound to the challenge: the same evidence for another request fails
    assert!(!v.verify(&dk, b"another request", &ev));
    // the same App Attest key cannot vouch for a second device key
    assert!(!v.verify(&device_key(), challenge, &ev));
    // renewals use assertions with a growing counter
    let next = b"renewal request";
    assert!(v.verify(&dk, next, &assertion(&cred, &key_id, next, 1)));
    assert!(
        !v.verify(&dk, next, &assertion(&cred, &key_id, next, 1)),
        "replayed counter"
    );
    assert!(
        !v.verify(&dk, b"other", &assertion(&cred, &key_id, next, 2)),
        "wrong challenge"
    );
    assert!(
        !v.verify(&device_key(), next, &assertion(&cred, &key_id, next, 3)),
        "other device"
    );
    assert!(v.verify(&dk, next, &assertion(&cred, &key_id, next, 3)));
}

#[test]
fn app_attest_rejects_wrong_app_root_or_counter() {
    let apple = fake_apple();
    let challenge = b"issue request bytes";
    // another app's id
    let (ev, _, _) = apple.attest(challenge, 0, "OTHER.com.example");
    assert!(!apple_verifier(apple.root_der.clone()).verify(&device_key(), challenge, &ev));
    // a non-zero counter on attestation
    let (ev, _, _) = apple.attest(challenge, 1, APP_ID);
    assert!(!apple_verifier(apple.root_der.clone()).verify(&device_key(), challenge, &ev));
    // a chain to a different root (e.g. Apple's real root against our fake chain)
    let (ev, _, _) = apple.attest(challenge, 0, APP_ID);
    let real = pem_to_der(APPLE_ROOT_PEM).unwrap();
    assert!(!apple_verifier(real).verify(&device_key(), challenge, &ev));
    // a changed byte anywhere
    let (mut ev, _, _) = apple.attest(challenge, 0, APP_ID);
    let n = ev.len();
    ev[n - 40] ^= 1;
    assert!(!apple_verifier(apple.root_der.clone()).verify(&device_key(), challenge, &ev));
    // unknown kinds and empty evidence
    let v = apple_verifier(apple.root_der);
    assert!(
        !v.verify(&device_key(), challenge, &[]) && !v.verify(&device_key(), challenge, &[9, 1])
    );
}

// ---------------------------------------------------------------- Play Integrity

struct FakeGoogle {
    aes: [u8; 32],
    sign: SigningKey,
}

impl FakeGoogle {
    fn new() -> Self {
        let mut aes = [0u8; 32];
        aes.copy_from_slice(&sha(&[&SigningKey::generate().to_bytes()]));
        Self {
            aes,
            sign: SigningKey::generate(),
        }
    }

    fn config(&self) -> PlayConfig {
        PlayConfig {
            package: "com.roadmesh.app".into(),
            cert_digests: vec!["c2lnbmluZy1jZXJ0".into()],
            decryption_key: self.aes,
            verification_key: *self.sign.verifying_key(),
            max_age_ms: 10 * 60 * 1000,
        }
    }

    /// An integrity token as Google returns it: JWS(ES256) inside JWE(A256KW, A256GCM).
    fn token(&self, payload: &serde_json::Value) -> Vec<u8> {
        let b64 = |b: &[u8]| URL_SAFE_NO_PAD.encode(b);
        let jh = b64(br#"{"alg":"ES256"}"#);
        let jp = b64(payload.to_string().as_bytes());
        let signed = format!("{jh}.{jp}");
        let sig: Signature = self.sign.sign(signed.as_bytes());
        let jws = format!("{signed}.{}", b64(&sig.to_bytes()));
        let cek = sha(&[b"cek", &self.aes]);
        let kw = aes_kw::KwAes256::new_from_slice(&self.aes).unwrap();
        let mut wrapped = [0u8; 40];
        let wrapped = kw.wrap_key(&cek, &mut wrapped).unwrap().to_vec();
        let header = b64(br#"{"alg":"A256KW","enc":"A256GCM"}"#);
        let iv = [7u8; 12];
        let key = ring::aead::LessSafeKey::new(
            ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, &cek).unwrap(),
        );
        let mut data = jws.into_bytes();
        let tag = key
            .seal_in_place_separate_tag(
                ring::aead::Nonce::assume_unique_for_key(iv),
                ring::aead::Aad::from(header.as_bytes()),
                &mut data,
            )
            .unwrap();
        let mut out = vec![kind::PLAY_INTEGRITY];
        out.extend_from_slice(
            format!(
                "{header}.{}.{}.{}.{}",
                b64(&wrapped),
                b64(&iv),
                b64(&data),
                b64(tag.as_ref())
            )
            .as_bytes(),
        );
        out
    }
}

fn verdict(challenge: &[u8], ts: i64) -> serde_json::Value {
    serde_json::json!({
        "requestDetails": {
            "requestPackageName": "com.roadmesh.app",
            "nonce": URL_SAFE_NO_PAD.encode(sha(&[challenge])),
            "timestampMillis": ts.to_string()
        },
        "appIntegrity": {
            "appRecognitionVerdict": "PLAY_RECOGNIZED",
            "packageName": "com.roadmesh.app",
            "certificateSha256Digest": ["c2lnbmluZy1jZXJ0"],
            "versionCode": "1"
        },
        "deviceIntegrity": { "deviceRecognitionVerdict": ["MEETS_DEVICE_INTEGRITY"] },
        "accountDetails": { "appLicensingVerdict": "LICENSED" }
    })
}

fn play_verifier(g: &FakeGoogle) -> DeviceAttestation {
    DeviceAttestation::new(None, Some(g.config()), Box::new(|| NOW_MS))
}

#[test]
fn play_integrity_genuine_device_passes() {
    let g = FakeGoogle::new();
    let c = b"issue request bytes";
    assert!(play_verifier(&g).verify(&device_key(), c, &g.token(&verdict(c, NOW_MS - 1000))));
}

#[test]
fn play_integrity_failures() {
    let g = FakeGoogle::new();
    let v = play_verifier(&g);
    let c = b"issue request bytes";
    let dk = device_key();
    let mut cases: Vec<(&str, serde_json::Value)> = Vec::new();
    let mut x = verdict(c, NOW_MS);
    x["deviceIntegrity"]["deviceRecognitionVerdict"] = serde_json::json!(["MEETS_BASIC_INTEGRITY"]);
    cases.push(("rooted or emulator", x));
    let mut x = verdict(c, NOW_MS);
    x["appIntegrity"]["appRecognitionVerdict"] = serde_json::json!("UNRECOGNIZED_VERSION");
    cases.push(("modified app", x));
    let mut x = verdict(c, NOW_MS);
    x["appIntegrity"]["certificateSha256Digest"] = serde_json::json!(["b3RoZXI"]);
    cases.push(("re-signed app", x));
    cases.push(("other request", verdict(b"another request", NOW_MS)));
    cases.push(("old verdict", verdict(c, NOW_MS - 11 * 60 * 1000)));
    for (why, x) in cases {
        assert!(!v.verify(&dk, c, &g.token(&x)), "{why}");
    }
    // a token from another key pair (not Google's for this app)
    let other = FakeGoogle::new();
    assert!(!v.verify(&dk, c, &other.token(&verdict(c, NOW_MS))));
    // tampered ciphertext
    let mut t = g.token(&verdict(c, NOW_MS));
    let n = t.len();
    t[n - 30] = if t[n - 30] == b'A' { b'B' } else { b'A' };
    assert!(!v.verify(&dk, c, &t));
    // without Play configured, Android evidence is refused
    let none = DeviceAttestation::new(None, None, Box::new(|| NOW_MS));
    assert!(!none.verify(&dk, c, &g.token(&verdict(c, NOW_MS))));
}
