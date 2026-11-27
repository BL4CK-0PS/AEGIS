//! `SimAdapter` over NATS: signed commands to `aeris.cmd.<entity>`, signed
//! acknowledgements back (request/reply). Talks to `aeris-simd`.

use crate::adapter::{Adapter, AdapterError};
use aeris_crypto::VerifyingKey;
use aeris_crypto::command_auth::{CommandSigner, open_ack};
use aeris_crypto::keys::SigningIdentity;
use aeris_wire::{CommandAck, CommandOp, WorldObservation};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Simulator adapter over NATS request/reply.
pub struct NatsSimAdapter {
    client: async_nats::Client,
    signer: CommandSigner,
    sim_key: VerifyingKey,
    timeout: Duration,
}

impl std::fmt::Debug for NatsSimAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NatsSimAdapter")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl NatsSimAdapter {
    /// `server` signs commands; acks must verify under `sim_key`.
    pub fn new(
        client: async_nats::Client,
        server: SigningIdentity,
        sim_key: VerifyingKey,
        timeout: Duration,
    ) -> Self {
        Self {
            client,
            signer: CommandSigner::new(server),
            sim_key,
            timeout,
        }
    }

    async fn request(
        &mut self,
        decision_id: &str,
        op: CommandOp,
    ) -> Result<CommandAck, AdapterError> {
        let id = uuid::Uuid::now_v7().hyphenated().to_string();
        let (cmd, env) = self
            .signer
            .issue(id, decision_id, op, now_ms())
            .map_err(|e| AdapterError::Transport(format!("sign: {e}")))?;
        let fut = self.client.request(cmd.subject(), env.into());
        let msg = match tokio::time::timeout(self.timeout, fut).await {
            Err(_) => return Err(AdapterError::Timeout(self.timeout.as_millis() as u64)),
            Ok(Err(e)) => {
                return Err(match e.kind() {
                    async_nats::RequestErrorKind::TimedOut => {
                        AdapterError::Timeout(self.timeout.as_millis() as u64)
                    }
                    _ => AdapterError::Transport(e.to_string()),
                });
            }
            Ok(Ok(m)) => m,
        };
        open_ack(&self.sim_key, &msg.payload, &cmd.id)
            .map_err(|e| AdapterError::BadAck(e.to_string()))
    }
}

impl Adapter for NatsSimAdapter {
    async fn submit(
        &mut self,
        decision_id: &str,
        op: &CommandOp,
    ) -> Result<CommandAck, AdapterError> {
        self.request(decision_id, op.clone()).await
    }

    async fn observe(&mut self) -> Result<WorldObservation, AdapterError> {
        let ack = self.request("", CommandOp::Observe).await?;
        ack.observation
            .ok_or_else(|| AdapterError::BadAck("observe ack without observation".into()))
    }
}
