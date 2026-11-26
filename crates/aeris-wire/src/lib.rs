//! AERIS wire format: Protobuf types generated from `proto/aeris/v1/` and
//! validated, lossless conversions to and from domain types.
//!
//! Owner: DHARSHAN. Contract: `docs/v0.1/06-contracts.md` ("Wire format").
//!
//! # Layers
//!
//! * [`pb`] — the prost-generated structs. Never trust them directly: every
//!   field may be missing, out of range, or an unknown enum value.
//! * [`WireEvent`], [`Command`], [`CommandAck`] — validated domain mirrors of
//!   the wire messages. `decode_*` functions only return these after
//!   validation, and `encode_*` functions refuse to emit invalid ones.
//! * [`WireEvent::to_verified_event`] — the projection into
//!   [`aeris_types::VerifiedEvent`], the pipeline's input type. It drops the
//!   fields the domain type does not have (`confidence`, `provenance`,
//!   `NetStats.peers`); see the crate README in the implementation report.
//!
//! # Signed bytes
//!
//! Signatures cover the exact `body` bytes carried in an envelope. Receivers
//! must verify the bytes *as received* (see `aeris-crypto`) and must never
//! re-encode a decoded body to verify it: protobuf encoding is not canonical.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use aeris_types::{EventPayload, VerifiedEvent};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Generated protobuf types for package `aeris.v1`.
#[allow(missing_docs, clippy::all, clippy::pedantic)]
pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/aeris.v1.rs"));
}

pub mod hlc;
pub use hlc::{HlcClock, HlcError};

mod command;
pub use command::*;

// ============================================================================
// Limits (resource-exhaustion guards; checked before and after decoding)
// ============================================================================

/// Maximum size of an encoded envelope accepted from the bus.
pub const MAX_ENVELOPE_BYTES: usize = 64 * 1024;
/// Maximum length of any free-text string field (service, principal, name, ...).
pub const MAX_TEXT_LEN: usize = 256;
/// Maximum length of an entity identifier (`node-b`, `link-l2`).
pub const MAX_ENTITY_LEN: usize = 63;
/// Maximum number of values in one sensor reading.
pub const MAX_SENSOR_VALUES: usize = 8;
/// Ed25519 signature length in bytes.
pub const SIGNATURE_LEN: usize = 64;

/// NATS subject prefix for events (`aeris.events.<node>`).
pub const EVENTS_SUBJECT_PREFIX: &str = "aeris.events";
/// NATS subject prefix for commands (`aeris.cmd.<entity>`).
pub const CMD_SUBJECT_PREFIX: &str = "aeris.cmd";

/// Subject on which events from `node` are published.
pub fn event_subject(node: &str) -> String {
    format!("{EVENTS_SUBJECT_PREFIX}.{node}")
}

/// Subject on which commands targeting `entity` are sent.
pub fn cmd_subject(entity: &str) -> String {
    format!("{CMD_SUBJECT_PREFIX}.{entity}")
}

// ============================================================================
// Errors
// ============================================================================

/// Why a message was rejected by the wire layer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    /// The input exceeds a size limit.
    #[error("message too large: {size} bytes > {limit}")]
    TooLarge {
        /// Observed size.
        size: usize,
        /// Limit that was exceeded.
        limit: usize,
    },
    /// Protobuf decoding failed (truncated, malformed varint, wrong wire type...).
    #[error("malformed protobuf: {0}")]
    Malformed(String),
    /// A required field is absent.
    #[error("missing required field `{0}`")]
    Missing(&'static str),
    /// A field has a value outside its allowed domain.
    #[error("invalid field `{field}`: {reason}")]
    Invalid {
        /// Field name.
        field: &'static str,
        /// Human-readable reason.
        reason: String,
    },
    /// An enum field carries a value this build does not know.
    #[error("unknown enum value {value} for `{field}`")]
    UnknownEnum {
        /// Field name.
        field: &'static str,
        /// Raw value received.
        value: i32,
    },
}

pub(crate) fn invalid(field: &'static str, reason: impl Into<String>) -> WireError {
    WireError::Invalid {
        field,
        reason: reason.into(),
    }
}

