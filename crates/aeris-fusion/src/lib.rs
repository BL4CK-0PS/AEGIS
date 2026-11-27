//! Bayesian hypothesis fusion, correlation discounting, and robust aggregation.
//!
//! Defined in docs/v0.1/04-mathematical-model.md §6.5.
//! Owner: AMIRTHA
//!
//! Implements:
//! - P(H | E) proportional to P(H) * prod_k P(E_k | H)^(w_k) in log-space
//! - Correlation discounting: tempered weights w_k = 1 / |g|
//! - Robust aggregators: median, trimmed mean
//! - Calibrated posterior generation across 5 hypotheses:
//!   {normal, compromise, sensor_failure, misconfig, environment}

use aeris_types::{EntityId, Evidence, EvidenceKind, Posterior};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The five discrete hypotheses evaluated for an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hypothesis {
    Normal = 0,
    Compromise = 1,
    SensorFailure = 2,
    Misconfig = 3,
    Environment = 4,
}

impl Hypothesis {
    pub const ALL: [Hypothesis; 5] = [
        Hypothesis::Normal,
        Hypothesis::Compromise,
        Hypothesis::SensorFailure,
        Hypothesis::Misconfig,
        Hypothesis::Environment,
    ];
}

/// Priors over the 5 hypotheses.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FusionPriors {
    pub normal: f64,
    pub compromise: f64,
    pub sensor_failure: f64,
    pub misconfig: f64,
    pub environment: f64,
}

impl Default for FusionPriors {
    fn default() -> Self {
        // §6.5: Priors: normal 0.95, compromise 0.01, sensor failure 0.02, misconfiguration 0.015, environment 0.005
        Self {
            normal: 0.950,
            compromise: 0.010,
            sensor_failure: 0.020,
            misconfig: 0.015,
            environment: 0.005,
        }
    }
}

impl FusionPriors {
    pub fn get(&self, h: Hypothesis) -> f64 {
        match h {
            Hypothesis::Normal => self.normal,
            Hypothesis::Compromise => self.compromise,
            Hypothesis::SensorFailure => self.sensor_failure,
            Hypothesis::Misconfig => self.misconfig,
            Hypothesis::Environment => self.environment,
        }
    }
}

/// Conditional likelihood profile P(E_k = 1 | H) for a specific evidence kind.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LikelihoodProfile {
    pub normal: f64,
    pub compromise: f64,
    pub sensor_failure: f64,
    pub misconfig: f64,
    pub environment: f64,
}

impl LikelihoodProfile {
    pub fn get(&self, h: Hypothesis) -> f64 {
        match h {
            Hypothesis::Normal => self.normal,
            Hypothesis::Compromise => self.compromise,
            Hypothesis::SensorFailure => self.sensor_failure,
            Hypothesis::Misconfig => self.misconfig,
            Hypothesis::Environment => self.environment,
        }
    }
}

/// Registry of likelihoods across all evidence kinds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LikelihoodRegistry {
    pub profiles: HashMap<String, LikelihoodProfile>,
    pub default_profile: LikelihoodProfile,
}

