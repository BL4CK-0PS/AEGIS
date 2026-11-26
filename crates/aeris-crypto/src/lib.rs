//! AERIS cryptography: Ed25519 event and command signing, per-source
//! sequencing and the replay window, and the BLAKE3 decision audit chain.
//!
//! Owner: DHARSHAN. Specification: `docs/v0.1/04-mathematical-model.md` §6.2.
//!
//! | Module | Implements |
//! |---|---|
//! | [`keys`] | identities, key files, public-key registry, domain-separated sign/verify |
//! | [`replay`] | `SequenceCounter`, `ReplayWindow` (`seq > last ∧ |t−now| ≤ 2 s`, S7 exception) |
//! | [`ingest`] | `EventSigner`, `EventVerifier` (decode → key → verify exact bytes → window), integrity evidence |
//! | [`command_auth`] | signed commands with subject binding, signed acknowledgements |
//! | [`audit`] | canonical record bytes, `h_n = BLAKE3(h_{n-1} ‖ bytes(D_n))`, signatures, chain verification |
//!
//! Security assumptions and the key lifecycle are documented in
//! `docs/security/dharshan-security-audit.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod audit;
pub mod command_auth;
pub mod ingest;
pub mod keys;
pub mod replay;

pub use ed25519_dalek::VerifyingKey;

/// Crypto-layer error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    /// Signature did not verify.
    #[error("signature verification failed")]
    BadSignature,
    /// No key registered for this id.
    #[error("unknown key id {0}")]
    UnknownKey(String),
    /// Key exists but was revoked.
    #[error("key {0} is revoked")]
    Revoked(String),
    /// Key material malformed, weak, or stored insecurely.
    #[error("key format: {0}")]
    KeyFormat(String),
    /// Filesystem error while reading/writing key material.
    #[error("io: {0}")]
    Io(String),
    /// OS random number generator failed.
    #[error("rng: {0}")]
    Rng(String),
    /// A sequence counter is exhausted.
    #[error("sequence space exhausted; re-key")]
    SequenceExhausted,
    /// Wire-layer validation failed.
    #[error("wire: {0}")]
    Wire(#[from] aeris_wire::WireError),
    /// Record serialization failed.
    #[error("serialize: {0}")]
    Serialize(String),
    /// An acknowledgement answers a different command.
    #[error("ack for {got}, expected {expected}")]
    AckMismatch {
        /// Command id we sent.
        expected: String,
        /// Command id in the ack.
        got: String,
    },
}

#[cfg(test)]
mod tests;
