//! CA keys in AWS KMS (AUDIT S-19): the pseudonym CA and the device CA each sign with
//! an `ECC_NIST_P256` / `SIGN_VERIFY` key that never leaves KMS. Grant the server's
//! role `kms:Sign` and `kms:GetPublicKey` on those two keys only; changes to the key
//! policy need two people (AWS IAM approval workflow, see docs/SERVER.md).
//!
//! [`CaKey::sign`] blocks on the KMS call: the broker calls it only from blocking
//! threads (`spawn_blocking`), never on the async workers.

use aws_sdk_kms::Client;
use aws_sdk_kms::primitives::Blob;
use aws_sdk_kms::types::{KeySpec, MessageType, SigningAlgorithmSpec};
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use roadmesh_core::pki::{PUBLIC_KEY_LEN, SIG_LEN};

use crate::ca::{CaError, CaKey};

/// A P-256 signing key in AWS KMS.
pub struct KmsKey {
    client: Client,
    key: String,
    key_id: u8,
    public: [u8; PUBLIC_KEY_LEN],
    rt: tokio::runtime::Handle,
}

/// Why a KMS key could not be used.
#[derive(Debug)]
pub enum KmsError {
    /// The call failed (network, permissions, unknown key).
    Call(String),
    /// Not an ECC_NIST_P256 signing key.
    WrongKeyType,
}

impl KmsKey {
    /// Look up `key` (ARN or alias) and check it is a P-256 signing key. `key_id` is
    /// the one-byte id written into certificates.
    pub async fn connect(client: Client, key: &str, key_id: u8) -> Result<Self, KmsError> {
        let out = client
            .get_public_key()
            .key_id(key)
            .send()
            .await
            .map_err(|e| KmsError::Call(e.to_string()))?;
        if out.key_spec() != Some(&KeySpec::EccNistP256) {
            return Err(KmsError::WrongKeyType);
        }
        let der = out.public_key().ok_or(KmsError::WrongKeyType)?.as_ref();
        let vk = VerifyingKey::from_public_key_der(der).map_err(|_| KmsError::WrongKeyType)?;
        let mut public = [0u8; PUBLIC_KEY_LEN];
        public.copy_from_slice(vk.to_sec1_point(true).as_bytes());
        Ok(Self {
            client,
            key: key.to_owned(),
            key_id,
            public,
            rt: tokio::runtime::Handle::current(),
        })
    }
}

impl CaKey for KmsKey {
    fn key_id(&self) -> u8 {
        self.key_id
    }

    fn public_key(&self) -> [u8; PUBLIC_KEY_LEN] {
        self.public
    }

    fn sign(&self, message: &[u8]) -> Result<[u8; SIG_LEN], CaError> {
        let call = self
            .client
            .sign()
            .key_id(&self.key)
            .message(Blob::new(message))
            .message_type(MessageType::Raw)
            .signing_algorithm(SigningAlgorithmSpec::EcdsaSha256)
            .send();
        let out = self.rt.block_on(call).map_err(|_| CaError::Unavailable)?;
        let der = out.signature().ok_or(CaError::Unavailable)?.as_ref();
        let sig = Signature::from_der(der).map_err(|_| CaError::Unavailable)?;
        let sig = sig.normalize_s();
        let mut raw = [0u8; SIG_LEN];
        raw.copy_from_slice(&sig.to_bytes());
        Ok(raw)
    }
}
