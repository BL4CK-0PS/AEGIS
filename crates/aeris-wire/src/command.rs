//! Command channel (server → simulator) and acknowledgements (`command.proto`).

use crate::{
    WireError, decode_err, in_range, invalid, pb, split_envelope, validate_entity_id,
    validate_text, validate_uuid_v7,
};
use aeris_types::Capability;
use prost::Message;
use serde::{Deserialize, Serialize};

fn cap_to_pb(c: Capability) -> pb::Capability {
    match c {
        Capability::Navigation => pb::Capability::Navigation,
        Capability::Communication => pb::Capability::Communication,
        Capability::Observation => pb::Capability::Observation,
    }
}

fn cap_from_pb(field: &'static str, raw: i32) -> Result<Capability, WireError> {
    match pb::Capability::try_from(raw) {
        Ok(pb::Capability::Navigation) => Ok(Capability::Navigation),
        Ok(pb::Capability::Communication) => Ok(Capability::Communication),
        Ok(pb::Capability::Observation) => Ok(Capability::Observation),
        Ok(pb::Capability::Unspecified) => Err(WireError::Missing(field)),
        Err(_) => Err(WireError::UnknownEnum { field, value: raw }),
    }
}

/// A concrete command `u ∈ Γ(a)` (§6.13).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOp {
    /// Remove the node from routing and stop accepting its traffic.
    Isolate {
        /// Target node.
        node: String,
    },
    /// Revoke the node's privileges and external egress; it keeps relaying.
    Restrict {
        /// Target node.
        node: String,
    },
    /// Return the node to ACTIVE.
    Recover {
        /// Target node.
        node: String,
    },
    /// Bring a standby link into routing.
    ActivateLink {
        /// Target link.
        link: String,
    },
    /// Move `from`'s relay role off it (see `aeris-sim` for exact semantics).
    Failover {
        /// Node giving up its role.
        from: String,
        /// Node that must remain active.
        to: String,
    },
    /// Run the mission on the degraded minimum for one capability.
    Degrade {
        /// Capability to degrade.
        capability: Capability,
    },
    /// Read-only request for the observable world state.
    Observe,
}

impl CommandOp {
    /// Entity used as the last subject token (`aeris.cmd.<entity>`).
    pub fn target_entity(&self) -> String {
        match self {
            CommandOp::Isolate { node }
            | CommandOp::Restrict { node }
            | CommandOp::Recover { node } => node.clone(),
            CommandOp::ActivateLink { link } => link.clone(),
            CommandOp::Failover { from, .. } => from.clone(),
            CommandOp::Degrade { .. } => "mission".to_string(),
            CommandOp::Observe => "world".to_string(),
        }
    }

    /// `true` for commands that change world state.
    pub fn is_mutating(&self) -> bool {
        !matches!(self, CommandOp::Observe)
    }

    fn validate(&self) -> Result<(), WireError> {
        match self {
            CommandOp::Isolate { node }
            | CommandOp::Restrict { node }
            | CommandOp::Recover { node } => validate_entity_id("op.node", node),
            CommandOp::ActivateLink { link } => validate_entity_id("op.link", link),
            CommandOp::Failover { from, to } => {
                validate_entity_id("op.from", from)?;
                validate_entity_id("op.to", to)?;
                if from == to {
                    return Err(invalid("op.to", "failover target equals source"));
                }
                Ok(())
            }
            CommandOp::Degrade { .. } | CommandOp::Observe => Ok(()),
        }
    }

    fn to_pb(&self) -> pb::command_body::Op {
        use pb::command_body::Op;
        match self.clone() {
            CommandOp::Isolate { node } => Op::Isolate(pb::NodeTarget { node }),
            CommandOp::Restrict { node } => Op::Restrict(pb::NodeTarget { node }),
            CommandOp::Recover { node } => Op::Recover(pb::NodeTarget { node }),
            CommandOp::ActivateLink { link } => Op::ActivateLink(pb::LinkTarget { link }),
            CommandOp::Failover { from, to } => Op::Failover(pb::Failover { from, to }),
            CommandOp::Degrade { capability } => Op::Degrade(pb::CapabilityTarget {
                capability: cap_to_pb(capability) as i32,
            }),
            CommandOp::Observe => Op::Observe(pb::Observe {}),
        }
    }