fn decode_err(e: prost::DecodeError) -> WireError {
    WireError::Malformed(e.to_string())
}

// ============================================================================
// Field validators
// ============================================================================

/// Validates an entity identifier: 1–63 chars of `[a-z0-9-]`, starting with
/// an alphanumeric. These ids are used as NATS subject tokens, so `.`, `*`,
/// `>` and whitespace are excluded by construction.
pub fn validate_entity_id(field: &'static str, id: &str) -> Result<(), WireError> {
    if id.is_empty() {
        return Err(WireError::Missing(field));
    }
    if id.len() > MAX_ENTITY_LEN {
        return Err(invalid(field, format!("longer than {MAX_ENTITY_LEN}")));
    }
    let mut chars = id.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !first_ok
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(invalid(field, "must match [a-z0-9][a-z0-9-]*"));
    }
    Ok(())
}

/// Validates an event source: `<node>` or `<node>/<sensor>`.
pub fn validate_source(source: &str) -> Result<(), WireError> {
    match source.split_once('/') {
        None => validate_entity_id("source", source),
        Some((node, sensor)) => {
            validate_entity_id("source", node)?;
            validate_entity_id("source", sensor)
        }
    }
}

/// The node part of a source (`node-b/gps` → `node-b`). Keys are per node.
pub fn source_node(source: &str) -> &str {
    source.split_once('/').map_or(source, |(n, _)| n)
}

/// Validates a canonical (lowercase, hyphenated) UUIDv7 string. Non-canonical
/// spellings of the same UUID are rejected so they cannot bypass id-based
/// deduplication.
pub fn validate_uuid_v7(field: &'static str, s: &str) -> Result<uuid::Uuid, WireError> {
    if s.is_empty() {
        return Err(WireError::Missing(field));
    }
    let u = uuid::Uuid::try_parse(s).map_err(|e| invalid(field, e.to_string()))?;
    if u.get_version_num() != 7 {
        return Err(invalid(field, "must be a UUIDv7"));
    }
    if u.hyphenated().to_string() != s {
        return Err(invalid(
            field,
            "must be canonical lowercase hyphenated form",
        ));
    }
    Ok(u)
}

pub(crate) fn validate_text(
    field: &'static str,
    s: &str,
    allow_empty: bool,
) -> Result<(), WireError> {
    if s.is_empty() && !allow_empty {
        return Err(WireError::Missing(field));
    }
    if s.len() > MAX_TEXT_LEN {
        return Err(invalid(field, format!("longer than {MAX_TEXT_LEN} bytes")));
    }
    if s.chars().any(char::is_control) {
        return Err(invalid(field, "contains control characters"));
    }
    Ok(())
}

pub(crate) fn finite(field: &'static str, v: f64) -> Result<f64, WireError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(invalid(field, "must be finite"))
    }
}

pub(crate) fn in_range(field: &'static str, v: f64, lo: f64, hi: f64) -> Result<f64, WireError> {
    let v = finite(field, v)?;
    if v < lo || v > hi {
        return Err(invalid(field, format!("{v} not in [{lo}, {hi}]")));
    }
    Ok(v)
}

fn non_negative(field: &'static str, v: f64) -> Result<f64, WireError> {
    in_range(field, v, 0.0, f64::MAX)
}

// ============================================================================
// Event domain model
// ============================================================================

/// Kind of sensor producing a [`WirePayload::Sensor`] reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    /// Satellite positioning: value = `[x_m, y_m]`.
    Gps,
    /// Inertial unit: value = `[vx_mps, vy_mps]`.
    Imu,
    /// Camera position fix: value = `[x_m, y_m]`.
    Camera,
}

