//! World configuration (`docs/spec/world.toml`).

use crate::SimError;
use aeris_types::Capability;
use serde::{Deserialize, Serialize};

/// The frozen v0.1 world file, embedded at compile time so algorithm crates
/// need no filesystem access and every build uses the same world.
pub const FROZEN_WORLD_TOML: &str = include_str!("../../../docs/spec/world.toml");

/// Root of `world.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldConfig {
    /// Global settings.
    pub world: WorldMeta,
    /// Nodes A, B, C.
    pub nodes: Vec<NodeConfig>,
    /// Links L1, L2.
    pub links: Vec<LinkConfig>,
    /// Mission endpoints and minima.
    pub mission: MissionConfig,
    /// Sensor noise.
    pub sensors: SensorConfig,
    /// Normal host/network behaviour.
    pub telemetry: TelemetryConfig,
    /// Environmental disturbances W.
    pub environment: EnvironmentConfig,
    /// Effects of actions.
    pub actions: ActionEffects,
    /// AERIS's attacker model for rollouts.
    pub threat_model: ThreatModel,
}

/// `[actions]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionEffects {
    /// Lateral hazard multiplier while a node is restricted.
    pub restrict_hazard_factor: f64,
}

/// `[world]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldMeta {
    /// Simulation rate (10 Hz in v0.1).
    pub tick_hz: u32,
    /// Patrol area half width, metres.
    pub area_half_width_m: f64,
}

/// `[[nodes]]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    /// Node id.
    pub id: String,
    /// Initial position.
    pub position_m: [f64; 2],
    /// Initial velocity.
    pub velocity_mps: [f64; 2],
    /// Fraction of the area the node's camera covers.
    pub coverage: f64,
    /// Probability a healthy camera frame is usable.
    pub camera_quality: f64,
}

/// `[[links]]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkConfig {
    /// Link id.
    pub id: String,
    /// The two ends.
    pub endpoints: [String; 2],
    /// Relay nodes in path order.
    pub via: Vec<String>,
    /// Availability per hop (`via.len() + 1` entries).
    pub hop_availability: Vec<f64>,
    /// Latency per hop, ms.
    pub latency_ms: Vec<f64>,
    /// Loss per hop.
    pub loss: Vec<f64>,
    /// Routing enabled at start.
    pub active: bool,
}

impl LinkConfig {
    /// Node ids along the path, endpoints included.
    pub fn path(&self) -> Vec<&str> {
        let mut p = Vec::with_capacity(self.via.len() + 2);
        p.push(self.endpoints[0].as_str());
        p.extend(self.via.iter().map(String::as_str));
        p.push(self.endpoints[1].as_str());
        p
    }
}

/// Per-capability minima.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityMinima {
    /// Navigation minimum.
    pub navigation: f64,
    /// Communication minimum.
    pub communication: f64,
    /// Observation minimum.
    pub observation: f64,
}

impl CapabilityMinima {
    /// Minimum for one capability.
    pub fn get(&self, c: Capability) -> f64 {
        match c {
            Capability::Navigation => self.navigation,
            Capability::Communication => self.communication,
            Capability::Observation => self.observation,
        }
    }
}

/// `[mission]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionConfig {
    /// Communication endpoints.
    pub endpoints: [String; 2],
    /// Node hosting the ground uplink.
    pub uplink_node: String,
    /// σ_max for navigation, metres.
    pub sigma_max_m: f64,
    /// c_k^min.
    pub min: CapabilityMinima,
    /// Minima while `degrade(k)` is in force.
    pub degraded_min: CapabilityMinima,
}

/// `[sensors]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorConfig {
    /// GPS 1-σ, metres.
    pub gps_sigma_m: f64,
    /// Camera fix 1-σ, metres.
    pub camera_sigma_m: f64,
    /// IMU velocity 1-σ, m/s.
    pub imu_sigma_mps: f64,
    /// Localisation error growth without any fix, m/s.
    pub dead_reckoning_drift_mps: f64,
    /// Reported confidence for GPS readings.
    pub gps_confidence: f64,
    /// Reported confidence for camera readings.
    pub camera_confidence: f64,
    /// Reported confidence for IMU readings.
    pub imu_confidence: f64,
}

