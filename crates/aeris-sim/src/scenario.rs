//! Scenario fault schedules (`scenarios/s1.toml` … `s8.toml`).

use crate::SimError;
use crate::config::WorldConfig;
use serde::{Deserialize, Serialize};

/// Built-in scenario files, embedded at compile time.
pub const BUILTIN: [(&str, &str); 8] = [
    ("s1", include_str!("../../../scenarios/s1.toml")),
    ("s2", include_str!("../../../scenarios/s2.toml")),
    ("s3", include_str!("../../../scenarios/s3.toml")),
    ("s4", include_str!("../../../scenarios/s4.toml")),
    ("s5", include_str!("../../../scenarios/s5.toml")),
    ("s6", include_str!("../../../scenarios/s6.toml")),
    ("s7", include_str!("../../../scenarios/s7.toml")),
    ("s8", include_str!("../../../scenarios/s8.toml")),
];

/// Ground-truth attacker parameters for a compromised node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttackerProfile {
    /// Fraction of relayed traffic forwarded faithfully.
    pub relay_integrity: f64,
    /// Usable fraction of the node's camera feed.
    pub observation_integrity: f64,
    /// P(spread to each reachable neighbour) per tick.
    pub lateral_hazard_per_tick: f64,
    /// Extra CPU load, %.
    pub cpu_extra_pct: f64,
    /// Extra memory use, %.
    pub mem_extra_pct: f64,
    /// Extra processes.
    pub rogue_procs: u32,
    /// Name of the implant process.
    pub rogue_process_name: String,
    /// Hex hash of the implant.
    pub rogue_process_hash: String,
    /// Beacon traffic, packets/s (stopped by restrict).
    pub beacon_pkts_per_sec: f64,
    /// Extra peers contacted (C2).
    pub beacon_peers: u32,
    /// Failed logins per second.
    pub auth_fail_rate_hz: f64,
    /// Service being brute-forced.
    pub auth_service: String,
    /// Principal being brute-forced.
    pub auth_principal: String,
    /// Offset the compromised node adds to its reported camera fixes.
    pub camera_report_offset_m: [f64; 2],
}

/// One injected fault. `start_s`/`end_s` are simulation seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Fault {
    /// GPS receiver failure: large noise and a wandering bias; reported σ unchanged.
    GpsFailure {
        /// Node.
        node: String,
        /// Start time.
        start_s: f64,
        /// Optional end time.
        #[serde(default)]
        end_s: Option<f64>,
        /// Extra 1-σ noise, metres.
        error_sigma_m: f64,
        /// Bias random-walk speed, m/s.
        bias_walk_mps: f64,
    },
    /// External GPS spoofing: readings and the node's own navigation are dragged.
    GpsSpoof {
        /// Node.
        node: String,
        /// Start time.
        start_s: f64,
        /// Optional end time.
        #[serde(default)]
        end_s: Option<f64>,
        /// Final offset, metres.
        offset_m: [f64; 2],
        /// Time to reach the full offset, s.
        ramp_s: f64,
    },
    /// Added latency/loss on every hop of a link.
    LinkDegradation {
        /// Link.
        link: String,
        /// Start time.
        start_s: f64,
        /// Optional end time.
        #[serde(default)]
        end_s: Option<f64>,
        /// Added latency per hop, ms.
        extra_latency_ms: f64,
        /// Added loss fraction per hop.
        extra_loss: f64,
    },
    /// Node takeover.
    Compromise {
        /// Node.
        node: String,
        /// Start time.
        start_s: f64,
        /// Optional end time (attacker leaves).
        #[serde(default)]
        end_s: Option<f64>,
        /// Ground-truth attacker.
        attacker: AttackerProfile,
    },
    /// The node lies about its position and flat-lines CPU telemetry.
    FalseTelemetry {
        /// Node.
        node: String,
        /// Start time.
        start_s: f64,
        /// Optional end time.
        #[serde(default)]
        end_s: Option<f64>,
        /// Reported position drift, m/s.
        position_drift_mps: [f64; 2],
        /// Report CPU as a constant.
        flatline_cpu: bool,
    },
    /// Hops cut (physically down) for the duration.
    Partition {
        /// Node pairs whose direct hop is cut.
        hops: Vec<[String; 2]>,
        /// Start time.
        start_s: f64,
        /// Optional end time.
        #[serde(default)]
        end_s: Option<f64>,
    },
    /// True accelerations far beyond the nominal process noise.
    ModelMismatch {
        /// Start time.
        start_s: f64,
        /// Optional end time.
        #[serde(default)]
        end_s: Option<f64>,
        /// True acceleration 1-σ, m/s².
        accel_sigma_mps2: f64,
    },
    /// Actuator fault for testing execution verification: commands that
    /// target `target` are acknowledged but have no effect (`ack_no_effect`)
    /// or are refused (`refuse`).
    ActuatorFault {
        /// Entity whose commands misbehave.
        target: String,
        /// Start time.
        start_s: f64,
        /// Optional end time.
        #[serde(default)]
        end_s: Option<f64>,
        /// Behaviour.
        behavior: ActuatorBehavior,
    },
}

