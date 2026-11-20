//! Trust estimation, Beta dynamics, evidence weighting, and uncertainty tracking.
//!
//! Defined in docs/v0.1/04-mathematical-model.md §6.6.
//! Owner: KISHANTH

use aeris_types::{
    Beta, EntityId, Evidence, EvidenceKind, TrustDimension, TrustLabel, TrustVector,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Parameters governing trust updates and decay.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrustParams {
    /// Forgetting factor gamma = exp(-lambda_d * delta_t), default 0.95
    pub gamma: f64,
    /// Default evidence weight, default 1.0
    pub default_weight: f64,
    /// Weights for the 6 trust dimensions (sum to 1.0)
    pub dimension_weights: [f64; 6],
    /// UI label threshold for NORMAL
    pub normal_threshold: f64,
    /// UI label threshold for SUSPICIOUS
    pub suspicious_threshold: f64,
}

impl Default for TrustParams {
    fn default() -> Self {
        Self {
            gamma: 0.95,
            default_weight: 1.0,
            dimension_weights: [1.0 / 6.0; 6],
            normal_threshold: 0.75,
            suspicious_threshold: 0.40,
        }
    }
}

/// Dynamic tracker for an entity's trust vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityTrustTracker {
    pub entity: EntityId,
    pub vector: TrustVector,
    pub params: TrustParams,
}

impl EntityTrustTracker {
    pub fn new(entity: EntityId, params: TrustParams) -> Self {
        Self {
            entity,
            vector: TrustVector::default(),
            params,
        }
    }

    pub fn with_prior(entity: EntityId, alpha: f64, beta: f64, params: TrustParams) -> Self {
        Self {
            entity,
            vector: TrustVector::new_with_prior(alpha, beta),
            params,
        }
    }

    /// Single step update with forgetting:
    /// alpha_t = gamma * alpha_{t-1} + w * (1 - s_t)
    /// beta_t  = gamma * beta_{t-1}  + w * s_t
    pub fn update_dimension(&mut self, dim: TrustDimension, s_t: f64, weight: Option<f64>) {
        let w = weight.unwrap_or(self.params.default_weight);
        let s = s_t.clamp(0.0, 1.0);
        let gamma = self.params.gamma;

        let b = self.vector.dimension_mut(dim);
        b.alpha = gamma * b.alpha + w * (1.0 - s);
        b.beta = gamma * b.beta + w * s;

        // Ensure numerical sanity
        b.alpha = b.alpha.max(1e-4);
        b.beta = b.beta.max(1e-4);
    }

    /// Update dimension directly with good/bad observation
    pub fn step_decay_only(&mut self, dim: TrustDimension, good_behaviour: bool) {
        let s = if good_behaviour { 0.05 } else { 0.90 };
        self.update_dimension(dim, s, None);
    }

    /// Update vector from batch of evidence in a tick.
    pub fn apply_evidence(&mut self, evidence_list: &[Evidence]) {
        // Collect highest anomaly score per dimension for this entity
        let mut dim_max_anomaly: [Option<f64>; 6] = [None; 6];
        let mut dim_weights: [f64; 6] = [1.0; 6];

        for ev in evidence_list {
            if ev.entity != self.entity {
                continue;
            }

            let dim = match ev.kind {
                EvidenceKind::NetworkAnomaly => TrustDimension::Network,
                EvidenceKind::AuthFailure => TrustDimension::Behaviour,
                EvidenceKind::ProcessAnomaly => TrustDimension::Behaviour,
                EvidenceKind::SensorDisagreement => TrustDimension::Telemetry,
                EvidenceKind::TelemetryAnomaly => TrustDimension::Telemetry,
                EvidenceKind::Other(_) => TrustDimension::Integrity,
            };

            let score = ev.score.clamp(0.0, 1.0);
            let idx = dim as usize;
            dim_max_anomaly[idx] = Some(match dim_max_anomaly[idx] {
                Some(prev) => prev.max(score),
                None => score,
            });

            // If evidence is part of a correlation group, discount weight
            if ev.group.is_some() {
                dim_weights[idx] = 0.5;
            }
        }

        // Apply updates across all dimensions
        for d in 0..6 {
            let dim = match d {
                0 => TrustDimension::Identity,
                1 => TrustDimension::Integrity,
                2 => TrustDimension::Behaviour,
                3 => TrustDimension::Telemetry,
                4 => TrustDimension::Network,
                _ => TrustDimension::Provenance,
            };

            let s = dim_max_anomaly[d].unwrap_or(0.02); // nominal healthy noise floor
            let w = dim_weights[d];
            self.update_dimension(dim, s, Some(w));
        }
    }