    fn from_pb(op: pb::command_body::Op) -> Result<Self, WireError> {
        use pb::command_body::Op;
        Ok(match op {
            Op::Isolate(t) => CommandOp::Isolate { node: t.node },
            Op::Restrict(t) => CommandOp::Restrict { node: t.node },
            Op::Recover(t) => CommandOp::Recover { node: t.node },
            Op::ActivateLink(t) => CommandOp::ActivateLink { link: t.link },
            Op::Failover(f) => CommandOp::Failover {
                from: f.from,
                to: f.to,
            },
            Op::Degrade(d) => CommandOp::Degrade {
                capability: cap_from_pb("op.degrade.capability", d.capability)?,
            },
            Op::Observe(_) => CommandOp::Observe,
        })
    }
}

/// A validated command body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    /// UUIDv7; idempotency key.
    pub id: String,
    /// Decision that authorised the command (empty only for `Observe`).
    pub decision_id: String,
    /// Issuer identity (`server`).
    pub issuer: String,
    /// Per-issuer sequence number, `>= 1`.
    pub seq: u64,
    /// Issue time, ms since the Unix epoch.
    pub t_unix_ms: i64,
    /// The operation.
    pub op: CommandOp,
}

impl Command {
    /// Validates every field.
    pub fn validate(&self) -> Result<(), WireError> {
        validate_uuid_v7("id", &self.id)?;
        if self.op.is_mutating() {
            validate_text("decision_id", &self.decision_id, false)?;
        } else {
            validate_text("decision_id", &self.decision_id, true)?;
        }
        validate_entity_id("issuer", &self.issuer)?;
        if self.seq == 0 {
            return Err(invalid("seq", "must be >= 1"));
        }
        if self.t_unix_ms <= 0 {
            return Err(invalid("t_unix_ms", "must be > 0"));
        }
        self.op.validate()
    }

    /// The NATS subject this command must be sent on.
    pub fn subject(&self) -> String {
        crate::cmd_subject(&self.op.target_entity())
    }
}

/// Validates and encodes a command body (the bytes that get signed).
pub fn encode_command(cmd: &Command) -> Result<Vec<u8>, WireError> {
    cmd.validate()?;
    Ok(pb::CommandBody {
        id: cmd.id.clone(),
        decision_id: cmd.decision_id.clone(),
        issuer: cmd.issuer.clone(),
        seq: cmd.seq,
        t_unix_ms: cmd.t_unix_ms,
        op: Some(cmd.op.to_pb()),
    }
    .encode_to_vec())
}

/// Decodes and validates a command body.
pub fn decode_command(bytes: &[u8]) -> Result<Command, WireError> {
    if bytes.len() > crate::MAX_ENVELOPE_BYTES {
        return Err(WireError::TooLarge {
            size: bytes.len(),
            limit: crate::MAX_ENVELOPE_BYTES,
        });
    }
    let b = pb::CommandBody::decode(bytes).map_err(decode_err)?;
    let cmd = Command {
        id: b.id,
        decision_id: b.decision_id,
        issuer: b.issuer,
        seq: b.seq,
        t_unix_ms: b.t_unix_ms,
        op: CommandOp::from_pb(b.op.ok_or(WireError::Missing("op"))?)?,
    };
    cmd.validate()?;
    Ok(cmd)
}

/// Encodes a command envelope around already-encoded body bytes.
pub fn encode_command_envelope(body: &[u8], signature: &[u8; crate::SIGNATURE_LEN]) -> Vec<u8> {
    pb::CommandEnvelope {
        body: body.to_vec(),
        signature: signature.to_vec(),
    }
    .encode_to_vec()
}

/// Splits a received command envelope.
pub fn decode_command_envelope(bytes: &[u8]) -> Result<crate::RawEnvelope, WireError> {
    split_envelope(bytes, |b| {
        pb::CommandEnvelope::decode(b).map(|e| (e.body, e.signature))
    })
}