impl Default for LikelihoodRegistry {
    fn default() -> Self {
        let mut profiles = HashMap::new();

        // Frozen likelihoods from §6.5 & docs/spec/likelihoods.toml:
        // NetworkAnomaly: compromise 0.85, normal 0.05, sensor_failure 0.05, misconfig 0.40, environment 0.20
        profiles.insert(
            "network_anomaly".into(),
            LikelihoodProfile {
                normal: 0.05,
                compromise: 0.85,
                sensor_failure: 0.05,
                misconfig: 0.40,
                environment: 0.20,
            },
        );

        // AuthFailure: compromise 0.70, normal 0.02, sensor_failure 0.01, misconfig 0.30, environment 0.02
        profiles.insert(
            "auth_failure".into(),
            LikelihoodProfile {
                normal: 0.02,
                compromise: 0.70,
                sensor_failure: 0.01,
                misconfig: 0.30,
                environment: 0.02,
            },
        );

        // ProcessAnomaly: compromise 0.75, normal 0.02, sensor_failure 0.01, misconfig 0.20, environment 0.01
        profiles.insert(
            "process_anomaly".into(),
            LikelihoodProfile {
                normal: 0.02,
                compromise: 0.75,
                sensor_failure: 0.01,
                misconfig: 0.20,
                environment: 0.01,
            },
        );

        // SensorDisagreement: compromise 0.60, normal 0.05, sensor_failure 0.90, misconfig 0.10, environment 0.50
        profiles.insert(
            "sensor_disagreement".into(),
            LikelihoodProfile {
                normal: 0.05,
                compromise: 0.60,
                sensor_failure: 0.90,
                misconfig: 0.10,
                environment: 0.50,
            },
        );

        // TelemetryAnomaly
        profiles.insert(
            "telemetry_anomaly".into(),
            LikelihoodProfile {
                normal: 0.05,
                compromise: 0.65,
                sensor_failure: 0.30,
                misconfig: 0.20,
                environment: 0.05,
            },
        );

        let default_profile = LikelihoodProfile {
            normal: 0.10,
            compromise: 0.50,
            sensor_failure: 0.20,
            misconfig: 0.20,
            environment: 0.10,
        };

        Self {
            profiles,
            default_profile,
        }
    }
}

impl LikelihoodRegistry {
    pub fn get_profile(&self, kind: &EvidenceKind) -> &LikelihoodProfile {
        let key = match kind {
            EvidenceKind::NetworkAnomaly => "network_anomaly",
            EvidenceKind::AuthFailure => "auth_failure",
            EvidenceKind::ProcessAnomaly => "process_anomaly",
            EvidenceKind::SensorDisagreement => "sensor_disagreement",
            EvidenceKind::TelemetryAnomaly => "telemetry_anomaly",
            EvidenceKind::Other(k) => k.as_str(),
        };

        self.profiles.get(key).unwrap_or(&self.default_profile)
    }
}

/// The Bayesian hypothesis fusion engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FusionEngine {
    pub priors: FusionPriors,
    pub likelihoods: LikelihoodRegistry,
}

impl Default for FusionEngine {
    fn default() -> Self {
        Self {
            priors: FusionPriors::default(),
            likelihoods: LikelihoodRegistry::default(),
        }
    }
}

impl FusionEngine {
    pub fn new(priors: FusionPriors, likelihoods: LikelihoodRegistry) -> Self {
        Self {
            priors,
            likelihoods,
        }
    }

    /// Computes group weights w_k = 1 / |g| for correlated evidence.
    pub fn compute_correlation_weights(evidence_list: &[Evidence]) -> Vec<f64> {
        let mut group_counts: HashMap<String, usize> = HashMap::new();
        for ev in evidence_list {
            if let Some(grp) = &ev.group {
                *group_counts.entry(grp.clone()).or_insert(0) += 1;
            }
        }

        evidence_list
            .iter()
            .map(|ev| {
                if let Some(grp) = &ev.group {
                    let count = *group_counts.get(grp).unwrap_or(&1);
                    if count > 0 {
                        1.0 / (count as f64)
                    } else {
                        1.0
                    }
                } else {
                    1.0
                }
            })
            .collect()
    }

