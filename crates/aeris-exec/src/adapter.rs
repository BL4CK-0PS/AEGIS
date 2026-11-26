//! The execution seam: [`Adapter`] and the in-process simulator adapter.
//!
//! v0.1 has exactly one real target, the simulator. [`LocalSimAdapter`]
//! drives an in-process [`World`]; `NatsSimAdapter` (feature `nats`) sends
//! the same commands over NATS to `aeris-simd`. Linux/network adapters are
//! v0.2 and deliberately absent.

use aeris_sim::{ApplyOutcome, SimError, World};
use aeris_wire::{AckStatus, CommandAck, CommandOp, WorldObservation};
use std::future::Future;
use std::sync::{Arc, Mutex};

/// Transport/adapter failure. A failure is never reported as success.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdapterError {
    /// No acknowledgement within the deadline. The command may or may not
    /// have been applied; verification decides.
    #[error("timed out after {0} ms")]
    Timeout(u64),
    /// The transport failed (connection refused, no responders, ...).
    #[error("transport: {0}")]
    Transport(String),
    /// An acknowledgement arrived but failed authentication or validation.
    #[error("bad acknowledgement: {0}")]
    BadAck(String),
}

/// Something that can carry commands to the world and report its observable
/// state. Implementations must be honest: an `Ok` ack means the target
/// answered, and its status says what happened.
pub trait Adapter: Send {
    /// Sends one command on behalf of `decision_id`.
    fn submit(
        &mut self,
        decision_id: &str,
        op: &CommandOp,
    ) -> impl Future<Output = Result<CommandAck, AdapterError>> + Send;

    /// Current observable state (topology and link measurements).
    fn observe(&mut self) -> impl Future<Output = Result<WorldObservation, AdapterError>> + Send;
}

/// Shared handle to an in-process world. The caller keeps stepping the world;
/// the adapter only applies commands and reads observations, holding the lock
/// briefly and never across an `.await`.
pub type SharedWorld = Arc<Mutex<World>>;

/// In-process adapter over a [`SharedWorld`].
#[derive(Debug, Clone)]
pub struct LocalSimAdapter {
    world: SharedWorld,
    next_id: u64,
}

impl LocalSimAdapter {
    /// New adapter.
    pub fn new(world: SharedWorld) -> Self {
        Self { world, next_id: 0 }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, World>, AdapterError> {
        self.world
            .lock()
            .map_err(|_| AdapterError::Transport("world mutex poisoned".into()))
    }
}

/// Maps a simulator result to an acknowledgement (shared with `aeris-simd`).
pub fn ack_for(
    command_id: &str,
    result: Result<ApplyOutcome, SimError>,
    observation: WorldObservation,
) -> CommandAck {
    let (status, reason) = match result {
        Ok(ApplyOutcome::Applied) => (AckStatus::Applied, String::new()),
        Ok(ApplyOutcome::AlreadyApplied) => (AckStatus::AlreadyApplied, String::new()),
        Err(e) => (AckStatus::Failed, e.to_string()),
    };
    CommandAck {
        command_id: command_id.to_string(),
        status,
        reason,
        t_unix_ms: observation.t_unix_ms,
        observation: Some(observation),
    }
}

impl Adapter for LocalSimAdapter {
    fn submit(
        &mut self,
        decision_id: &str,
        op: &CommandOp,
    ) -> impl Future<Output = Result<CommandAck, AdapterError>> + Send {
        self.next_id += 1;
        let id = format!("local-{decision_id}-{}", self.next_id);
        let r = self.lock().map(|mut w| {
            let res = w.apply_command(op);
            ack_for(&id, res, w.observe())
        });
        std::future::ready(r)
    }

    fn observe(&mut self) -> impl Future<Output = Result<WorldObservation, AdapterError>> + Send {
        std::future::ready(self.lock().map(|w| w.observe()))
    }
}
