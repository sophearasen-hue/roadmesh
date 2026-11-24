//! RoadMesh edge server library.
//!
//! Step 3: the certificate authority logic ([`ca`]): pseudonym certificate
//! issuance, misbehaviour reports, revocation and the revocation list.
//! Step 9: the edge broker ([`broker`], [`serve`], [`tls`]): MQTT over TLS 1.3 with
//! mutual TLS, the hazard gate ([`hazard`]), device certificates ([`devca`]),
//! App Attest / Play Integrity ([`attest`]), CA keys in AWS KMS (`kms`, feature
//! `aws-kms`), rate limits ([`limits`]) and monitoring ([`monitor`]) (AUDIT S-16 to S-21).

pub mod attest;
pub mod broker;
pub mod ca;
pub mod devca;
pub mod hazard;
#[cfg(feature = "aws-kms")]
pub mod kms;
pub mod limits;
pub mod monitor;
pub mod serve;
pub mod tls;