/// Actuator fault behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActuatorBehavior {
    /// Report success, change nothing.
    AckNoEffect,
    /// Refuse the command.
    Refuse,
}

impl Fault {
    /// `(start_s, end_s)`.
    pub fn window(&self) -> (f64, Option<f64>) {
        match self {
            Fault::GpsFailure { start_s, end_s, .. }
            | Fault::GpsSpoof { start_s, end_s, .. }
            | Fault::LinkDegradation { start_s, end_s, .. }
            | Fault::Compromise { start_s, end_s, .. }
            | Fault::FalseTelemetry { start_s, end_s, .. }
            | Fault::Partition { start_s, end_s, .. }
            | Fault::ModelMismatch { start_s, end_s, .. }
            | Fault::ActuatorFault { start_s, end_s, .. } => (*start_s, *end_s),
        }
    }

    /// Short kind name.
    pub fn kind(&self) -> &'static str {
        match self {
            Fault::GpsFailure { .. } => "gps_failure",
            Fault::GpsSpoof { .. } => "gps_spoof",
            Fault::LinkDegradation { .. } => "link_degradation",
            Fault::Compromise { .. } => "compromise",
            Fault::FalseTelemetry { .. } => "false_telemetry",
            Fault::Partition { .. } => "partition",
            Fault::ModelMismatch { .. } => "model_mismatch",
            Fault::ActuatorFault { .. } => "actuator_fault",
        }
    }

    /// Node the fault targets, if it targets exactly one node. Used by the
    /// evaluator for precision/recall (ground truth only).
    pub fn target_node(&self) -> Option<&str> {
        match self {
            Fault::GpsFailure { node, .. }
            | Fault::GpsSpoof { node, .. }
            | Fault::Compromise { node, .. }
            | Fault::FalseTelemetry { node, .. } => Some(node),
            _ => None,
        }
    }
}

/// `[scenario]` header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioMeta {
    /// `s1` … `s8`.
    pub id: String,
    /// Human name.
    pub name: String,
    /// What is injected.
    pub description: String,
    /// Run length, seconds.
    pub duration_s: f64,
}

/// A scenario file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Header.
    pub scenario: ScenarioMeta,
    /// Faults (may be empty).
    #[serde(default)]
    pub faults: Vec<Fault>,
}

fn check_prob(f: &str, v: f64) -> Result<(), SimError> {
    if v.is_finite() && (0.0..=1.0).contains(&v) {
        Ok(())
    } else {
        Err(SimError::Scenario(format!("{f} = {v} not in [0, 1]")))
    }
}

fn check_nonneg(f: &str, v: f64) -> Result<(), SimError> {
    if v.is_finite() && v >= 0.0 {
        Ok(())
    } else {
        Err(SimError::Scenario(format!(
            "{f} = {v} must be finite and >= 0"
        )))
    }
}