/// Encodes an ack envelope around already-encoded body bytes.
pub fn encode_ack_envelope(body: &[u8], signature: &[u8; crate::SIGNATURE_LEN]) -> Vec<u8> {
    pb::AckEnvelope {
        body: body.to_vec(),
        signature: signature.to_vec(),
    }
    .encode_to_vec()
}

/// Splits a received ack envelope.
pub fn decode_ack_envelope(bytes: &[u8]) -> Result<crate::RawEnvelope, WireError> {
    split_envelope(bytes, |b| {
        pb::AckEnvelope::decode(b).map(|e| (e.body, e.signature))
    })
}

// ============================================================================
// Acknowledgements and observations
// ============================================================================

/// Outcome of a command at the simulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AckStatus {
    /// State changed as commanded.
    Applied,
    /// Target was already in the commanded state; nothing changed.
    AlreadyApplied,
    /// Authentication or validation failed; nothing applied.
    Rejected,
    /// Authentic, but the world refused it.
    Failed,
}

impl AckStatus {
    /// `true` when the commanded state now holds.
    pub fn is_success(self) -> bool {
        matches!(self, AckStatus::Applied | AckStatus::AlreadyApplied)
    }

    fn to_pb(self) -> pb::AckStatus {
        match self {
            AckStatus::Applied => pb::AckStatus::Applied,
            AckStatus::AlreadyApplied => pb::AckStatus::AlreadyApplied,
            AckStatus::Rejected => pb::AckStatus::Rejected,
            AckStatus::Failed => pb::AckStatus::Failed,
        }
    }

    fn from_pb(raw: i32) -> Result<Self, WireError> {
        match pb::AckStatus::try_from(raw) {
            Ok(pb::AckStatus::Applied) => Ok(AckStatus::Applied),
            Ok(pb::AckStatus::AlreadyApplied) => Ok(AckStatus::AlreadyApplied),
            Ok(pb::AckStatus::Rejected) => Ok(AckStatus::Rejected),
            Ok(pb::AckStatus::Failed) => Ok(AckStatus::Failed),
            Ok(pb::AckStatus::Unspecified) => Err(WireError::Missing("status")),
            Err(_) => Err(WireError::UnknownEnum {
                field: "status",
                value: raw,
            }),
        }
    }
}

/// Operational mode of a node, as observable by the control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeMode {
    /// Normal operation.
    Active,
    /// Privileges/egress revoked, still relaying.
    Restricted,
    /// Removed from routing; traffic not accepted.
    Isolated,
    /// Role handed over by failover; not relaying.
    Standby,
}

impl NodeMode {
    fn to_pb(self) -> pb::NodeMode {
        match self {
            NodeMode::Active => pb::NodeMode::Active,
            NodeMode::Restricted => pb::NodeMode::Restricted,
            NodeMode::Isolated => pb::NodeMode::Isolated,
            NodeMode::Standby => pb::NodeMode::Standby,
        }
    }

    fn from_pb(raw: i32) -> Result<Self, WireError> {
        match pb::NodeMode::try_from(raw) {
            Ok(pb::NodeMode::Active) => Ok(NodeMode::Active),
            Ok(pb::NodeMode::Restricted) => Ok(NodeMode::Restricted),
            Ok(pb::NodeMode::Isolated) => Ok(NodeMode::Isolated),
            Ok(pb::NodeMode::Standby) => Ok(NodeMode::Standby),
            Ok(pb::NodeMode::Unspecified) => Err(WireError::Missing("node.mode")),
            Err(_) => Err(WireError::UnknownEnum {
                field: "node.mode",
                value: raw,
            }),
        }
    }
}

/// Observed link state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkObservation {
    /// Link id.
    pub link: String,
    /// Part of routing.
    pub active: bool,
    /// Physically connected.
    pub up: bool,
    /// Measured end-to-end availability.
    pub availability: f64,
    /// Latency, ms.
    pub latency_ms: f64,
    /// Loss fraction.
    pub loss: f64,
}

