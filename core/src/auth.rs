//! Message authentication interfaces (SPEC §10).
//!
//! The core never holds private keys. [`crate::pki::PseudonymSigner`] implements
//! [`Signer`] on top of the platform's hardware key store (Secure Enclave /
//! Android Keystore, AUDIT S-02); [`crate::pki::CertVerifier`] implements
//! [`Verifier`] with pseudonym certificates, the trusted CA keys and the
//! revocation list. No shared, hard-coded or test keys exist in this crate.

use crate::rsm::{BODY_LEN, TempId};

/// Why a message failed authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    /// The message names a certificate this phone has not seen yet (digest only).
    UnknownCertificate,
    /// The certificate is not signed by a trusted CA key, or its key is invalid.
    BadCertificate,
    /// The certificate is not valid at this time.
    ExpiredCertificate,
    /// The sender's certificate is on the revocation list.
    RevokedCertificate,
    /// The signature does not match the body.
    BadSignature,
    /// The temp_id in the body is not the certificate's pseudonym.
    TempIdMismatch,
    /// The auth bytes are not in the expected format.
    MalformedAuth,
}

/// Verifies the originator's signature over a message body.
pub trait Verifier {
    /// `Ok(())` only if `auth` carries a valid, unrevoked certificate whose pseudonym is the
    /// body's temp_id and a valid signature over the body (relay-mutable fields excluded),
    /// as sent within half an hour of `now` (Unix s).
    fn verify(&mut self, now: f64, body: &[u8; BODY_LEN], auth: &[u8]) -> Result<(), AuthError>;

    /// Every check of [`Verifier::verify`] except the message signature itself, for
    /// sampled heartbeats (SPEC §10; off by default, see `Security::verify_sample_above_peers`).
    fn verify_certificate_only(
        &mut self,
        now: f64,
        body: &[u8; BODY_LEN],
        auth: &[u8],
    ) -> Result<(), AuthError> {
        self.verify(now, body, auth)
    }
}

/// Produces this device's pseudonym and signatures.
pub trait Signer {
    /// The pseudonym in use at `now` (rotates with the certificate). `None` when the
    /// phone holds no valid certificate: it must not transmit (receive-only).
    fn current_temp_id(&mut self, now: f64) -> Option<TempId>;
    /// The `auth` bytes for `body` (whose temp_id must be the current pseudonym).
    /// `None` if signing is impossible right now.
    fn sign(&mut self, now: f64, is_event: bool, body: &[u8; BODY_LEN]) -> Option<Vec<u8>>;
}