impl Scenario {
    /// A built-in scenario by id (`s1`…`s8`).
    pub fn builtin(id: &str, world: &WorldConfig) -> Result<Self, SimError> {
        let text = BUILTIN
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, t)| *t)
            .ok_or_else(|| SimError::Scenario(format!("no built-in scenario {id:?}")))?;
        Self::from_toml_str(text, world)
    }

    /// Parses and validates against `world`.
    pub fn from_toml_str(text: &str, world: &WorldConfig) -> Result<Self, SimError> {
        let s: Scenario = toml::from_str(text).map_err(|e| SimError::Scenario(e.to_string()))?;
        s.validate(world)?;
        Ok(s)
    }

    /// Number of ticks the scenario runs.
    pub fn duration_ticks(&self, tick_hz: u32) -> u64 {
        (self.scenario.duration_s * f64::from(tick_hz)).round() as u64
    }

    /// Checks every reference and number.
    pub fn validate(&self, world: &WorldConfig) -> Result<(), SimError> {
        let m = &self.scenario;
        aeris_wire::validate_entity_id("scenario id", &m.id)
            .map_err(|e| SimError::Scenario(e.to_string()))?;
        if !(m.duration_s.is_finite() && m.duration_s > 0.0 && m.duration_s <= 86_400.0) {
            return Err(SimError::Scenario(format!(
                "duration_s = {} must be in (0, 86400]",
                m.duration_s
            )));
        }
        let node = |n: &str| {
            world
                .node_index(n)
                .map(|_| ())
                .ok_or_else(|| SimError::Scenario(format!("unknown node {n:?}")))
        };
        for (i, f) in self.faults.iter().enumerate() {
            let (start, end) = f.window();
            check_nonneg(&format!("faults[{i}].start_s"), start)?;
            if start > m.duration_s {
                return Err(SimError::Scenario(format!(
                    "faults[{i}] starts after the scenario ends"
                )));
            }
            if let Some(e) = end
                && !(e.is_finite() && e > start)
            {
                return Err(SimError::Scenario(format!(
                    "faults[{i}].end_s = {e} must be > start_s"
                )));
            }
            match f {
                Fault::GpsFailure {
                    node: n,
                    error_sigma_m,
                    bias_walk_mps,
                    ..
                } => {
                    node(n)?;
                    check_nonneg("error_sigma_m", *error_sigma_m)?;
                    check_nonneg("bias_walk_mps", *bias_walk_mps)?;
                }
                Fault::GpsSpoof {
                    node: n,
                    offset_m,
                    ramp_s,
                    ..
                } => {
                    node(n)?;
                    check_nonneg("ramp_s", *ramp_s)?;
                    if !offset_m.iter().all(|v| v.is_finite()) {
                        return Err(SimError::Scenario("offset_m must be finite".into()));
                    }
                }
                Fault::LinkDegradation {
                    link,
                    extra_latency_ms,
                    extra_loss,
                    ..
                } => {
                    if world.link_index(link).is_none() {
                        return Err(SimError::Scenario(format!("unknown link {link:?}")));
                    }
                    check_nonneg("extra_latency_ms", *extra_latency_ms)?;
                    check_prob("extra_loss", *extra_loss)?;
                }
                Fault::Compromise {
                    node: n, attacker, ..
                } => {
                    node(n)?;
                    let a = attacker;
                    check_prob("relay_integrity", a.relay_integrity)?;
                    check_prob("observation_integrity", a.observation_integrity)?;
                    check_prob("lateral_hazard_per_tick", a.lateral_hazard_per_tick)?;
                    check_nonneg("cpu_extra_pct", a.cpu_extra_pct)?;
                    check_nonneg("mem_extra_pct", a.mem_extra_pct)?;
                    check_nonneg("beacon_pkts_per_sec", a.beacon_pkts_per_sec)?;
                    check_nonneg("auth_fail_rate_hz", a.auth_fail_rate_hz)?;
                    if a.auth_fail_rate_hz > f64::from(world.world.tick_hz) {
                        return Err(SimError::Scenario(
                            "auth_fail_rate_hz cannot exceed tick_hz".into(),
                        ));
                    }
                    if !a.camera_report_offset_m.iter().all(|v| v.is_finite()) {
                        return Err(SimError::Scenario(
                            "camera_report_offset_m must be finite".into(),
                        ));
                    }
                    for (f, s) in [
                        ("rogue_process_name", &a.rogue_process_name),
                        ("auth_service", &a.auth_service),
                        ("auth_principal", &a.auth_principal),
                    ] {
                        if s.is_empty() || s.len() > 64 || s.chars().any(char::is_control) {
                            return Err(SimError::Scenario(format!("{f} invalid")));
                        }
                    }
                    if a.rogue_process_hash.len() > 128
                        || !a.rogue_process_hash.chars().all(|c| c.is_ascii_hexdigit())
                    {
                        return Err(SimError::Scenario("rogue_process_hash must be hex".into()));
                    }
                }
                Fault::FalseTelemetry {
                    node: n,
                    position_drift_mps,
                    ..
                } => {
                    node(n)?;
                    if !position_drift_mps.iter().all(|v| v.is_finite()) {
                        return Err(SimError::Scenario(
                            "position_drift_mps must be finite".into(),
                        ));
                    }
                }
                Fault::Partition { hops, .. } => {
                    if hops.is_empty() {
                        return Err(SimError::Scenario("partition needs hops".into()));
                    }
                    for [a, b] in hops {
                        node(a)?;
                        node(b)?;
                        let exists = world.links.iter().any(|l| {
                            l.path()
                                .windows(2)
                                .any(|w| (w[0] == a && w[1] == b) || (w[0] == b && w[1] == a))
                        });
                        if !exists {
                            return Err(SimError::Scenario(format!("no hop {a}–{b} in any link")));
                        }
                    }
                }
                Fault::ModelMismatch {
                    accel_sigma_mps2, ..
                } => check_nonneg("accel_sigma_mps2", *accel_sigma_mps2)?,
                Fault::ActuatorFault { target, .. } => {
                    if world.node_index(target).is_none() && world.link_index(target).is_none() {
                        return Err(SimError::Scenario(format!("unknown target {target:?}")));
                    }
                }
            }
        }
        Ok(())
    }
}
