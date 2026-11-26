//! Event admission: the integrity gate between the bus and the pipeline.
//!
//! Order of checks for one envelope:
//!
//! 1. size limit and envelope split (`aeris-wire`);
//! 2. body decode + validation (`aeris-wire`), which yields the *claimed*
//!    source;
//! 3. key lookup for the claimed source's node (unknown / revoked → reject);
//! 4. Ed25519 `verify_strict` over the exact received body bytes;
//! 5. replay window: `seq > last ∧ freshness` (§6.2).
//!
//! Every rejection is returned with the claimed source so it can be turned
//! into integrity evidence ([`IntegrityLedger`]). Nothing is ever silently
//! dropped.

use crate::CryptoError;
use crate::keys::{Domain, KeyRegistry, SigningIdentity, verify};
use crate::replay::{Admission, ReplayConfig, ReplayRejection, ReplayWindow, SequenceCounter};
use aeris_types::{Evidence, EvidenceKind, VerifiedEvent};
use aeris_wire::{WireError, WireEvent};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Why an envelope was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RejectReason {
    /// Malformed, oversized or invalid message.
    #[error("wire: {0}")]
    Wire(#[from] WireError),
    /// The claimed source has no registered key.
    #[error("unknown source")]
    UnknownSource,
    /// The claimed source's key was revoked (isolated node).
    #[error("revoked source")]
    RevokedSource,
    /// Signature does not verify over the received bytes.
    #[error("bad signature")]
    BadSignature,
    /// Replay window refused it.
    #[error("replay: {0}")]
    Replay(#[from] ReplayRejection),
}

impl RejectReason {
    /// Stable short label (used in evidence kinds and metrics).
    pub fn label(&self) -> &'static str {
        match self {
            RejectReason::Wire(_) => "malformed",
            RejectReason::UnknownSource => "unknown_source",
            RejectReason::RevokedSource => "revoked_source",
            RejectReason::BadSignature => "bad_signature",
            RejectReason::Replay(ReplayRejection::Duplicate { .. }) => "duplicate",
            RejectReason::Replay(ReplayRejection::OutOfOrder { .. }) => "out_of_order",
            RejectReason::Replay(ReplayRejection::Stale { .. }) => "stale",
            RejectReason::Replay(ReplayRejection::FromFuture { .. }) => "from_future",
            RejectReason::Replay(ReplayRejection::ReplayTooOld { .. }) => "replay_too_old",
        }
    }
}

/// A refused envelope with whatever identity it claimed. The claims are
/// unauthenticated unless `reason` is a replay rejection.
#[derive(Debug, Clone, PartialEq)]
pub struct Rejected {
    /// Why it was refused.
    pub reason: RejectReason,
    /// Claimed source, if the body decoded.
    pub claimed_source: Option<String>,
    /// Claimed event id, if the body decoded.
    pub claimed_id: Option<String>,
    /// Claimed seq, if the body decoded.
    pub claimed_seq: Option<u64>,
    /// Receiver time.
    pub at_ms: i64,
}

/// An event that passed every check.
#[derive(Debug, Clone, PartialEq)]
pub struct Admitted {
    /// The validated event.
    pub event: WireEvent,
    /// How the window judged it.
    pub admission: Admission,
    /// Body bytes exactly as received (what the signature covers).
    pub body: Vec<u8>,
    /// The signature.
    pub signature: [u8; 64],
}

impl Admitted {
    /// Projection into the pipeline's input type (`signature_valid = true`
    /// because it was verified here).
    pub fn to_verified_event(&self) -> VerifiedEvent {
        self.event.to_verified_event(true)
    }
}

/// Signs events for one node: owns the node key and its sequence counter.
#[derive(Debug)]
pub struct EventSigner {
    identity: SigningIdentity,
    seq: SequenceCounter,
}

impl EventSigner {
    /// New signer starting at seq 1.
    pub fn new(identity: SigningIdentity) -> Self {
        Self {
            identity,
            seq: SequenceCounter::new(),
        }
    }

    /// Node id.
    pub fn node(&self) -> &str {
        self.identity.id()
    }

    /// Reserves the next sequence number.
    pub fn next_seq(&mut self) -> Result<u64, CryptoError> {
        self.seq
            .advance()
            .map_err(|_| CryptoError::SequenceExhausted)
    }

    /// Encodes and signs `ev`, returning the envelope bytes. `ev.source` must
    /// belong to this node; `ev.seq` must have come from [`Self::next_seq`].
    pub fn seal(&self, ev: &WireEvent) -> Result<Vec<u8>, CryptoError> {
        if ev.node() != self.identity.id() {
            return Err(CryptoError::KeyFormat(format!(
                "signer {} cannot sign for source {}",
                self.identity.id(),
                ev.source
            )));
        }
        let body = aeris_wire::encode_body(ev)?;
        let sig = self.identity.sign(Domain::Event, &body);
        Ok(aeris_wire::encode_envelope(&body, &sig))
    }
}

/// Server-side admission: key registry + replay window.
#[derive(Debug, Clone)]
pub struct EventVerifier {
    keys: KeyRegistry,
    window: ReplayWindow,
}

impl EventVerifier {
    /// New verifier.
    pub fn new(keys: KeyRegistry, cfg: ReplayConfig) -> Self {
        Self {
            keys,
            window: ReplayWindow::new(cfg),
        }
    }

    /// Mutable access to the registry (revoke on isolation).
    pub fn keys_mut(&mut self) -> &mut KeyRegistry {
        &mut self.keys
    }

    /// The registry.
    pub fn keys(&self) -> &KeyRegistry {
        &self.keys
    }

    /// Last accepted seq for `source`.
    pub fn last_seq(&self, source: &str) -> u64 {
        self.window.last(source)
    }

    /// Runs every check on one received envelope.
    pub fn admit(&mut self, envelope: &[u8], now_ms: i64) -> Result<Admitted, Rejected> {
        let reject = |reason: RejectReason, ev: Option<&WireEvent>| Rejected {
            reason,
            claimed_source: ev.map(|e| e.source.clone()),
            claimed_id: ev.map(|e| e.id.clone()),
            claimed_seq: ev.map(|e| e.seq),
            at_ms: now_ms,
        };
        let raw = aeris_wire::decode_envelope(envelope).map_err(|e| reject(e.into(), None))?;
        let ev = aeris_wire::decode_body(&raw.body).map_err(|e| reject(e.into(), None))?;
        let key = match self.keys.get(ev.node()) {
            Ok(k) => k,
            Err(CryptoError::Revoked(_)) => {
                return Err(reject(RejectReason::RevokedSource, Some(&ev)));
            }
            Err(_) => return Err(reject(RejectReason::UnknownSource, Some(&ev))),
        };
        if verify(key, Domain::Event, &raw.body, &raw.signature).is_err() {
            return Err(reject(RejectReason::BadSignature, Some(&ev)));
        }
        let admission = self
            .window
            .admit(&ev.source, ev.seq, ev.t_unix_ms, ev.replayed, now_ms)
            .map_err(|r| reject(r.into(), Some(&ev)))?;
        Ok(Admitted {
            event: ev,
            admission,
            body: raw.body,
            signature: raw.signature,
        })
    }
}

/// Aggregated integrity evidence against one node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityCount {
    /// Node the rejected events claimed to come from.
    pub node: String,
    /// [`RejectReason::label`].
    pub reason: String,
    /// Number of rejections since the last drain.
    pub count: u64,
    /// First and last rejection time in the batch.
    pub first_ms: i64,
    /// Last rejection time.
    pub last_ms: i64,
    /// Up to 8 sample claimed event ids.
    pub sample_ids: Vec<String>,
}