    /// Fuses a collection of evidence for a specific entity in log-space:
    /// log P(H | E) = log P(H) + sum_k w_k * log P(E_k | H) - log Z
    pub fn fuse_evidence(&self, evidence_list: &[Evidence]) -> Posterior {
        if evidence_list.is_empty() {
            return Posterior {
                normal: self.priors.normal,
                compromise: self.priors.compromise,
                sensor_failure: self.priors.sensor_failure,
                misconfig: self.priors.misconfig,
                environment: self.priors.environment,
            };
        }

        let weights = Self::compute_correlation_weights(evidence_list);
        let mut log_posteriors = [0.0f64; 5];

        for (idx, &h) in Hypothesis::ALL.iter().enumerate() {
            let prior = self.priors.get(h).max(1e-12);
            let mut log_p = prior.ln();

            for (k, ev) in evidence_list.iter().enumerate() {
                let w = weights[k];
                let profile = self.likelihoods.get_profile(&ev.kind);
                let lik = profile.get(h).clamp(1e-6, 1.0);
                log_p += w * lik.ln();
            }
            log_posteriors[idx] = log_p;
        }

        // Log-sum-exp normalization
        let max_log = log_posteriors
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let mut sum_exp = 0.0;
        for &lp in &log_posteriors {
            sum_exp += (lp - max_log).exp();
        }
        let log_z = max_log + sum_exp.ln();

        let p_norm = (log_posteriors[0] - log_z).exp();
        let p_comp = (log_posteriors[1] - log_z).exp();
        let p_sens = (log_posteriors[2] - log_z).exp();
        let p_misc = (log_posteriors[3] - log_z).exp();
        let p_envi = (log_posteriors[4] - log_z).exp();

        Posterior {
            normal: p_norm,
            compromise: p_comp,
            sensor_failure: p_sens,
            misconfig: p_misc,
            environment: p_envi,
        }
    }

    /// Fuses evidence across all entities in a tick.
    pub fn fuse_all_by_entity(
        &self,
        evidence_list: &[Evidence],
    ) -> HashMap<EntityId, Posterior> {
        let mut by_entity: HashMap<EntityId, Vec<Evidence>> = HashMap::new();
        for ev in evidence_list {
            by_entity
                .entry(ev.entity.clone())
                .or_default()
                .push(ev.clone());
        }

        let mut results = HashMap::new();
        for (entity, evs) in by_entity {
            results.insert(entity, self.fuse_evidence(&evs));
        }
        results
    }
}

// ============================================================================
// Robust Numerical Aggregators (§6.5)
// ============================================================================

/// Computes the sample median.
pub fn robust_median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let len = sorted.len();
    if len % 2 == 1 {
        sorted[len / 2]
    } else {
        (sorted[len / 2 - 1] + sorted[len / 2]) / 2.0
    }
}

/// Computes the trimmed mean by dropping `trim_count` samples from both extremes.
pub fn robust_trimmed_mean(values: &[f64], trim_count: usize) -> f64 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    if 2 * trim_count >= n {
        return robust_median(values);
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let slice = &sorted[trim_count..(n - trim_count)];
    let sum: f64 = slice.iter().sum();
    sum / (slice.len() as f64)
}

