//! Core domain and API types for AERIS.
//!
//! Frozen contract specification defined in docs/v0.1/06-contracts.md.
//! Owner: KISHANTH

use serde::{Deserialize, Serialize};
use std::fmt;

// ============================================================================
// Identifiers
// ============================================================================

pub type EntityId = String;
pub type EvidenceId = String;
pub type EventId = String;
pub type GroupId = String;
pub type LinkId = String;
pub type IncidentId = String;
pub type DecisionId = String;
pub type OperatorId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Navigation,
    Communication,
    Observation,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::Navigation => write!(f, "navigation"),
            Capability::Communication => write!(f, "communication"),
            Capability::Observation => write!(f, "observation"),
        }
    }
}

// ============================================================================
// Evidence & Anomaly Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    NetworkAnomaly,
    AuthFailure,
    ProcessAnomaly,
    SensorDisagreement,
    TelemetryAnomaly,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub id: EvidenceId,
    pub entity: EntityId,
    pub kind: EvidenceKind,
    pub score: f64,
    pub event: EventId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupId>,
    #[serde(default)]
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Posterior {
    pub normal: f64,
    pub compromise: f64,
    pub sensor_failure: f64,
    pub misconfig: f64,
    pub environment: f64,
}

impl Default for Posterior {
    fn default() -> Self {
        Self {
            normal: 1.0,
            compromise: 0.0,
            sensor_failure: 0.0,
            misconfig: 0.0,
            environment: 0.0,
        }
    }
}

impl Posterior {
    pub fn normalized_entropy(&self) -> f64 {
        let probs = [
            self.normal,
            self.compromise,
            self.sensor_failure,
            self.misconfig,
            self.environment,
        ];
        let mut entropy = 0.0;
        for &p in &probs {
            if p > 1e-9 {
                entropy -= p * p.ln();
            }
        }
        // Normalize by ln(5)
        let max_entropy = (5.0f64).ln();
        (entropy / max_entropy).clamp(0.0, 1.0)
    }

    pub fn most_likely(&self) -> (&'static str, f64) {
        let mut best_name = "normal";
        let mut best_prob = self.normal;

        if self.compromise > best_prob {
            best_name = "compromise";
            best_prob = self.compromise;
        }
        if self.sensor_failure > best_prob {
            best_name = "sensor_failure";
            best_prob = self.sensor_failure;
        }
        if self.misconfig > best_prob {
            best_name = "misconfig";
            best_prob = self.misconfig;
        }
        if self.environment > best_prob {
            best_name = "environment";
            best_prob = self.environment;
        }
        (best_name, best_prob)
    }
}

// ============================================================================
// Trust Vector & Beta Distribution
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Beta {
    pub alpha: f64,
    pub beta: f64,
}

impl Beta {
    pub fn new(alpha: f64, beta: f64) -> Self {
        Self {
            alpha: alpha.max(1e-4),
            beta: beta.max(1e-4),
        }
    }

    #[inline]
    pub fn mean(&self) -> f64 {
        let denom = self.alpha + self.beta;
        if denom <= 0.0 {
            0.5
        } else {
            self.alpha / denom
        }
    }

    #[inline]
    pub fn var(&self) -> f64 {
        let sum = self.alpha + self.beta;
        if sum <= 0.0 {
            0.0
        } else {
            (self.alpha * self.beta) / (sum * sum * (sum + 1.0))
        }
    }

    #[inline]
    pub fn std_dev(&self) -> f64 {
        self.var().sqrt()
    }
}

