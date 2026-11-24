//! RoadMesh edge server (BUILD_PLAN Step 9). See docs/SERVER.md for deployment.
//!
//! `roadmesh-server pins <server-cert.pem>`: print the SPKI pin (base64 SHA-256) of a
//! certificate for the app configuration.
//! `roadmesh-server`: run, configured by environment variables (docs/SERVER.md). It
//! refuses to start without the `aws-kms` feature: CA keys live only in KMS.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;

fn pins(path: &str) -> Result<(), String> {
    for c in CertificateDer::pem_file_iter(path).map_err(|e| e.to_string())? {
        let c = c.map_err(|e| e.to_string())?;
        let pin = roadmesh_server::tls::spki_sha256(&c).ok_or("not a certificate")?;
        println!("{}", STANDARD.encode(pin));
    }
    Ok(())
}

#[cfg(not(feature = "aws-kms"))]
async fn run() -> Result<(), String> {
    Err("built without the aws-kms feature: CA keys must be in AWS KMS (AUDIT S-19)".into())
}

#[cfg(feature = "aws-kms")]
async fn run() -> Result<(), String> {
    use std::sync::Arc;

    use roadmesh_core::config::Config;
    use roadmesh_server::attest::{
        APPLE_ROOT_PEM, AppleConfig, DeviceAttestation, PlayConfig, pem_to_der,
    };
    use roadmesh_server::broker::{Broker, Settings};
    use roadmesh_server::ca::CertAuthority;
    use roadmesh_server::devca::DeviceCa;
    use roadmesh_server::hazard::HazardGate;
    use roadmesh_server::kms::KmsKey;
    use roadmesh_server::monitor::Monitor;
    use roadmesh_server::serve::{Port, accept_loop};
    use roadmesh_server::tls;
    use rustls_pki_types::PrivateKeyDer;

    fn env(k: &str) -> Result<String, String> {
        std::env::var(k).map_err(|_| format!("{k} is not set"))
    }
    fn file(k: &str) -> Result<Vec<u8>, String> {
        let p = env(k)?;
        std::fs::read(&p).map_err(|e| format!("{k}: {e}"))
    }
    fn now_s() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(env("RM_TLS_CERT")?)
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let key = PrivateKeyDer::from_pem_file(env("RM_TLS_KEY")?).map_err(|e| e.to_string())?;

    let aws = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_kms::Client::new(&aws);
    let ca_key_id: u8 = env("RM_CA_KEY_ID")?
        .parse()
        .map_err(|_| "RM_CA_KEY_ID: 0-255")?;
    let ca_key = KmsKey::connect(client.clone(), &env("RM_CA_KMS_KEY")?, ca_key_id)
        .await
        .map_err(|e| format!("CA key: {e:?}"))?;
    let dev_key = KmsKey::connect(client, &env("RM_DEVICE_CA_KMS_KEY")?, 0)
        .await
        .map_err(|e| format!("device CA key: {e:?}"))?;

    let apple = match std::env::var("RM_APPLE_APP_ID") {
        Ok(app_id) => Some(AppleConfig {
            app_id,
            root_der: pem_to_der(APPLE_ROOT_PEM).ok_or("Apple root")?,
            production: std::env::var("RM_APPLE_DEVELOPMENT").is_err(),
        }),
        Err(_) => None,
    };
    let play = match std::env::var("RM_PLAY_PACKAGE") {
        Ok(package) => {
            use p256::pkcs8::DecodePublicKey;
            let dec = STANDARD
                .decode(String::from_utf8_lossy(&file("RM_PLAY_DECRYPTION_KEY_FILE")?).trim())
                .map_err(|_| "Play decryption key: base64")?;
            let ver = STANDARD
                .decode(String::from_utf8_lossy(&file("RM_PLAY_VERIFICATION_KEY_FILE")?).trim())
                .map_err(|_| "Play verification key: base64")?;
            Some(PlayConfig {
                package,
                cert_digests: env("RM_PLAY_CERT_DIGESTS")?
                    .split(',')
                    .map(str::to_owned)
                    .collect(),
                decryption_key: dec
                    .try_into()
                    .map_err(|_| "Play decryption key: 32 bytes")?,
                verification_key: p256::ecdsa::VerifyingKey::from_public_key_der(&ver)
                    .map_err(|_| "Play verification key: P-256 SPKI")?,
                max_age_ms: 10 * 60 * 1000,
            })
        }
        Err(_) => None,
    };
    if apple.is_none() && play.is_none() {
        return Err("set RM_APPLE_APP_ID and/or RM_PLAY_PACKAGE: no device could enrol".into());
    }
    let attestation = DeviceAttestation::new(apple, play, Box::new(|| (now_s() * 1000.0) as i64));

    // the device CA signs its root through KMS: do it on a blocking thread
    let devca = tokio::task::spawn_blocking(move || DeviceCa::new(dev_key))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("device CA: {e:?}"))?;
    let cfg = Config::spec_v0();
    let ca = CertAuthority::new(ca_key, attestation, cfg.clone());
    let monitor = Arc::new(Monitor::new());
    let broker = Broker::new(
        ca,
        devca,
        HazardGate::new(cfg),
        Settings::default(),
        monitor,
        Box::new(now_s),
    );
    let mqtt_cfg = tls::mqtt_config(chain.clone(), key.clone_key(), &broker.device_root())
        .map_err(|e| e.to_string())?;
    let enrol_cfg = tls::enrol_config(chain, key).map_err(|e| e.to_string())?;
    let mqtt_addr = std::env::var("RM_MQTT_ADDR").unwrap_or_else(|_| "0.0.0.0:8883".into());
    let enrol_addr = std::env::var("RM_ENROL_ADDR").unwrap_or_else(|_| "0.0.0.0:8884".into());
    let m = tokio::net::TcpListener::bind(&mqtt_addr)
        .await
        .map_err(|e| e.to_string())?;
    let e = tokio::net::TcpListener::bind(&enrol_addr)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{{\"started\":\"roadmesh-server {}\",\"trust_anchor\":\"{}\"}}",
        env!("CARGO_PKG_VERSION"),
        STANDARD.encode(broker.trust_anchor())
    );
    tokio::spawn(accept_loop(broker.clone(), m, mqtt_cfg, Port::Mqtt));
    tokio::spawn(accept_loop(broker.clone(), e, enrol_cfg, Port::Enrol));
    let b = broker.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let b = b.clone();
            let _ = tokio::task::spawn_blocking(move || b.tick()).await;
        }
    });
    tokio::signal::ctrl_c().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("pins") => args
            .get(2)
            .map_or(Err("usage: roadmesh-server pins <cert.pem>".into()), |p| {
                pins(p)
            }),
        Some(_) => Err("usage: roadmesh-server [pins <cert.pem>]".into()),
        None => run().await,
    };
    if let Err(e) = result {
        eprintln!("roadmesh-server: {e}");
        std::process::exit(1);
    }
}
