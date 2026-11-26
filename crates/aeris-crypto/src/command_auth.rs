//! Authentication of the command channel (server → simulator) and its
//! acknowledgements (simulator → server).

use crate::CryptoError;
use crate::keys::{Domain, SigningIdentity, verify};
use crate::replay::{ReplayConfig, ReplayRejection, ReplayWindow, SequenceCounter};
use aeris_wire::{Command, CommandAck, CommandOp, WireError};
use ed25519_dalek::VerifyingKey;

/// Issues signed commands with a monotonic per-issuer sequence.
#[derive(Debug)]
pub struct CommandSigner {
    identity: SigningIdentity,
    seq: SequenceCounter,
}

impl CommandSigner {
    /// New signer; the identity id is the issuer name (normally `server`).
    pub fn new(identity: SigningIdentity) -> Self {
        Self {
            identity,
            seq: SequenceCounter::new(),
        }
    }

    /// Builds, encodes and signs a command. Returns the command (with its
    /// assigned seq) and the envelope bytes.
    pub fn issue(
        &mut self,
        id: String,
        decision_id: &str,
        op: CommandOp,
        t_unix_ms: i64,
    ) -> Result<(Command, Vec<u8>), CryptoError> {
        let seq = self
            .seq
            .advance()
            .map_err(|_| CryptoError::SequenceExhausted)?;
        let cmd = Command {
            id,
            decision_id: decision_id.to_string(),
            issuer: self.identity.id().to_string(),
            seq,
            t_unix_ms,
            op,
        };
        let body = aeris_wire::encode_command(&cmd)?;
        let sig = self.identity.sign(Domain::Command, &body);
        Ok((cmd, aeris_wire::encode_command_envelope(&body, &sig)))
    }
}

/// Why a command was refused by the simulator.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CommandRejection {
    /// Malformed or invalid.
    #[error("wire: {0}")]
    Wire(#[from] WireError),
    /// Issuer is not the trusted server identity.
    #[error("untrusted issuer {0}")]
    UntrustedIssuer(String),
    /// Signature does not verify.
    #[error("bad signature")]
    BadSignature,
    /// The NATS subject does not match the command's target.
    #[error("subject {subject} does not match target {expected}")]
    SubjectMismatch {
        /// Subject the command arrived on.
        subject: String,
        /// Subject it should have arrived on.
        expected: String,
    },
    /// Replay window refused it.
    #[error("replay: {0}")]
    Replay(#[from] ReplayRejection),
}

/// Verifies commands against the server's public key, subject binding and a
/// replay window on `(issuer, seq)`.
#[derive(Debug, Clone)]
pub struct CommandVerifier {
    issuer: String,
    key: VerifyingKey,
    window: ReplayWindow,
}

impl CommandVerifier {
    /// Trusts commands from `issuer` signed by `key`.
    pub fn new(issuer: impl Into<String>, key: VerifyingKey, cfg: ReplayConfig) -> Self {
        Self {
            issuer: issuer.into(),
            key,
            window: ReplayWindow::new(cfg),
        }
    }

    /// Authenticates a command envelope received on `subject` and consumes
    /// its sequence number. Equivalent to [`Self::authenticate`] followed by
    /// [`Self::admit_seq`].
    pub fn admit(
        &mut self,
        subject: &str,
        envelope: &[u8],
        now_ms: i64,
    ) -> Result<Command, CommandRejection> {
        let cmd = self.authenticate(subject, envelope)?;
        self.admit_seq(&cmd, now_ms)?;
        Ok(cmd)
    }

    /// Checks issuer, signature and subject binding without touching the
    /// replay window. Lets a receiver answer a re-delivered command from its
    /// idempotency cache (same id ⇒ same ack) instead of rejecting it.
    pub fn authenticate(
        &self,
        subject: &str,
        envelope: &[u8],
    ) -> Result<Command, CommandRejection> {
        let raw = aeris_wire::decode_command_envelope(envelope)?;
        let cmd = aeris_wire::decode_command(&raw.body)?;
        if cmd.issuer != self.issuer {
            return Err(CommandRejection::UntrustedIssuer(cmd.issuer));
        }
        // Signature before anything that changes state, so a forged command
        // cannot advance the window.
        verify(&self.key, Domain::Command, &raw.body, &raw.signature)
            .map_err(|_| CommandRejection::BadSignature)?;
        let expected = cmd.subject();
        if subject != expected {
            return Err(CommandRejection::SubjectMismatch {
                subject: subject.to_string(),
                expected,
            });
        }
        Ok(cmd)
    }

    /// Replay window on `(issuer, seq)` plus freshness, for an authenticated
    /// command.
    pub fn admit_seq(&mut self, cmd: &Command, now_ms: i64) -> Result<(), CommandRejection> {
        self.window
            .admit(&cmd.issuer, cmd.seq, cmd.t_unix_ms, false, now_ms)?;
        Ok(())
    }
}

/// Signs an acknowledgement with the simulator controller key.
pub fn seal_ack(identity: &SigningIdentity, ack: &CommandAck) -> Result<Vec<u8>, CryptoError> {
    let body = aeris_wire::encode_ack(ack)?;
    let sig = identity.sign(Domain::Ack, &body);
    Ok(aeris_wire::encode_ack_envelope(&body, &sig))
}

/// Verifies an acknowledgement envelope and checks it answers `command_id`.
pub fn open_ack(
    key: &VerifyingKey,
    envelope: &[u8],
    command_id: &str,
) -> Result<CommandAck, CryptoError> {
    let raw = aeris_wire::decode_ack_envelope(envelope)?;
    verify(key, Domain::Ack, &raw.body, &raw.signature)?;
    let ack = aeris_wire::decode_ack(&raw.body)?;
    if ack.command_id != command_id {
        return Err(CryptoError::AckMismatch {
            expected: command_id.to_string(),
            got: ack.command_id,
        });
    }
    Ok(ack)
}