impl Default for Beta {
    fn default() -> Self {
        // Prior Beta(9, 1) -> E[T] = 0.90
        Self::new(9.0, 1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLabel {
    Normal,
    Suspicious,
    Untrusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustDimension {
    Identity = 0,
    Integrity = 1,
    Behaviour = 2,
    Telemetry = 3,
    Network = 4,
    Provenance = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrustVector {
    pub dims: [Beta; 6],
}

impl Default for TrustVector {
    fn default() -> Self {
        Self {
            dims: [Beta::default(); 6],
        }
    }
}

impl TrustVector {
    pub fn new_with_prior(alpha: f64, beta: f64) -> Self {
        Self {
            dims: [Beta::new(alpha, beta); 6],
        }
    }

    #[inline]
    pub fn dimension(&self, dim: TrustDimension) -> Beta {
        self.dims[dim as usize]
    }

    #[inline]
    pub fn dimension_mut(&mut self, dim: TrustDimension) -> &mut Beta {
        &mut self.dims[dim as usize]
    }

    /// Geometric mean aggregation: T_i = prod_d (T_{i,d})^(omega_d)
    pub fn aggregate(&self, weights: Option<&[f64; 6]>) -> f64 {
        let default_w = [1.0 / 6.0; 6];
        let w = weights.unwrap_or(&default_w);

        let mut log_sum = 0.0;
        for i in 0..6 {
            let m = self.dims[i].mean().clamp(1e-6, 1.0);
            log_sum += w[i] * m.ln();
        }
        log_sum.exp()
    }

    /// Maximum uncertainty across dimensions (highest std dev)
    pub fn max_std_dev(&self) -> f64 {
        let mut max_sd = 0.0f64;
        for b in &self.dims {
            let sd = b.std_dev();
            if sd > max_sd {
                max_sd = sd;
            }
        }
        max_sd
    }

    pub fn label(&self, weights: Option<&[f64; 6]>) -> TrustLabel {
        let agg = self.aggregate(weights);
        if agg >= 0.75 {
            TrustLabel::Normal
        } else if agg >= 0.40 {
            TrustLabel::Suspicious
        } else {
            TrustLabel::Untrusted
        }
    }
}

// ============================================================================
// Actions, Constraints & Optimization Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Monitor,
    Restrict(EntityId),
    Isolate(EntityId),
    Reroute(LinkId),
    Failover { from: EntityId, to: EntityId },
    Degrade(Capability),
    Recover(EntityId),
    Combo(Vec<Action>),
}

impl Action {
    pub fn disruption_cost(&self) -> f64 {
        match self {
            Action::Monitor => 0.00,
            Action::Restrict(_) => 0.05,
            Action::Reroute(_) => 0.05,
            Action::Isolate(_) => 0.10,
            Action::Failover { .. } => 0.10,
            Action::Degrade(_) => 0.30,
            Action::Recover(_) => 0.02,
            Action::Combo(actions) => {
                let has_isolate = actions.iter().any(|a| matches!(a, Action::Isolate(_)));
                let has_reroute = actions.iter().any(|a| matches!(a, Action::Reroute(_)));
                if has_isolate && has_reroute && actions.len() == 2 {
                    0.15
                } else {
                    actions.iter().map(|a| a.disruption_cost()).sum::<f64>().min(1.0)
                }
            }
        }
    }

    pub fn is_reversible(&self) -> bool {
        match self {
            Action::Degrade(_) => false,
            _ => true,
        }
    }

    pub fn impact_class(&self, target_criticality: f64) -> ImpactClass {
        match self {
            Action::Monitor | Action::Restrict(_) | Action::Recover(_) => ImpactClass::Low,
            Action::Reroute(_) => ImpactClass::Medium,
            Action::Degrade(_) => ImpactClass::Medium,
            Action::Failover { .. } => ImpactClass::Medium,
            Action::Isolate(_) => {
                if target_criticality >= 0.50 {
                    ImpactClass::High
                } else {
                    ImpactClass::Medium
                }
            }
            Action::Combo(actions) => {
                let mut highest = ImpactClass::Low;
                for a in actions {
                    let c = a.impact_class(target_criticality);
                    if c > highest {
                        highest = c;
                    }
                }
                highest
            }
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            Action::Monitor => "Monitor".to_string(),
            Action::Restrict(id) => format!("Restrict {}", id),
            Action::Isolate(id) => format!("Isolate {}", id),
            Action::Reroute(link) => format!("Reroute via {}", link),
            Action::Failover { from, to } => format!("Failover {} -> {}", from, to),
            Action::Degrade(cap) => format!("Degrade {}", cap),
            Action::Recover(id) => format!("Recover {}", id),
            Action::Combo(acts) => {
                let names: Vec<String> = acts.iter().map(|a| a.display_name()).collect();
                names.join(" + ")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ImpactClass {
    Low = 0,
    Medium = 1,
    High = 2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(non_camel_case_types)]
pub enum Invariant {
    I1_NoSeverLastCommPath,
    I2_NoIsolateSoleCapabilityProvider,
    I3_EntityMustExistAndNotRedundant,
    I4_DegradedModelRestrictedToLow,
}

impl fmt::Display for Invariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Invariant::I1_NoSeverLastCommPath => write!(f, "I1: Never remove last active communication path"),
            Invariant::I2_NoIsolateSoleCapabilityProvider => write!(f, "I2: Never isolate sole provider of required capability"),
            Invariant::I3_EntityMustExistAndNotRedundant => write!(f, "I3: Target entity must exist and not be redundant"),
            Invariant::I4_DegradedModelRestrictedToLow => write!(f, "I4: While model degraded, only LOW impact actions allowed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub action: Action,
    pub p_success: f64,
    pub se: f64,
    pub exp_utility: f64,
    pub cvar: f64,
    pub score: f64,
    #[serde(default)]
    pub violations: Vec<Invariant>,
    pub disruption_cost: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Auto,
    HumanReview { reason: String },
    Reject { invariant: Invariant },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Mode {
    Normal,
    Degraded,
    Containment,
    HumanReview,
    Recovery,
    Failsafe,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::Normal => write!(f, "NORMAL"),
            Mode::Degraded => write!(f, "DEGRADED"),
            Mode::Containment => write!(f, "CONTAINMENT"),
            Mode::HumanReview => write!(f, "HUMAN_REVIEW"),
            Mode::Recovery => write!(f, "RECOVERY"),
            Mode::Failsafe => write!(f, "FAILSAFE"),
        }
    }
}

// ============================================================================
// Incident & Proposal
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentStatus {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Incident {
    pub id: IncidentId,
    pub entity: EntityId,
    pub opened_at_tick: u64,
    pub status: IncidentStatus,
    pub posterior: Posterior,
    pub evidence_ids: Vec<EvidenceId>,
    pub risk: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: DecisionId,
    pub incident_id: IncidentId,
    pub recommended_action: Action,
    pub candidates: Vec<Candidate>,
    pub gate: Gate,
    pub explanation: String,
    pub cited_evidence: Vec<EvidenceId>,
}

// ============================================================================
// Wire / Event Types
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Hlc {
    pub l: i64,
    pub c: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPayload {
    Telemetry { cpu: f64, mem: f64, proc_count: u32 },
    Net { pkts_per_sec: f64, bytes_per_sec: f64, loss: f64, latency_ms: f64 },
    Auth { service: String, success: bool, principal: String },
    Process { name: String, started: bool, hash: String },
    Sensor { kind: String, value: Vec<f64>, sigma: f64 },
    Link { link_id: LinkId, up: bool, availability: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifiedEvent {
    pub id: EventId,
    pub source: EntityId,
    pub seq: u64,
    pub hlc: Hlc,
    pub t_unix_ms: i64,
    pub replayed: bool,
    pub payload: EventPayload,
    pub signature_valid: bool,
}

// ============================================================================
// Cycle Output & State
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CycleOutput {
    pub tick: u64,
    pub mode: Mode,
    pub trust: Vec<(EntityId, TrustVector)>,
    pub sensor_trust: Vec<(EntityId, f64)>,
    pub capabilities: Vec<(Capability, f64)>,
    pub mission_score: f64,
    pub risks: Vec<(EntityId, f64)>,
    pub incidents: Vec<Incident>,
    pub proposal: Option<Proposal>,
}

// ============================================================================
// Stage Function Signatures (Contracts)
// ============================================================================

pub type AnomalyDetectorFn = fn(events: &[VerifiedEvent]) -> Vec<Evidence>;
pub type HypothesisFusionFn = fn(evidence: &[Evidence]) -> Posterior;
pub type TrustUpdaterFn = fn(current: &mut TrustVector, evidence: &[Evidence], dt: f64);
pub type ActionGeneratorFn = fn(entity: &EntityId, active_links: &[LinkId]) -> Vec<Action>;
pub type InvariantCheckerFn = fn(action: &Action, active_links: &[LinkId], active_nodes: &[EntityId]) -> Result<(), Invariant>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_beta_statistics() {
        let b = Beta::new(9.0, 1.0);
        assert!((b.mean() - 0.900).abs() < 1e-4);
        // var = (9 * 1) / (100 * 11) = 9 / 1100 ≈ 0.0081818
        assert!((b.var() - (9.0 / 1100.0)).abs() < 1e-6);
        assert!((b.std_dev() - (9.0 / 1100.0f64).sqrt()).abs() < 1e-6);
    }

    #[test]
    fn test_trust_vector_aggregation() {
        let mut tv = TrustVector::default();
        // Set dimensions as in worked example §6.6:
        // [identity: 0.97, integrity: 0.94, behaviour: 0.44, telemetry: 0.44, network: 0.22, provenance: 0.83]
        // Geometric mean: (0.97 * 0.94 * 0.44 * 0.44 * 0.22 * 0.83)^(1/6) ≈ 0.565
        tv.dims[0] = Beta::new(97.0, 3.0);   // ~0.97
        tv.dims[1] = Beta::new(94.0, 6.0);   // ~0.94
        tv.dims[2] = Beta::new(44.0, 56.0);  // ~0.44
        tv.dims[3] = Beta::new(44.0, 56.0);  // ~0.44
        tv.dims[4] = Beta::new(22.0, 78.0);  // ~0.22
        tv.dims[5] = Beta::new(83.0, 17.0);  // ~0.83

        let agg = tv.aggregate(None);
        assert!((agg - 0.565).abs() < 0.015, "Expected ~0.565, got {}", agg);
        assert_eq!(tv.label(None), TrustLabel::Suspicious);
    }

    #[test]
    fn test_action_disruption_costs() {
        assert_eq!(Action::Monitor.disruption_cost(), 0.00);
        assert_eq!(Action::Restrict("node-b".into()).disruption_cost(), 0.05);
        assert_eq!(Action::Reroute("link-l2".into()).disruption_cost(), 0.05);
        assert_eq!(Action::Isolate("node-b".into()).disruption_cost(), 0.10);

        let combo = Action::Combo(vec![
            Action::Isolate("node-b".into()),
            Action::Reroute("link-l2".into()),
        ]);
        assert_eq!(combo.disruption_cost(), 0.15);
    }

    #[test]
    fn test_serde_cycle_output() {
        let output = CycleOutput {
            tick: 42,
            mode: Mode::Containment,
            trust: vec![("node-b".into(), TrustVector::default())],
            sensor_trust: vec![("node-b/gps".into(), 0.85)],
            capabilities: vec![(Capability::Communication, 0.554)],
            mission_score: 0.615,
            risks: vec![("node-b".into(), 0.987)],
            incidents: vec![],
            proposal: None,
        };

        let json = serde_json::to_string(&output).expect("serialize");
        let deserialized: CycleOutput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.tick, 42);
        assert_eq!(deserialized.mode, Mode::Containment);
        assert_eq!(deserialized.mission_score, 0.615);
    }
}