    /// Overall aggregated trust scalar: geometric mean
    #[inline]
    pub fn current_trust(&self) -> f64 {
        self.vector.aggregate(Some(&self.params.dimension_weights))
    }

    /// Current trust label for UI
    #[inline]
    pub fn current_label(&self) -> TrustLabel {
        let t = self.current_trust();
        if t >= self.params.normal_threshold {
            TrustLabel::Normal
        } else if t >= self.params.suspicious_threshold {
            TrustLabel::Suspicious
        } else {
            TrustLabel::Untrusted
        }
    }
}

/// Tracks sensor trust separately from node trust:
/// T_j^obs = P(reading from sensor j is reliable | E).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorTrustTracker {
    pub sensor_id: EntityId,
    pub beta: Beta,
    pub gamma: f64,
}

impl SensorTrustTracker {
    pub fn new(sensor_id: EntityId) -> Self {
        Self {
            sensor_id,
            beta: Beta::new(9.0, 1.0),
            gamma: 0.95,
        }
    }

    /// Update with sensor innovation or disagreement residual score s in [0, 1]
    pub fn update(&mut self, score: f64, weight: f64) {
        let s = score.clamp(0.0, 1.0);
        self.beta.alpha = self.gamma * self.beta.alpha + weight * (1.0 - s);
        self.beta.beta = self.gamma * self.beta.beta + weight * s;
        self.beta.alpha = self.beta.alpha.max(1e-4);
        self.beta.beta = self.beta.beta.max(1e-4);
    }

    #[inline]
    pub fn trust_score(&self) -> f64 {
        self.beta.mean()
    }
}

/// High-level registry managing trust vectors for all nodes and sensors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustRegistry {
    pub nodes: HashMap<EntityId, EntityTrustTracker>,
    pub sensors: HashMap<EntityId, SensorTrustTracker>,
    pub params: TrustParams,
}

impl TrustRegistry {
    pub fn new(params: TrustParams) -> Self {
        Self {
            nodes: HashMap::new(),
            sensors: HashMap::new(),
            params,
        }
    }

    pub fn register_node(&mut self, entity: EntityId) {
        self.register_node_with_prior(entity, 9.2, 0.8);
    }

    pub fn register_node_with_prior(&mut self, entity: EntityId, alpha: f64, beta: f64) {
        self.nodes.entry(entity.clone()).or_insert_with(|| {
            EntityTrustTracker::with_prior(entity, alpha, beta, self.params)
        });
    }

    pub fn register_sensor(&mut self, sensor_id: EntityId) {
        self.sensors.entry(sensor_id.clone()).or_insert_with(|| {
            SensorTrustTracker::new(sensor_id)
        });
    }

    pub fn update_from_evidence(&mut self, evidence_list: &[Evidence]) {
        for tracker in self.nodes.values_mut() {
            tracker.apply_evidence(evidence_list);
        }

        for ev in evidence_list {
            if let Some(sensor_tracker) = self.sensors.get_mut(&ev.entity) {
                let weight = if ev.group.is_some() { 0.5 } else { 1.0 };
                sensor_tracker.update(ev.score, weight);
            }
        }
    }

    pub fn node_trust(&self, entity: &str) -> Option<f64> {
        self.nodes.get(entity).map(|n| n.current_trust())
    }