/// Collects rejections and turns them into at most one evidence item per
/// `(node, reason)` per drain, so a flood of forged events produces one
/// counted item instead of unbounded evidence (DoS guard). Only rejections
/// that name a *registered* node are kept; anything else cannot be
/// attributed and is only counted in `unattributed`.
#[derive(Debug, Clone, Default)]
pub struct IntegrityLedger {
    counts: BTreeMap<(String, &'static str), IntegrityCount>,
    /// Rejections with no attributable registered node.
    pub unattributed: u64,
}

impl IntegrityLedger {
    /// Records one rejection.
    pub fn record(&mut self, r: &Rejected, keys: &KeyRegistry) {
        let node = r
            .claimed_source
            .as_deref()
            .map(aeris_wire::source_node)
            .filter(|n| keys.contains(n));
        let Some(node) = node else {
            self.unattributed = self.unattributed.saturating_add(1);
            return;
        };
        let label = r.reason.label();
        let e = self
            .counts
            .entry((node.to_string(), label))
            .or_insert_with(|| IntegrityCount {
                node: node.to_string(),
                reason: label.to_string(),
                count: 0,
                first_ms: r.at_ms,
                last_ms: r.at_ms,
                sample_ids: Vec::new(),
            });
        e.count = e.count.saturating_add(1);
        e.last_ms = r.at_ms;
        if e.sample_ids.len() < 8
            && let Some(id) = &r.claimed_id
        {
            e.sample_ids.push(id.clone());
        }
    }

    /// Takes the aggregated counts.
    pub fn drain(&mut self) -> Vec<IntegrityCount> {
        std::mem::take(&mut self.counts).into_values().collect()
    }

    /// Takes the aggregated counts as pipeline evidence. Kind is
    /// `Other("integrity:<reason>")`, which `aeris-trust` maps to the
    /// integrity dimension. `score` is 1.0 = "this happened"; how much it
    /// should move trust is a fusion/trust policy decision (owners A/K).
    pub fn drain_evidence(&mut self) -> Vec<Evidence> {
        self.drain()
            .into_iter()
            .map(|c| {
                let digest = blake3::hash(
                    format!("{}|{}|{}|{}", c.node, c.reason, c.first_ms, c.count).as_bytes(),
                );
                Evidence {
                    id: format!("ev-int-{}", &digest.to_hex()[..12]),
                    entity: c.node.clone(),
                    kind: EvidenceKind::Other(format!("integrity:{}", c.reason)),
                    score: 1.0,
                    event: c.sample_ids.first().cloned().unwrap_or_default(),
                    group: Some(format!("integrity-{}", c.node)),
                    timestamp_ms: c.last_ms,
                }
            })
            .collect()
    }
}