impl SensorKind {
    /// The lowercase name used in `aeris_types::EventPayload::Sensor::kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            SensorKind::Gps => "gps",
            SensorKind::Imu => "imu",
            SensorKind::Camera => "camera",
        }
    }

    /// Parses the lowercase name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "gps" => Some(SensorKind::Gps),
            "imu" => Some(SensorKind::Imu),
            "camera" => Some(SensorKind::Camera),
            _ => None,
        }
    }

    fn to_pb(self) -> pb::SensorKind {
        match self {
            SensorKind::Gps => pb::SensorKind::Gps,
            SensorKind::Imu => pb::SensorKind::Imu,
            SensorKind::Camera => pb::SensorKind::Camera,
        }
    }

    fn from_pb(raw: i32) -> Result<Self, WireError> {
        match pb::SensorKind::try_from(raw) {
            Ok(pb::SensorKind::Gps) => Ok(SensorKind::Gps),
            Ok(pb::SensorKind::Imu) => Ok(SensorKind::Imu),
            Ok(pb::SensorKind::Camera) => Ok(SensorKind::Camera),
            Ok(pb::SensorKind::Unspecified) => Err(WireError::Missing("sensor.kind")),
            Err(_) => Err(WireError::UnknownEnum {
                field: "sensor.kind",
                value: raw,
            }),
        }
    }
}

impl fmt::Display for SensorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Event payload with full wire fidelity (a superset of
/// [`aeris_types::EventPayload`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WirePayload {
    /// Host telemetry.
    Telemetry {
        /// CPU utilisation, percent.
        cpu_pct: f64,
        /// Memory utilisation, percent.
        mem_pct: f64,
        /// Number of running processes.
        proc_count: u32,
    },
    /// Network interface statistics.
    Net {
        /// Packets per second.
        pkts_per_sec: f64,
        /// Bytes per second.
        bytes_per_sec: f64,
        /// Distinct peers contacted in the last interval.
        peers: u32,
        /// Round-trip latency, ms.
        latency_ms: f64,
        /// Packet loss fraction.
        loss: f64,
    },
    /// Authentication attempt.
    Auth {
        /// Service name (`sshd`).
        service: String,
        /// Whether authentication succeeded.
        success: bool,
        /// Principal that attempted to authenticate.
        principal: String,
    },
    /// Process start/stop.
    Process {
        /// Process name.
        name: String,
        /// `true` = started, `false` = stopped.
        started: bool,
        /// Hex digest of the binary.
        hash: String,
    },
    /// Sensor reading.
    Sensor {
        /// Sensor kind.
        kind: SensorKind,
        /// Reading values (see [`SensorKind`]).
        value: Vec<f64>,
        /// Reported 1-sigma noise.
        sigma: f64,
    },
    /// Link status as seen by a link endpoint.
    Link {
        /// Link id.
        link_id: String,
        /// Physically connected.
        up: bool,
        /// Measured availability in `[0, 1]`.
        availability: f64,
    },
}