/// `[telemetry]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    /// CPU mean, %.
    pub cpu_mean_pct: f64,
    /// CPU sd, %.
    pub cpu_sd_pct: f64,
    /// Memory mean, %.
    pub mem_mean_pct: f64,
    /// Memory sd, %.
    pub mem_sd_pct: f64,
    /// Baseline process count.
    pub proc_count: u32,
    /// Packets/s mean.
    pub pkts_mean_per_s: f64,
    /// Packets/s sd.
    pub pkts_sd_per_s: f64,
    /// Bytes per packet.
    pub bytes_per_pkt: f64,
    /// Baseline peer count.
    pub peers: u32,
    /// Latency jitter sd, ms.
    pub latency_jitter_ms: f64,
    /// Legitimate login rate, Hz.
    pub legit_auth_rate_hz: f64,
    /// Legitimate login service.
    pub legit_auth_service: String,
    /// Legitimate login principal.
    pub legit_auth_principal: String,
    /// Link status report period, ticks.
    pub link_status_period_ticks: u32,
}

/// `[environment]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentConfig {
    /// Process noise on acceleration, m/s².
    pub process_noise_accel_mps2: f64,
    /// P(a hop starts fading) per tick.
    pub fade_prob_per_tick: f64,
    /// Fade length, ticks.
    pub fade_duration_ticks: u32,
    /// Availability multiplier during a fade.
    pub fade_availability_factor: f64,
}

/// `[threat_model]`: AERIS's assumption about a compromised node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreatModel {
    /// Integrity of traffic through a compromised node.
    pub relay_integrity: f64,
    /// Usable fraction of a compromised camera feed.
    pub observation_integrity: f64,
    /// P(spread to each reachable neighbour) per tick.
    pub lateral_hazard_per_tick: f64,
}

fn prob(field: &str, v: f64) -> Result<(), SimError> {
    if !(0.0..=1.0).contains(&v) || !v.is_finite() {
        return Err(SimError::Config(format!("{field} = {v} not in [0, 1]")));
    }
    Ok(())
}

fn positive(field: &str, v: f64) -> Result<(), SimError> {
    if !(v.is_finite() && v > 0.0) {
        return Err(SimError::Config(format!("{field} = {v} must be > 0")));
    }
    Ok(())
}

fn non_negative(field: &str, v: f64) -> Result<(), SimError> {
    if !(v.is_finite() && v >= 0.0) {
        return Err(SimError::Config(format!("{field} = {v} must be >= 0")));
    }
    Ok(())
}

impl WorldConfig {
    /// The frozen v0.1 world.
    pub fn frozen() -> Result<Self, SimError> {
        Self::from_toml_str(FROZEN_WORLD_TOML)
    }

