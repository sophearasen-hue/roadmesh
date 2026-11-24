//! Developer certificate authority for Android/iOS field tests (BUILD_PLAN Step 5).
//!
//! NOT the product CA: that is the server with its key in a KMS/HSM and App Attest /
//! Play Integrity checks (Step 9). This tool makes a key at run time on the developer's
//! computer, stores it outside the repository, accepts every request, and only debug
//! builds of the app trust it (`-Proadmesh.devCaAnchor=<anchor>`).
//!
//! ```text
//! devca init  <dir>                       make the CA key, print the trust anchor
//! devca anchor <dir>                      print the trust anchor again
//! devca issue <dir> <request.bin> <certificates.bin>
//! ```

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::Generate;
use roadmesh_core::config::Config;
use roadmesh_core::pki;
use roadmesh_server::ca::{Attestation, CaError, CaKey, CertAuthority, IssueRequest};

struct DevKey(SigningKey);

impl CaKey for DevKey {
    fn key_id(&self) -> u8 {
        0xde // never a production key id
    }
    fn public_key(&self) -> [u8; 33] {
        let mut o = [0u8; 33];
        o.copy_from_slice(self.0.verifying_key().to_sec1_point(true).as_bytes());
        o
    }
    fn sign(&self, m: &[u8]) -> Result<[u8; 64], CaError> {
        let s: Signature = self.0.sign(m);
        let mut o = [0u8; 64];
        o.copy_from_slice(&s.to_bytes());
        Ok(o)
    }
}

/// Field tests only: every phone is accepted (Play Integrity / App Attest is Step 9).
struct AcceptForFieldTest;

impl Attestation for AcceptForFieldTest {
    fn verify(&self, _: &[u8; 33], _: &[u8], _: &[u8]) -> bool {
        true
    }
}

fn key_path(dir: &Path) -> PathBuf {
    dir.join("devca-key.bin")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn load(dir: &Path) -> Result<DevKey, String> {
    let bytes =
        std::fs::read(key_path(dir)).map_err(|e| format!("no CA key in {}: {e}", dir.display()))?;
    SigningKey::from_slice(&bytes)
        .map(DevKey)
        .map_err(|_| "the CA key file is damaged".to_owned())
}

fn anchor(k: &DevKey) -> String {
    let mut a = vec![k.key_id()];
    a.extend(k.public_key());
    hex(&a)
}

fn run(args: &[String]) -> Result<String, String> {
    match args {
        [cmd, dir] if cmd == "init" => {
            let dir = Path::new(dir);
            if key_path(dir).exists() {
                return Err("a CA key already exists there".into());
            }
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let k = DevKey(SigningKey::generate());
            std::fs::write(key_path(dir), k.0.to_bytes()).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(key_path(dir), std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
            }
            Ok(format!(
                "developer CA created; keep {} out of git\ntrust anchor (debug builds): {}",
                key_path(dir).display(),
                anchor(&k)
            ))
        }
        [cmd, dir] if cmd == "anchor" => Ok(anchor(&load(Path::new(dir))?)),
        [cmd, dir, req, out] if cmd == "issue" => {
            let k = load(Path::new(dir))?;
            let bytes = std::fs::read(req).map_err(|e| e.to_string())?;
            let r = IssueRequest::decode(&bytes).map_err(|e| format!("bad request: {e:?}"))?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs() as u32;
            let mut ca = CertAuthority::new(k, AcceptForFieldTest, Config::spec_v0());
            let certs = ca.issue(&r, now).map_err(|e| format!("refused: {e:?}"))?;
            std::fs::write(out, pki::encode_certificates(&certs)).map_err(|e| e.to_string())?;
            Ok(format!("issued {} certificates, valid for 7 days", certs.len()))
        }
        _ => Err("usage: devca init <dir> | devca anchor <dir> | devca issue <dir> <request.bin> <certificates.bin>".into()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(msg) => println!("{msg}"),
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]
    use super::*;
    use roadmesh_core::pki::{CertVerifier, IssueRequestWire, TrustAnchor};

    #[test]
    fn issues_certificates_the_core_accepts() {
        let dir = std::env::temp_dir().join(format!("devca-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let d = dir.to_string_lossy().to_string();
        let out = run(&["init".into(), d.clone()]).unwrap_or_else(|e| panic!("{e}"));
        assert!(out.contains("trust anchor"));
        // a request exactly as the phone builds it
        let device = SigningKey::generate();
        let keys: Vec<SigningKey> = (0..2).map(|_| SigningKey::generate()).collect();
        let pk = |k: &SigningKey| {
            let mut o = [0u8; 33];
            o.copy_from_slice(k.verifying_key().to_sec1_point(true).as_bytes());
            o
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()) as u32;
        let pubs: Vec<[u8; 33]> = keys.iter().map(pk).collect();
        let s: Signature = device.sign(&pki::issue_tbs(&pk(&device), now, &pubs));
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&s.to_bytes());
        let wire = IssueRequestWire {
            device_key: pk(&device),
            requested_at: now,
            pseudonym_keys: pubs,
            attestation: vec![],
            signature: sig,
        }
        .encode()
        .unwrap_or_else(|| panic!("encode"));
        let req = dir.join("req.bin");
        let certs = dir.join("certs.bin");
        std::fs::write(&req, wire).unwrap_or_else(|e| panic!("{e}"));
        let msg = run(&[
            "issue".into(),
            d.clone(),
            req.to_string_lossy().into(),
            certs.to_string_lossy().into(),
        ])
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(msg.contains("issued 2"));
        let batch = pki::decode_certificates(&std::fs::read(&certs).unwrap_or_default())
            .unwrap_or_else(|e| panic!("{e:?}"));
        let k = load(&dir).unwrap_or_else(|e| panic!("{e}"));
        let mut v = CertVerifier::new(
            &[TrustAnchor {
                key_id: k.key_id(),
                public_key: k.public_key(),
            }],
            &Config::spec_v0().security,
        )
        .unwrap_or_else(|_| panic!("anchor"));
        for c in &batch {
            assert!(v.check_certificate(c, f64::from(now)).is_ok());
        }
        // refuses to overwrite an existing key
        assert!(run(&["init".into(), d]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
