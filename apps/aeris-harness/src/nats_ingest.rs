//! JetStream ingest for tests and evaluation: consume `aeris.events.>`,
//! verify every envelope with `aeris-crypto`, hand admitted events on.
//!
//! This is a test/evaluation consumer. The production ingest (persistence,
//! `(source, seq)` uniqueness in Postgres, WebSocket fan-out) belongs to
//! `aeris-server` (owner: Kesivarshini) and should reuse
//! `aeris_crypto::ingest::EventVerifier` exactly as done here.

use aeris_crypto::ingest::{Admitted, EventVerifier, Rejected};
use async_nats::jetstream::consumer::{AckPolicy, DeliverPolicy, pull};
use futures::StreamExt;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

/// What the ingest task emits.
#[derive(Debug)]
pub enum Ingested {
    /// Passed every check.
    Admitted(Box<Admitted>),
    /// Refused, with reason and claimed identity.
    Rejected(Box<Rejected>),
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Starts consuming new events. Verification happens on receipt (so a slow
/// downstream cannot make fresh events look stale); results are sent on the
/// returned channel. The JetStream message is acked only after the result
/// was handed on.
pub async fn spawn(
    client: async_nats::Client,
    verifier: EventVerifier,
) -> Result<mpsc::Receiver<Ingested>, String> {
    spawn_shared(client, std::sync::Arc::new(std::sync::Mutex::new(verifier))).await
}

/// As [`spawn`], with the verifier shared so the caller can revoke a key
/// (e.g. after a verified isolation) while ingest runs.
pub async fn spawn_shared(
    client: async_nats::Client,
    verifier: std::sync::Arc<std::sync::Mutex<EventVerifier>>,
) -> Result<mpsc::Receiver<Ingested>, String> {
    let js = async_nats::jetstream::new(client);
    let stream = js
        .get_stream(aeris_simd_stream_name())
        .await
        .map_err(|e| format!("get stream: {e}"))?;
    let consumer = stream
        .create_consumer(pull::Config {
            deliver_policy: DeliverPolicy::New,
            ack_policy: AckPolicy::Explicit,
            filter_subject: format!("{}.>", aeris_wire::EVENTS_SUBJECT_PREFIX),
            ..Default::default()
        })
        .await
        .map_err(|e| format!("create consumer: {e}"))?;
    let mut messages = consumer
        .messages()
        .await
        .map_err(|e| format!("messages: {e}"))?;
    let (tx, rx) = mpsc::channel(65_536);
    tokio::spawn(async move {
        while let Some(m) = messages.next().await {
            let m = match m {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "consumer error");
                    continue;
                }
            };
            let res = match verifier.lock() {
                Ok(mut v) => v.admit(&m.payload, now_ms()),
                Err(_) => break, // poisoned: stop admitting anything
            };
            let out = match res {
                Ok(a) => Ingested::Admitted(Box::new(a)),
                Err(r) => Ingested::Rejected(Box::new(r)),
            };
            if tx.send(out).await.is_err() {
                break;
            }
            if let Err(e) = m.ack().await {
                tracing::warn!(error = %e, "ack failed");
            }
        }
    });
    Ok(rx)
}

fn aeris_simd_stream_name() -> &'static str {
    // Kept in sync with aeris_simd::STREAM_NAME (asserted in tests).
    "AERIS_EVENTS"
}