/// Observable world state: topology and link measurements only. It never
/// carries attacker ground truth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WorldObservation {
    /// Simulator tick.
    pub tick: u64,
    /// Observation time.
    pub t_unix_ms: i64,
    /// Node modes, sorted by node id.
    pub nodes: Vec<(String, NodeMode)>,
    /// Link states, sorted by link id.
    pub links: Vec<LinkObservation>,
    /// Capabilities running on degraded minima.
    pub degraded: Vec<Capability>,
}

impl WorldObservation {
    /// Mode of `node`, if present.
    pub fn node_mode(&self, node: &str) -> Option<NodeMode> {
        self.nodes.iter().find(|(n, _)| n == node).map(|(_, m)| *m)
    }

    /// State of `link`, if present.
    pub fn link(&self, link: &str) -> Option<&LinkObservation> {
        self.links.iter().find(|l| l.link == link)
    }

    fn to_pb(&self) -> pb::WorldObservation {
        pb::WorldObservation {
            tick: self.tick,
            t_unix_ms: self.t_unix_ms,
            nodes: self
                .nodes
                .iter()
                .map(|(n, m)| pb::NodeObservation {
                    node: n.clone(),
                    mode: m.to_pb() as i32,
                })
                .collect(),
            links: self
                .links
                .iter()
                .map(|l| pb::LinkObservation {
                    link: l.link.clone(),
                    active: l.active,
                    up: l.up,
                    availability: l.availability,
                    latency_ms: l.latency_ms,
                    loss: l.loss,
                })
                .collect(),
            degraded: self.degraded.iter().map(|c| cap_to_pb(*c) as i32).collect(),
        }
    }

    fn from_pb(o: pb::WorldObservation) -> Result<Self, WireError> {
        const MAX_ENTRIES: usize = 256;
        if o.nodes.len() > MAX_ENTRIES || o.links.len() > MAX_ENTRIES {
            return Err(invalid("observation", "too many entries"));
        }
        let mut nodes = Vec::with_capacity(o.nodes.len());
        for n in o.nodes {
            validate_entity_id("observation.node", &n.node)?;
            nodes.push((n.node, NodeMode::from_pb(n.mode)?));
        }
        let mut links = Vec::with_capacity(o.links.len());
        for l in o.links {
            validate_entity_id("observation.link", &l.link)?;
            links.push(LinkObservation {
                link: l.link,
                active: l.active,
                up: l.up,
                availability: in_range("observation.availability", l.availability, 0.0, 1.0)?,
                latency_ms: in_range("observation.latency_ms", l.latency_ms, 0.0, f64::MAX)?,
                loss: in_range("observation.loss", l.loss, 0.0, 1.0)?,
            });
        }
        let degraded = o
            .degraded
            .into_iter()
            .map(|c| cap_from_pb("observation.degraded", c))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(WorldObservation {
            tick: o.tick,
            t_unix_ms: o.t_unix_ms,
            nodes,
            links,
            degraded,
        })
    }
}

/// A validated command acknowledgement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandAck {
    /// The command being acknowledged.
    pub command_id: String,
    /// Outcome.
    pub status: AckStatus,
    /// Diagnostic reason (empty on success).
    pub reason: String,
    /// Observable world state after the command was processed.
    pub observation: Option<WorldObservation>,
    /// Ack time.
    pub t_unix_ms: i64,
}

/// Encodes an ack body (the bytes that get signed).
pub fn encode_ack(ack: &CommandAck) -> Result<Vec<u8>, WireError> {
    validate_text("command_id", &ack.command_id, false)?;
    validate_text("reason", &ack.reason, true)?;
    Ok(pb::CommandAck {
        command_id: ack.command_id.clone(),
        status: ack.status.to_pb() as i32,
        reason: ack.reason.clone(),
        observation: ack.observation.as_ref().map(WorldObservation::to_pb),
        t_unix_ms: ack.t_unix_ms,
    }
    .encode_to_vec())
}