// ============================================================================
// Unit Tests (validating §6.5 worked examples)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ev(id: &str, kind: EvidenceKind, group: Option<&str>) -> Evidence {
        Evidence {
            id: id.to_string(),
            entity: "node-b".to_string(),
            kind,
            score: 0.90,
            event: "ev-0".to_string(),
            group: group.map(|s| s.to_string()),
            timestamp_ms: 1000,
        }
    }

    #[test]
    fn test_worked_example_step1_sensor_disagreement_only() {
        // Table in §6.5: E4 sensor disagreement only
        // Expected: normal ~0.629, compromise ~0.079, sensor fail ~0.238, misconfig ~0.020, environment ~0.033
        let engine = FusionEngine::default();
        let evs = vec![make_ev("E4", EvidenceKind::SensorDisagreement, None)];

        let post = engine.fuse_evidence(&evs);
        assert!((post.normal - 0.629).abs() < 0.015, "normal: {}", post.normal);
        assert!((post.compromise - 0.079).abs() < 0.015, "compromise: {}", post.compromise);
        assert!((post.sensor_failure - 0.238).abs() < 0.015, "sensor fail: {}", post.sensor_failure);
        assert!((post.misconfig - 0.020).abs() < 0.010, "misconfig: {}", post.misconfig);
        assert!((post.environment - 0.033).abs() < 0.010, "environment: {}", post.environment);
    }

    #[test]
    fn test_worked_example_step2_plus_network_anomaly() {
        // Table in §6.5: + E1 network anomaly
        // Expected: normal ~0.251, compromise ~0.538, sensor fail ~0.095, misconfig ~0.063, environment ~0.053
        let engine = FusionEngine::default();
        let evs = vec![
            make_ev("E4", EvidenceKind::SensorDisagreement, None),
            make_ev("E1", EvidenceKind::NetworkAnomaly, None),
        ];

        let post = engine.fuse_evidence(&evs);
        assert!((post.normal - 0.251).abs() < 0.015, "normal: {}", post.normal);
        assert!((post.compromise - 0.538).abs() < 0.015, "compromise: {}", post.compromise);
        assert!((post.sensor_failure - 0.095).abs() < 0.015, "sensor fail: {}", post.sensor_failure);
        assert!((post.misconfig - 0.063).abs() < 0.015, "misconfig: {}", post.misconfig);
        assert!((post.environment - 0.053).abs() < 0.015, "environment: {}", post.environment);
    }

    #[test]
    fn test_worked_example_step3_plus_auth_failures() {
        // Table in §6.5: + E2 auth failures
        // Expected: normal ~0.012, compromise ~0.935, sensor fail ~0.002, misconfig ~0.047, environment ~0.003
        let engine = FusionEngine::default();
        let evs = vec![
            make_ev("E4", EvidenceKind::SensorDisagreement, None),
            make_ev("E1", EvidenceKind::NetworkAnomaly, None),
            make_ev("E2", EvidenceKind::AuthFailure, None),
        ];

        let post = engine.fuse_evidence(&evs);
        assert!((post.normal - 0.012).abs() < 0.015, "normal: {}", post.normal);
        assert!((post.compromise - 0.935).abs() < 0.015, "compromise: {}", post.compromise);
        assert!((post.sensor_failure - 0.002).abs() < 0.010, "sensor fail: {}", post.sensor_failure);
        assert!((post.misconfig - 0.047).abs() < 0.015, "misconfig: {}", post.misconfig);
        assert!((post.environment - 0.003).abs() < 0.010, "environment: {}", post.environment);
    }

    #[test]
    fn test_worked_example_step4_correlation_discount() {
        // Table in §6.5: + E3 with E1 and E3 grouped (w = 1/2)
        // Expected: normal ~0.013, compromise ~0.947, sensor fail ~0.003, misconfig ~0.036, environment ~0.001
        let engine = FusionEngine::default();
        let evs = vec![
            make_ev("E4", EvidenceKind::SensorDisagreement, None),
            make_ev("E1", EvidenceKind::NetworkAnomaly, Some("grp-c2")),
            make_ev("E2", EvidenceKind::AuthFailure, None),
            make_ev("E3", EvidenceKind::ProcessAnomaly, Some("grp-c2")),
        ];

        let post = engine.fuse_evidence(&evs);
        assert!((post.normal - 0.013).abs() < 0.015, "normal: {}", post.normal);
        assert!((post.compromise - 0.947).abs() < 0.015, "compromise: {}", post.compromise);
        assert!((post.sensor_failure - 0.003).abs() < 0.010, "sensor fail: {}", post.sensor_failure);
        assert!((post.misconfig - 0.036).abs() < 0.015, "misconfig: {}", post.misconfig);
        assert!((post.environment - 0.001).abs() < 0.010, "environment: {}", post.environment);
    }

    #[test]
    fn test_robust_aggregation() {
        let vals = [10.0, 10.2, 10.1, 100.0, 9.9]; // 100.0 is an outlier
        let med = robust_median(&vals);
        assert!((med - 10.1).abs() < 1e-4);

        let trimmed = robust_trimmed_mean(&vals, 1); // drops 9.9 and 100.0
        assert!((trimmed - 10.1).abs() < 0.05);
    }
}
