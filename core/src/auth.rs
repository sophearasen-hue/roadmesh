//! Message authentication interfaces (SPEC §10).
//!
//! The core never holds keys. The platform implements [`Signer`] with keys in the
//! Secure Enclave / Android Keystore (AUDIT S-02) and [`Verifier`] with pseudonym
//! certificates and the revocation list; both arrive in Step 3. The simulator's
//! demo implementation lives in `sim/` only, never in this crate (CLAUDE.md:
//! no shared, hard-coded or test keys in production code).

use crate::rsm::{BODY_LEN, TempId};

/// Why a message failed authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    /// No certificate known for the sender.
    UnknownCertificate,
    /// The sender's certificate is on the revocation list.
    RevokedCertificate,
    /// The signature does not match the body.
    BadSignature,
    /// The temp_id in the body is not the certificate's pseudonym.
    TempIdMismatch,
    /// The auth bytes are not in the expected format.
    MalformedAuth,
}

/// Verifies the originator's signature over [`crate::rsm::signing_bytes`].
pub trait Verifier {
    /// `Ok(())` only if `auth` is a valid, unrevoked signature by `temp_id` over `signed`.
    fn verify(
        &self,
        temp_id: &TempId,
        signed: &[u8; BODY_LEN],
        auth: &[u8],
    ) -> Result<(), AuthError>;
}

/// Produces this device's pseudonym and signatures.
pub trait Signer {
    /// The pseudonym in use at `now` (rotates with the certificate).
    fn current_temp_id(&mut self, now: f64) -> TempId;
    /// Sign `signed` (already passed through `signing_bytes`) with the key of `temp_id`.
    fn sign(&mut self, temp_id: &TempId, signed: &[u8; BODY_LEN]) -> Vec<u8>;
}