/// Decodes and validates an ack body.
pub fn decode_ack(bytes: &[u8]) -> Result<CommandAck, WireError> {
    if bytes.len() > crate::MAX_ENVELOPE_BYTES {
        return Err(WireError::TooLarge {
            size: bytes.len(),
            limit: crate::MAX_ENVELOPE_BYTES,
        });
    }
    let a = pb::CommandAck::decode(bytes).map_err(decode_err)?;
    validate_text("command_id", &a.command_id, false)?;
    validate_text("reason", &a.reason, true)?;
    Ok(CommandAck {
        command_id: a.command_id,
        status: AckStatus::from_pb(a.status)?,
        reason: a.reason,
        observation: a.observation.map(WorldObservation::from_pb).transpose()?,
        t_unix_ms: a.t_unix_ms,
    })
}

// ============================================================================
// Γ(a): action → ordered commands (§6.13)
// ============================================================================

/// Why an action cannot be turned into commands.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// `Combo` inside `Combo`.
    #[error("nested combo actions are not allowed")]
    NestedCombo,
    /// `Combo(vec![])`.
    #[error("empty combo action")]
    EmptyCombo,
    /// Two different operations on the same node in one action.
    #[error("conflicting operations on {0}")]
    Conflict(String),
    /// Invalid identifier.
    #[error("invalid action: {0}")]
    Invalid(#[from] WireError),
}

fn single(a: &aeris_types::Action) -> Result<Option<CommandOp>, PlanError> {
    use aeris_types::Action as A;
    Ok(match a {
        A::Monitor => None,
        A::Restrict(n) => Some(CommandOp::Restrict { node: n.clone() }),
        A::Isolate(n) => Some(CommandOp::Isolate { node: n.clone() }),
        A::Recover(n) => Some(CommandOp::Recover { node: n.clone() }),
        A::Reroute(l) => Some(CommandOp::ActivateLink { link: l.clone() }),
        A::Failover { from, to } => Some(CommandOp::Failover {
            from: from.clone(),
            to: to.clone(),
        }),
        A::Degrade(c) => Some(CommandOp::Degrade { capability: *c }),
        A::Combo(_) => return Err(PlanError::NestedCombo),
    })
}

fn phase(op: &CommandOp) -> u8 {
    // Paths first, so an isolation never runs before its replacement path
    // exists ("activate L2, then remove B from routing").
    match op {
        CommandOp::ActivateLink { .. } | CommandOp::Failover { .. } => 0,
        CommandOp::Degrade { .. } => 1,
        CommandOp::Restrict { .. } | CommandOp::Isolate { .. } | CommandOp::Recover { .. } => 2,
        CommandOp::Observe => 3,
    }
}

/// Γ(a): translates an approved action into the ordered list of concrete
/// commands. `Monitor` yields no commands. Used by both `aeris-exec`
/// (execution) and `aeris-sim` rollouts, so a simulated action has exactly the
/// effects its execution would have.
pub fn commands_for_action(action: &aeris_types::Action) -> Result<Vec<CommandOp>, PlanError> {
    let parts: Vec<CommandOp> = match action {
        aeris_types::Action::Combo(items) => {
            if items.is_empty() {
                return Err(PlanError::EmptyCombo);
            }
            let mut v = Vec::with_capacity(items.len());
            for a in items {
                if let Some(op) = single(a)? {
                    v.push(op);
                }
            }
            v
        }
        other => single(other)?.into_iter().collect(),
    };
    let mut out: Vec<CommandOp> = Vec::with_capacity(parts.len());
    for op in parts {
        op.validate()?;
        if out.contains(&op) {
            continue;
        }
        if let CommandOp::Isolate { node } | CommandOp::Restrict { node } | CommandOp::Recover { node } = &op
            && out.iter().any(|o| {
                matches!(o, CommandOp::Isolate { node: n } | CommandOp::Restrict { node: n } | CommandOp::Recover { node: n } if n == node)
            })
        {
            return Err(PlanError::Conflict(node.clone()));
        }
        out.push(op);
    }
    // Stable sort keeps the caller's order within a phase.
    out.sort_by_key(phase);
    Ok(out)
}