impl WirePayload {
    /// Short type name, matching the `oneof payload` field names.
    pub fn type_name(&self) -> &'static str {
        match self {
            WirePayload::Telemetry { .. } => "telemetry",
            WirePayload::Net { .. } => "net",
            WirePayload::Auth { .. } => "auth",
            WirePayload::Process { .. } => "proc",
            WirePayload::Sensor { .. } => "sensor",
            WirePayload::Link { .. } => "link",
        }
    }

    /// Validates every field.
    pub fn validate(&self) -> Result<(), WireError> {
        match self {
            WirePayload::Telemetry {
                cpu_pct, mem_pct, ..
            } => {
                in_range("telemetry.cpu_pct", *cpu_pct, 0.0, 100.0)?;
                in_range("telemetry.mem_pct", *mem_pct, 0.0, 100.0)?;
            }
            WirePayload::Net {
                pkts_per_sec,
                bytes_per_sec,
                latency_ms,
                loss,
                ..
            } => {
                non_negative("net.pkts_per_sec", *pkts_per_sec)?;
                non_negative("net.bytes_per_sec", *bytes_per_sec)?;
                non_negative("net.latency_ms", *latency_ms)?;
                in_range("net.loss", *loss, 0.0, 1.0)?;
            }
            WirePayload::Auth {
                service, principal, ..
            } => {
                validate_text("auth.service", service, false)?;
                validate_text("auth.principal", principal, true)?;
            }
            WirePayload::Process { name, hash, .. } => {
                validate_text("proc.name", name, false)?;
                if hash.len() > 128 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(invalid("proc.hash", "must be <=128 hex chars"));
                }
            }
            WirePayload::Sensor { value, sigma, .. } => {
                if value.is_empty() {
                    return Err(WireError::Missing("sensor.value"));
                }
                if value.len() > MAX_SENSOR_VALUES {
                    return Err(invalid(
                        "sensor.value",
                        format!("more than {MAX_SENSOR_VALUES} values"),
                    ));
                }
                for v in value {
                    finite("sensor.value", *v)?;
                }
                let s = finite("sensor.sigma", *sigma)?;
                if s <= 0.0 {
                    return Err(invalid("sensor.sigma", "must be > 0"));
                }
            }
            WirePayload::Link {
                link_id,
                availability,
                ..
            } => {
                validate_entity_id("link.link_id", link_id)?;
                in_range("link.availability", *availability, 0.0, 1.0)?;
            }
        }
        Ok(())
    }

    fn to_pb(&self) -> pb::body::Payload {
        use pb::body::Payload as P;
        match self.clone() {
            WirePayload::Telemetry {
                cpu_pct,
                mem_pct,
                proc_count,
            } => P::Telemetry(pb::Telemetry {
                cpu_pct,
                mem_pct,
                proc_count,
            }),
            WirePayload::Net {
                pkts_per_sec,
                bytes_per_sec,
                peers,
                latency_ms,
                loss,
            } => P::Net(pb::NetStats {
                pkts_per_sec,
                bytes_per_sec,
                peers,
                latency_ms,
                loss,
            }),
            WirePayload::Auth {
                service,
                success,
                principal,
            } => P::Auth(pb::AuthEvent {
                service,
                success,
                principal,
            }),
            WirePayload::Process {
                name,
                started,
                hash,
            } => P::Proc(pb::ProcessEvent {
                name,
                started,
                hash,
            }),
            WirePayload::Sensor { kind, value, sigma } => P::Sensor(pb::SensorReading {
                kind: kind.to_pb() as i32,
                value,
                sigma,
            }),
            WirePayload::Link {
                link_id,
                up,
                availability,
            } => P::Link(pb::LinkStatus {
                link_id,
                up,
                availability,
            }),
        }
    }

    fn from_pb(p: pb::body::Payload) -> Result<Self, WireError> {
        use pb::body::Payload as P;
        let out = match p {
            P::Telemetry(t) => WirePayload::Telemetry {
                cpu_pct: t.cpu_pct,
                mem_pct: t.mem_pct,
                proc_count: t.proc_count,
            },
            P::Net(n) => WirePayload::Net {
                pkts_per_sec: n.pkts_per_sec,
                bytes_per_sec: n.bytes_per_sec,
                peers: n.peers,
                latency_ms: n.latency_ms,
                loss: n.loss,
            },
            P::Auth(a) => WirePayload::Auth {
                service: a.service,
                success: a.success,
                principal: a.principal,
            },
            P::Proc(p) => WirePayload::Process {
                name: p.name,
                started: p.started,
                hash: p.hash,
            },
            P::Sensor(s) => WirePayload::Sensor {
                kind: SensorKind::from_pb(s.kind)?,
                value: s.value,
                sigma: s.sigma,
            },
            P::Link(l) => WirePayload::Link {
                link_id: l.link_id,
                up: l.up,
                availability: l.availability,
            },
        };
        out.validate()?;
        Ok(out)
    }

    /// Projection into the pipeline's payload type. Lossy only for
    /// `Net.peers`, which `aeris_types::EventPayload::Net` does not carry.
    pub fn to_domain(&self) -> EventPayload {
        match self.clone() {
            WirePayload::Telemetry {
                cpu_pct,
                mem_pct,
                proc_count,
            } => EventPayload::Telemetry {
                cpu: cpu_pct,
                mem: mem_pct,
                proc_count,
            },
            WirePayload::Net {
                pkts_per_sec,
                bytes_per_sec,
                latency_ms,
                loss,
                peers: _,
            } => EventPayload::Net {
                pkts_per_sec,
                bytes_per_sec,
                loss,
                latency_ms,
            },
            WirePayload::Auth {
                service,
                success,
                principal,
            } => EventPayload::Auth {
                service,
                success,
                principal,
            },
            WirePayload::Process {
                name,
                started,
                hash,
            } => EventPayload::Process {
                name,
                started,
                hash,
            },
            WirePayload::Sensor { kind, value, sigma } => EventPayload::Sensor {
                kind: kind.as_str().to_string(),
                value,
                sigma,
            },
            WirePayload::Link {
                link_id,
                up,
                availability,
            } => EventPayload::Link {
                link_id,
                up,
                availability,
            },
        }
    }

    /// Builds a wire payload from the pipeline's payload type. The domain type
    /// has no `peers` count, so `Net` payloads get `peers = 0`; producers that
    /// know the count should construct [`WirePayload::Net`] directly.
    pub fn from_domain(p: &EventPayload) -> Result<Self, WireError> {
        let out = match p.clone() {
            EventPayload::Telemetry {
                cpu,
                mem,
                proc_count,
            } => WirePayload::Telemetry {
                cpu_pct: cpu,
                mem_pct: mem,
                proc_count,
            },
            EventPayload::Net {
                pkts_per_sec,
                bytes_per_sec,
                loss,
                latency_ms,
            } => WirePayload::Net {
                pkts_per_sec,
                bytes_per_sec,
                peers: 0,
                latency_ms,
                loss,
            },
            EventPayload::Auth {
                service,
                success,
                principal,
            } => WirePayload::Auth {
                service,
                success,
                principal,
            },
            EventPayload::Process {
                name,
                started,
                hash,
            } => WirePayload::Process {
                name,
                started,
                hash,
            },
            EventPayload::Sensor { kind, value, sigma } => WirePayload::Sensor {
                kind: SensorKind::parse(&kind)
                    .ok_or_else(|| invalid("sensor.kind", format!("unknown kind {kind:?}")))?,
                value,
                sigma,
            },
            EventPayload::Link {
                link_id,
                up,
                availability,
            } => WirePayload::Link {
                link_id,
                up,
                availability,
            },
        };
        out.validate()?;
        Ok(out)
    }
}