    /// Parses and validates a world file.
    pub fn from_toml_str(text: &str) -> Result<Self, SimError> {
        let cfg: WorldConfig = toml::from_str(text).map_err(|e| SimError::Config(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Index of `id` in `nodes`.
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// Index of `id` in `links`.
    pub fn link_index(&self, id: &str) -> Option<usize> {
        self.links.iter().position(|l| l.id == id)
    }

    /// Checks internal consistency.
    pub fn validate(&self) -> Result<(), SimError> {
        if self.world.tick_hz == 0 {
            return Err(SimError::Config("tick_hz must be > 0".into()));
        }
        positive("area_half_width_m", self.world.area_half_width_m)?;
        if self.nodes.is_empty() || self.nodes.len() > 64 {
            return Err(SimError::Config("1..=64 nodes required".into()));
        }
        let mut seen = std::collections::BTreeSet::new();
        for n in &self.nodes {
            aeris_wire::validate_entity_id("node id", &n.id)
                .map_err(|e| SimError::Config(e.to_string()))?;
            if !seen.insert(n.id.clone()) {
                return Err(SimError::Config(format!("duplicate id {}", n.id)));
            }
            prob("coverage", n.coverage)?;
            prob("camera_quality", n.camera_quality)?;
            for v in n.position_m.iter().chain(n.velocity_mps.iter()) {
                if !v.is_finite() {
                    return Err(SimError::Config(format!("{}: non-finite kinematics", n.id)));
                }
            }
        }
        for l in &self.links {
            aeris_wire::validate_entity_id("link id", &l.id)
                .map_err(|e| SimError::Config(e.to_string()))?;
            if !seen.insert(l.id.clone()) {
                return Err(SimError::Config(format!("duplicate id {}", l.id)));
            }
            let hops = l.via.len() + 1;
            if l.hop_availability.len() != hops
                || l.latency_ms.len() != hops
                || l.loss.len() != hops
            {
                return Err(SimError::Config(format!(
                    "{}: hop_availability/latency_ms/loss need {hops} entries",
                    l.id
                )));
            }
            for n in l.path() {
                if self.node_index(n).is_none() {
                    return Err(SimError::Config(format!("{}: unknown node {n}", l.id)));
                }
            }
            for (i, &a) in l.hop_availability.iter().enumerate() {
                prob(&format!("{}.hop_availability[{i}]", l.id), a)?;
                prob(&format!("{}.loss[{i}]", l.id), l.loss[i])?;
                non_negative(&format!("{}.latency_ms[{i}]", l.id), l.latency_ms[i])?;
            }
        }
        let m = &self.mission;
        for e in m.endpoints.iter().chain(std::iter::once(&m.uplink_node)) {
            if self.node_index(e).is_none() {
                return Err(SimError::Config(format!("mission: unknown node {e}")));
            }
        }
        positive("sigma_max_m", m.sigma_max_m)?;
        for (name, mins) in [("min", &m.min), ("degraded_min", &m.degraded_min)] {
            prob(&format!("mission.{name}.navigation"), mins.navigation)?;
            prob(&format!("mission.{name}.communication"), mins.communication)?;
            prob(&format!("mission.{name}.observation"), mins.observation)?;
        }
        let s = &self.sensors;
        positive("gps_sigma_m", s.gps_sigma_m)?;
        positive("camera_sigma_m", s.camera_sigma_m)?;
        positive("imu_sigma_mps", s.imu_sigma_mps)?;
        non_negative("dead_reckoning_drift_mps", s.dead_reckoning_drift_mps)?;
        prob("gps_confidence", s.gps_confidence)?;
        prob("camera_confidence", s.camera_confidence)?;
        prob("imu_confidence", s.imu_confidence)?;
        let t = &self.telemetry;
        for (f, v) in [
            ("cpu_mean_pct", t.cpu_mean_pct),
            ("cpu_sd_pct", t.cpu_sd_pct),
            ("mem_mean_pct", t.mem_mean_pct),
            ("mem_sd_pct", t.mem_sd_pct),
            ("pkts_mean_per_s", t.pkts_mean_per_s),
            ("pkts_sd_per_s", t.pkts_sd_per_s),
            ("bytes_per_pkt", t.bytes_per_pkt),
            ("latency_jitter_ms", t.latency_jitter_ms),
            ("legit_auth_rate_hz", t.legit_auth_rate_hz),
        ] {
            non_negative(f, v)?;
        }
        if t.link_status_period_ticks == 0 {
            return Err(SimError::Config(
                "link_status_period_ticks must be > 0".into(),
            ));
        }
        let e = &self.environment;
        non_negative("process_noise_accel_mps2", e.process_noise_accel_mps2)?;
        prob("fade_prob_per_tick", e.fade_prob_per_tick)?;
        prob("fade_availability_factor", e.fade_availability_factor)?;
        let tm = &self.threat_model;
        prob("threat_model.relay_integrity", tm.relay_integrity)?;
        prob(
            "threat_model.observation_integrity",
            tm.observation_integrity,
        )?;
        prob(
            "threat_model.lateral_hazard_per_tick",
            tm.lateral_hazard_per_tick,
        )?;
        prob(
            "actions.restrict_hazard_factor",
            self.actions.restrict_hazard_factor,
        )?;
        Ok(())
    }
}