    pub fn sensor_trust(&self, sensor_id: &str) -> Option<f64> {
        self.sensors.get(sensor_id).map(|s| s.trust_score())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies the exact worked numbers from docs/v0.1/04-mathematical-model.md §6.6:
    /// Starting from Beta(9, 1), trust 0.900.
    /// Behaviour evidence is s = 0.9 every tick, w = 1.0, gamma = 0.95:
    /// - after 3 ticks: alpha = 8.00, beta = 3.43, trust 0.700 ± 0.130
    /// - after 6 ticks: trust 0.565 ± 0.134
    /// - after 10 ticks: trust 0.442 ± 0.128
    #[test]
    fn test_worked_example_decay() {
        let params = TrustParams::default();
        let mut tracker = EntityTrustTracker::new("node-b".into(), params);

        // Tick 1, 2, 3
        for _ in 0..3 {
            tracker.update_dimension(TrustDimension::Behaviour, 0.9, Some(1.0));
        }
        let b3 = tracker.vector.dimension(TrustDimension::Behaviour);
        assert!((b3.alpha - 8.00).abs() < 0.05, "alpha expected ~8.00, got {}", b3.alpha);
        assert!((b3.beta - 3.43).abs() < 0.05, "beta expected ~3.43, got {}", b3.beta);
        assert!((b3.mean() - 0.700).abs() < 0.015, "trust expected ~0.700, got {}", b3.mean());
        assert!((b3.std_dev() - 0.130).abs() < 0.01, "std dev expected ~0.130, got {}", b3.std_dev());

        // Tick 4, 5, 6
        for _ in 3..6 {
            tracker.update_dimension(TrustDimension::Behaviour, 0.9, Some(1.0));
        }
        let b6 = tracker.vector.dimension(TrustDimension::Behaviour);
        assert!((b6.mean() - 0.565).abs() < 0.015, "trust expected ~0.565, got {}", b6.mean());
        assert!((b6.std_dev() - 0.134).abs() < 0.01, "std dev expected ~0.134, got {}", b6.std_dev());

        // Tick 7..10
        for _ in 6..10 {
            tracker.update_dimension(TrustDimension::Behaviour, 0.9, Some(1.0));
        }
        let b10 = tracker.vector.dimension(TrustDimension::Behaviour);
        assert!((b10.mean() - 0.442).abs() < 0.015, "trust expected ~0.442, got {}", b10.mean());
        assert!((b10.std_dev() - 0.128).abs() < 0.01, "std dev expected ~0.128, got {}", b10.std_dev());

        // Now test recovery: if B behaves well (s = 0.05)
        // after 10 good ticks: ~0.690
        for _ in 0..10 {
            tracker.update_dimension(TrustDimension::Behaviour, 0.05, Some(1.0));
        }
        let b_rec10 = tracker.vector.dimension(TrustDimension::Behaviour);
        assert!((b_rec10.mean() - 0.690).abs() < 0.03, "recovery at 10 ticks expected ~0.690, got {}", b_rec10.mean());

        // after 20 total good ticks: ~0.807
        for _ in 0..10 {
            tracker.update_dimension(TrustDimension::Behaviour, 0.05, Some(1.0));
        }
        let b_rec20 = tracker.vector.dimension(TrustDimension::Behaviour);
        assert!((b_rec20.mean() - 0.807).abs() < 0.03, "recovery at 20 ticks expected ~0.807, got {}", b_rec20.mean());

        // after 40 total good ticks: ~0.902
        for _ in 0..20 {
            tracker.update_dimension(TrustDimension::Behaviour, 0.05, Some(1.0));
        }
        let b_rec40 = tracker.vector.dimension(TrustDimension::Behaviour);
        assert!((b_rec40.mean() - 0.902).abs() < 0.03, "recovery at 40 ticks expected ~0.902, got {}", b_rec40.mean());
    }

    #[test]
    fn test_sensor_trust_separate_from_node() {
        let mut reg = TrustRegistry::new(TrustParams::default());
        reg.register_node("node-a".into());
        reg.register_sensor("node-a/gps".into());

        // Sensor has anomaly
        let ev = Evidence {
            id: "ev-01".into(),
            entity: "node-a/gps".into(),
            kind: EvidenceKind::SensorDisagreement,
            score: 0.95,
            event: "evt-01".into(),
            group: None,
            timestamp_ms: 1000,
        };

        reg.update_from_evidence(&[ev]);
        let s_trust = reg.sensor_trust("node-a/gps").unwrap();
        assert!(s_trust < 0.90);

        // Node trust remains high because GPS was addressed to sensor entity
        let n_trust = reg.node_trust("node-a").unwrap();
        assert!(n_trust > 0.85);
    }
}