/// Where a reading came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// Producing component (`aeris-sim/node-b/gps`).
    pub producer: String,
    /// Producer version (`0.1.0`).
    pub producer_version: String,
}

/// A fully validated event body: `e = (id, source, seq, hlc, type, payload,
/// confidence, provenance)` from §6.2. The signature lives in the envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireEvent {
    /// UUIDv7, canonical form.
    pub id: String,
    /// `node-b` or `node-b/gps`.
    pub source: String,
    /// Per-source sequence number, `>= 1`.
    pub seq: u64,
    /// Hybrid logical clock at send time.
    pub hlc: aeris_types::Hlc,
    /// Observation time, ms since the Unix epoch.
    pub t_unix_ms: i64,
    /// Sent from a partition/outage buffer.
    pub replayed: bool,
    /// Producer's confidence, `[0, 1]`.
    pub confidence: f64,
    /// Producer identity.
    pub provenance: Provenance,
    /// Payload (its variant is the event type).
    pub payload: WirePayload,
}

impl WireEvent {
    /// Validates every field.
    pub fn validate(&self) -> Result<(), WireError> {
        validate_uuid_v7("id", &self.id)?;
        validate_source(&self.source)?;
        if self.seq == 0 {
            return Err(invalid("seq", "must be >= 1"));
        }
        if self.hlc.l < 0 {
            return Err(invalid("hlc.l", "must be >= 0"));
        }
        if self.t_unix_ms <= 0 {
            return Err(invalid("t_unix_ms", "must be > 0"));
        }
        in_range("confidence", self.confidence, 0.0, 1.0)?;
        validate_text("provenance.producer", &self.provenance.producer, false)?;
        validate_text(
            "provenance.producer_version",
            &self.provenance.producer_version,
            true,
        )?;
        self.payload.validate()
    }

    /// The node whose key signs this event.
    pub fn node(&self) -> &str {
        source_node(&self.source)
    }

    /// Converts to the generated protobuf struct.
    pub fn to_pb(&self) -> pb::Body {
        pb::Body {
            id: self.id.clone(),
            source: self.source.clone(),
            seq: self.seq,
            hlc: Some(pb::Hlc {
                l: self.hlc.l,
                c: self.hlc.c,
            }),
            t_unix_ms: self.t_unix_ms,
            replayed: self.replayed,
            confidence: Some(self.confidence),
            provenance: Some(pb::Provenance {
                producer: self.provenance.producer.clone(),
                producer_version: self.provenance.producer_version.clone(),
            }),
            payload: Some(self.payload.to_pb()),
        }
    }

    /// Validating conversion from the generated protobuf struct.
    pub fn from_pb(b: pb::Body) -> Result<Self, WireError> {
        let hlc = b.hlc.ok_or(WireError::Missing("hlc"))?;
        let prov = b.provenance.ok_or(WireError::Missing("provenance"))?;
        let payload = b.payload.ok_or(WireError::Missing("payload"))?;
        let ev = WireEvent {
            id: b.id,
            source: b.source,
            seq: b.seq,
            hlc: aeris_types::Hlc { l: hlc.l, c: hlc.c },
            t_unix_ms: b.t_unix_ms,
            replayed: b.replayed,
            confidence: b.confidence.ok_or(WireError::Missing("confidence"))?,
            provenance: Provenance {
                producer: prov.producer,
                producer_version: prov.producer_version,
            },
            payload: WirePayload::from_pb(payload)?,
        };
        ev.validate()?;
        Ok(ev)
    }

    /// Projects into the pipeline's input type. `signature_valid` must come
    /// from `aeris-crypto` verification of the received bytes, never from the
    /// sender.
    pub fn to_verified_event(&self, signature_valid: bool) -> VerifiedEvent {
        VerifiedEvent {
            id: self.id.clone(),
            source: self.source.clone(),
            seq: self.seq,
            hlc: self.hlc,
            t_unix_ms: self.t_unix_ms,
            replayed: self.replayed,
            payload: self.payload.to_domain(),
            signature_valid,
        }
    }
}

/// Validates and encodes an event body. The returned bytes are what gets
/// signed and transmitted.
pub fn encode_body(ev: &WireEvent) -> Result<Vec<u8>, WireError> {
    ev.validate()?;
    Ok(ev.to_pb().encode_to_vec())
}

/// Decodes and validates an event body.
pub fn decode_body(bytes: &[u8]) -> Result<WireEvent, WireError> {
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(WireError::TooLarge {
            size: bytes.len(),
            limit: MAX_ENVELOPE_BYTES,
        });
    }
    let b = pb::Body::decode(bytes).map_err(decode_err)?;
    WireEvent::from_pb(b)
}

/// An envelope split into the exact body bytes and the 64-byte signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEnvelope {
    /// Body bytes exactly as received.
    pub body: Vec<u8>,
    /// Detached signature.
    pub signature: [u8; SIGNATURE_LEN],
}

/// Encodes an envelope around already-encoded body bytes.
pub fn encode_envelope(body: &[u8], signature: &[u8; SIGNATURE_LEN]) -> Vec<u8> {
    pb::Envelope {
        body: body.to_vec(),
        signature: signature.to_vec(),
    }
    .encode_to_vec()
}

/// Splits a received envelope. Enforces the size limit before decoding.
pub fn decode_envelope(bytes: &[u8]) -> Result<RawEnvelope, WireError> {
    split_envelope(bytes, |b| {
        pb::Envelope::decode(b).map(|e| (e.body, e.signature))
    })
}

pub(crate) fn split_envelope(
    bytes: &[u8],
    decode: impl FnOnce(&[u8]) -> Result<(Vec<u8>, Vec<u8>), prost::DecodeError>,
) -> Result<RawEnvelope, WireError> {
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(WireError::TooLarge {
            size: bytes.len(),
            limit: MAX_ENVELOPE_BYTES,
        });
    }
    let (body, sig) = decode(bytes).map_err(decode_err)?;
    if body.is_empty() {
        return Err(WireError::Missing("body"));
    }
    let signature: [u8; SIGNATURE_LEN] = sig.as_slice().try_into().map_err(|_| {
        invalid(
            "signature",
            format!("expected {SIGNATURE_LEN} bytes, got {}", sig.len()),
        )
    })?;
    Ok(RawEnvelope { body, signature })
}

#[cfg(test)]
mod tests;
