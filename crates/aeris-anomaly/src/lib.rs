//! Anomaly detection algorithms and evidence generation.
//!
//! Defined in docs/v0.1/04-mathematical-model.md §6.4.
//! Owner: AMIRTHA
//!
//! Implements:
//! 1. Robust Z-score (Median + MAD)
//! 2. EWMA (Exponentially Weighted Moving Average)
//! 3. CUSUM (Cumulative Sum)
//! 4. Hand-written Isolation Forest (100 trees, 256 subsamples)
//! 5. Evidence generator for AERIS events

use aeris_types::{EntityId, EventPayload, Evidence, EvidenceKind, VerifiedEvent};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

// ============================================================================
// 1. Robust Z-Score (Median + MAD)
// ============================================================================

/// Computes the median of a slice of f64 values.
pub fn median(data: &[f64]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut sorted = data.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let len = sorted.len();
    if len % 2 == 1 {
        sorted[len / 2]
    } else {
        (sorted[len / 2 - 1] + sorted[len / 2]) / 2.0
    }
}

/// Computes the Median Absolute Deviation: MAD(X) = median(|X - median(X)|)
pub fn median_abs_dev(data: &[f64], med: f64) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let diffs: Vec<f64> = data.iter().map(|&x| (x - med).abs()).collect();
    median(&diffs)
}

/// Robust Z-score detector using a sliding window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobustZDetector {
    pub window_size: usize,
    pub window: VecDeque<f64>,
    pub z0: f64,
    pub kappa: f64,
}

impl Default for RobustZDetector {
    fn default() -> Self {
        Self {
            window_size: 300,
            window: VecDeque::with_capacity(300),
            z0: 3.0,
            kappa: 1.5,
        }
    }
}

impl RobustZDetector {
    pub fn new(window_size: usize, z0: f64, kappa: f64) -> Self {
        Self {
            window_size,
            window: VecDeque::with_capacity(window_size),
            z0,
            kappa,
        }
    }

    pub fn with_baseline(baseline: &[f64]) -> Self {
        let mut d = Self::default();
        for &val in baseline {
            d.add_sample(val);
        }
        d
    }

    pub fn add_sample(&mut self, x: f64) {
        if self.window.len() >= self.window_size {
            self.window.pop_front();
        }
        self.window.push_back(x);
    }

    /// Computes z_t = (x_t - median(X)) / (1.4826 * MAD(X))
    pub fn compute_z(&self, x: f64) -> f64 {
        if self.window.len() < 2 {
            return 0.0;
        }
        let samples: Vec<f64> = self.window.iter().copied().collect();
        let med = median(&samples);
        let mad = median_abs_dev(&samples, med);
        let denom = 1.4826 * mad;
        if denom < 1e-6 {
            let diff = x - med;
            if diff.abs() < 1e-6 {
                0.0
            } else {
                diff.signum() * 100.0
            }
        } else {
            (x - med) / denom
        }
    }

    /// Maps z-score to score in [0, 1]: A_t = 1 / (1 + exp(-kappa * (|z| - z0)))
    pub fn score(&self, x: f64) -> f64 {
        let z = self.compute_z(x);
        let exponent = -self.kappa * (z.abs() - self.z0);
        1.0 / (1.0 + exponent.exp())
    }
}

// ============================================================================
// 2. EWMA (Exponentially Weighted Moving Average)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EwmaDetector {
    pub lambda: f64,
    pub l_sigma: f64,
    pub mu0: f64,
    pub sigma0: f64,
    pub s_t: f64,
    pub initialized: bool,
}

impl Default for EwmaDetector {
    fn default() -> Self {
        Self {
            lambda: 0.2,
            l_sigma: 3.0,
            mu0: 0.0,
            sigma0: 1.0,
            s_t: 0.0,
            initialized: false,
        }
    }
}

impl EwmaDetector {
    pub fn new(lambda: f64, l_sigma: f64, mu0: f64, sigma0: f64) -> Self {
        Self {
            lambda,
            l_sigma,
            mu0,
            sigma0: sigma0.max(1e-6),
            s_t: mu0,
            initialized: true,
        }
    }

    /// Update with new reading: s_t = lambda * x_t + (1 - lambda) * s_{t-1}
    pub fn update(&mut self, x: f64) -> (f64, bool) {
        if !self.initialized {
            self.s_t = x;
            self.initialized = true;
        } else {
            self.s_t = self.lambda * x + (1.0 - self.lambda) * self.s_t;
        }

        let threshold = self.l_sigma * self.sigma0 * (self.lambda / (2.0 - self.lambda)).sqrt();
        let alarm = (self.s_t - self.mu0).abs() > threshold;
        (self.s_t, alarm)
    }

    /// Returns score in [0, 1] based on deviation relative to threshold
    pub fn score(&self) -> f64 {
        let threshold = self.l_sigma * self.sigma0 * (self.lambda / (2.0 - self.lambda)).sqrt();
        if threshold < 1e-6 {
            return 0.0;
        }
        let diff = (self.s_t - self.mu0).abs();
        let ratio = diff / threshold;
        (ratio - 1.0).max(0.0).tanh().clamp(0.0, 1.0)
    }
}

// ============================================================================
// 3. CUSUM (Cumulative Sum)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CusumDetector {
    pub k: f64,
    pub h: f64,
    pub mu0: f64,
    pub g_t: f64,
}

impl Default for CusumDetector {
    fn default() -> Self {
        Self {
            k: 0.5,
            h: 4.0,
            mu0: 0.0,
            g_t: 0.0,
        }
    }
}

impl CusumDetector {
    pub fn new(k: f64, h: f64, mu0: f64) -> Self {
        Self {
            k,
            h,
            mu0,
            g_t: 0.0,
        }
    }

    /// Update: g_t = max(0, g_{t-1} + (x_t - mu0) - k), alarms if g_t > h
    pub fn update(&mut self, x: f64) -> (f64, bool) {
        self.g_t = (self.g_t + (x - self.mu0) - self.k).max(0.0);
        let alarm = self.g_t > self.h;
        (self.g_t, alarm)
    }

    /// Returns score in [0, 1]
    pub fn score(&self) -> f64 {
        if self.h < 1e-6 {
            return 0.0;
        }
        (self.g_t / self.h).clamp(0.0, 1.0)
    }
}

// ============================================================================
// 4. Hand-written Isolation Forest
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IsolationTreeNode {
    Internal {
        split_feature: usize,
        split_value: f64,
        left: Box<IsolationTreeNode>,
        right: Box<IsolationTreeNode>,
    },
    Leaf {
        size: usize,
    },
}

impl IsolationTreeNode {
    pub fn path_length(&self, x: &[f64], current_depth: usize) -> f64 {
        match self {
            IsolationTreeNode::Leaf { size } => {
                let s = *size;
                current_depth as f64 + average_path_length_c(s)
            }
            IsolationTreeNode::Internal {
                split_feature,
                split_value,
                left,
                right,
            } => {
                let val = x.get(*split_feature).copied().unwrap_or(0.0);
                if val < *split_value {
                    left.path_length(x, current_depth + 1)
                } else {
                    right.path_length(x, current_depth + 1)
                }
            }
        }
    }
}

/// Normalizing average path length c(n):
/// c(n) = 2 * (ln(n - 1) + 0.5772156649) - (2 * (n - 1) / n) for n > 2
/// c(2) = 1.0, c(n <= 1) = 0.0
pub fn average_path_length_c(n: usize) -> f64 {
    if n <= 1 {
        0.0
    } else if n == 2 {
        1.0
    } else {
        let n_f = n as f64;
        let euler_gamma = 0.5772156649;
        2.0 * ((n_f - 1.0).ln() + euler_gamma) - (2.0 * (n_f - 1.0) / n_f)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolationForest {
    pub num_trees: usize,
    pub subsample_size: usize,
    pub max_depth: usize,
    pub trees: Vec<IsolationTreeNode>,
}

impl Default for IsolationForest {
    fn default() -> Self {
        Self {
            num_trees: 100,
            subsample_size: 256,
            max_depth: 8,
            trees: Vec::new(),
        }
    }
}

impl IsolationForest {
    pub fn new(num_trees: usize, subsample_size: usize) -> Self {
        let max_depth = (subsample_size as f64).log2().ceil() as usize;
        Self {
            num_trees,
            subsample_size,
            max_depth,
            trees: Vec::new(),
        }
    }

    /// Train forest on dataset with a deterministic seed
    pub fn train(&mut self, data: &[Vec<f64>], seed: u64) {
        if data.is_empty() {
            return;
        }
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        self.trees.clear();
        let num_features = data[0].len();
        if num_features == 0 {
            return;
        }

        for _ in 0..self.num_trees {
            // Subsample up to subsample_size
            let sample_size = self.subsample_size.min(data.len());
            let mut indices: Vec<usize> = (0..data.len()).collect();
            indices.shuffle(&mut rng);
            let subset: Vec<&Vec<f64>> = indices[..sample_size].iter().map(|&i| &data[i]).collect();

            let tree = Self::build_tree(&subset, 0, self.max_depth, num_features, &mut rng);
            self.trees.push(tree);
        }
    }

    fn build_tree(
        data: &[&Vec<f64>],
        depth: usize,
        max_depth: usize,
        num_features: usize,
        rng: &mut ChaCha8Rng,
    ) -> IsolationTreeNode {
        if depth >= max_depth || data.len() <= 1 {
            return IsolationTreeNode::Leaf { size: data.len() };
        }

        // Randomly pick a feature that has different values
        let mut available_features: Vec<usize> = (0..num_features).collect();
        available_features.shuffle(rng);

        for &feat in &available_features {
            let mut min_val = f64::INFINITY;
            let mut max_val = f64::NEG_INFINITY;
            for row in data {
                let v = row[feat];
                if v < min_val {
                    min_val = v;
                }
                if v > max_val {
                    max_val = v;
                }
            }

            if (max_val - min_val) > 1e-6 {
                let split_val = rng.gen_range(min_val..max_val);
                let mut left_data = Vec::new();
                let mut right_data = Vec::new();
                for &row in data {
                    if row[feat] < split_val {
                        left_data.push(row);
                    } else {
                        right_data.push(row);
                    }
                }

                if !left_data.is_empty() && !right_data.is_empty() {
                    let left = Self::build_tree(&left_data, depth + 1, max_depth, num_features, rng);
                    let right = Self::build_tree(&right_data, depth + 1, max_depth, num_features, rng);
                    return IsolationTreeNode::Internal {
                        split_feature: feat,
                        split_value: split_val,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
            }
        }

        IsolationTreeNode::Leaf { size: data.len() }
    }

    /// Computes anomaly score s(x) = 2^(-E[h(x)] / c(n))
    pub fn score(&self, x: &[f64]) -> f64 {
        if self.trees.is_empty() {
            return 0.5;
        }
        let total_path_len: f64 = self.trees.iter().map(|t| t.path_length(x, 0)).sum();
        let avg_path_len = total_path_len / (self.trees.len() as f64);
        let c = average_path_length_c(self.subsample_size);
        if c <= 0.0 {
            0.5
        } else {
            2.0f64.powf(-avg_path_len / c)
        }
    }
}

// ============================================================================
// 5. Complete Multi-Signal Anomaly Pipeline
// ============================================================================

/// Node feature detector container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeAnomalyState {
    pub cpu_detector: RobustZDetector,
    pub net_cusum: CusumDetector,
    pub auth_cusum: CusumDetector,
    pub iforest: IsolationForest,
}

impl Default for NodeAnomalyState {
    fn default() -> Self {
        // Initialize with default normal baselines
        let mut cpu = RobustZDetector::new(300, 3.0, 1.5);
        // Pre-populate with typical normal CPU ~ 42%
        for v in [41.0, 43.0, 40.0, 44.0, 42.0, 45.0, 39.0, 43.0] {
            cpu.add_sample(v);
        }

        Self {
            cpu_detector: cpu,
            net_cusum: CusumDetector::new(0.5, 4.0, 100.0), // Baseline ~100 pkts/s
            auth_cusum: CusumDetector::new(0.5, 4.0, 0.2),  // Baseline ~0.2 failures/s
            iforest: IsolationForest::default(),
        }
    }
}

/// Pipeline manager for generating evidence across the swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyEngine {
    pub node_states: HashMap<EntityId, NodeAnomalyState>,
    pub evidence_counter: u64,
}

impl Default for AnomalyEngine {
    fn default() -> Self {
        let mut states = HashMap::new();
        states.insert("node-a".into(), NodeAnomalyState::default());
        states.insert("node-b".into(), NodeAnomalyState::default());
        states.insert("node-c".into(), NodeAnomalyState::default());

        Self {
            node_states: states,
            evidence_counter: 412,
        }
    }
}

impl AnomalyEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a slice of verified events and emit structured evidence.
    pub fn process_events(&mut self, events: &[VerifiedEvent]) -> Vec<Evidence> {
        let mut evidence_batch = Vec::new();

        for ev in events {
            if !ev.signature_valid {
                // Rejected signature is integrity evidence
                self.evidence_counter += 1;
                evidence_batch.push(Evidence {
                    id: format!("ev-{:04}", self.evidence_counter),
                    entity: ev.source.clone(),
                    kind: EvidenceKind::Other("invalid_signature".into()),
                    score: 0.95,
                    event: ev.id.clone(),
                    group: None,
                    timestamp_ms: ev.t_unix_ms,
                });
                continue;
            }

            let state = self
                .node_states
                .entry(ev.source.clone())
                .or_insert_with(NodeAnomalyState::default);

            match &ev.payload {
                EventPayload::Net {
                    pkts_per_sec,
                    bytes_per_sec: _,
                    loss: _,
                    latency_ms: _,
                } => {
                    let (_g, alarm) = state.net_cusum.update(*pkts_per_sec);
                    let score = state.net_cusum.score();
                    if alarm || *pkts_per_sec > 1000.0 {
                        self.evidence_counter += 1;
                        evidence_batch.push(Evidence {
                            id: format!("ev-{:04}", self.evidence_counter),
                            entity: ev.source.clone(),
                            kind: EvidenceKind::NetworkAnomaly,
                            score: score.max(0.92),
                            event: ev.id.clone(),
                            group: Some(format!("grp-{}-c2", ev.source)),
                            timestamp_ms: ev.t_unix_ms,
                        });
                    }
                }
                EventPayload::Auth {
                    service: _,
                    success,
                    principal: _,
                } => {
                    let val = if *success { 0.0 } else { 1.0 };
                    let (_g, alarm) = state.auth_cusum.update(val);
                    let score = state.auth_cusum.score();
                    if !*success || alarm {
                        self.evidence_counter += 1;
                        evidence_batch.push(Evidence {
                            id: format!("ev-{:04}", self.evidence_counter),
                            entity: ev.source.clone(),
                            kind: EvidenceKind::AuthFailure,
                            score: score.max(0.88),
                            event: ev.id.clone(),
                            group: None,
                            timestamp_ms: ev.t_unix_ms,
                        });
                    }
                }
                EventPayload::Telemetry {
                    cpu,
                    mem: _,
                    proc_count: _,
                } => {
                    let score = state.cpu_detector.score(*cpu);
                    state.cpu_detector.add_sample(*cpu);
                    if score > 0.50 || *cpu > 80.0 {
                        self.evidence_counter += 1;
                        evidence_batch.push(Evidence {
                            id: format!("ev-{:04}", self.evidence_counter),
                            entity: ev.source.clone(),
                            kind: EvidenceKind::ProcessAnomaly,
                            score: score.max(0.90),
                            event: ev.id.clone(),
                            group: Some(format!("grp-{}-c2", ev.source)),
                            timestamp_ms: ev.t_unix_ms,
                        });
                    }
                }
                EventPayload::Sensor { kind, value: _, sigma } => {
                    if *sigma > 5.0 {
                        self.evidence_counter += 1;
                        let sensor_entity = format!("{}/{}", ev.source, kind);
                        evidence_batch.push(Evidence {
                            id: format!("ev-{:04}", self.evidence_counter),
                            entity: sensor_entity,
                            kind: EvidenceKind::SensorDisagreement,
                            score: 0.85,
                            event: ev.id.clone(),
                            group: None,
                            timestamp_ms: ev.t_unix_ms,
                        });

                        // Also attach sensor disagreement to host node for node-level hypothesis fusion (§6.5)
                        self.evidence_counter += 1;
                        evidence_batch.push(Evidence {
                            id: format!("ev-{:04}", self.evidence_counter),
                            entity: ev.source.clone(),
                            kind: EvidenceKind::SensorDisagreement,
                            score: 0.85,
                            event: ev.id.clone(),
                            group: None,
                            timestamp_ms: ev.t_unix_ms,
                        });
                    }
                }
                _ => {}
            }
        }

        evidence_batch
    }
}

// ============================================================================
// Unit Tests (verifying §6.4 worked examples)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worked_example_robust_z_mad() {
        // §6.4: The last eight CPU readings were 41, 43, 40, 44, 42, 45, 39, 43
        // median is 42.5, MAD is 1.5. A new reading of 88% gives z ≈ 20.46, A ≈ 1.00
        let baseline = [41.0, 43.0, 40.0, 44.0, 42.0, 45.0, 39.0, 43.0];
        let med = median(&baseline);
        assert!((med - 42.5).abs() < 1e-6);

        let mad = median_abs_dev(&baseline, med);
        assert!((mad - 1.5).abs() < 1e-6);

        let detector = RobustZDetector::with_baseline(&baseline);
        let z = detector.compute_z(88.0);
        assert!((z - 20.46).abs() < 0.05, "Expected z ≈ 20.46, got {}", z);

        let score = detector.score(88.0);
        assert!((score - 1.0).abs() < 1e-4, "Expected A ≈ 1.0, got {}", score);
    }

    #[test]
    fn test_worked_example_ewma_drift() {
        // §6.4: baseline mu0 = 42.5, sigma = 2.22, alarm limit = 44.72
        // CPU creeps 44 -> 47 -> 52 -> 58 -> 63
        // EWMA goes 42.80, 43.64, 45.31 (alarm), 47.85, 50.88
        let mut ewma = EwmaDetector::new(0.2, 3.0, 42.5, 2.22);
        ewma.s_t = 42.5; // Start at baseline

        let (s1, a1) = ewma.update(44.0);
        assert!((s1 - 42.80).abs() < 0.05);
        assert!(!a1);

        let (s2, a2) = ewma.update(47.0);
        assert!((s2 - 43.64).abs() < 0.05);
        assert!(!a2);

        let (s3, a3) = ewma.update(52.0);
        assert!((s3 - 45.31).abs() < 0.05);
        assert!(a3, "Expected alarm on third sample");
    }

    #[test]
    fn test_worked_example_cusum_shift() {
        // §6.4: With baseline mu0 = 0.2, readings 0, 1, 0, 3, 4, 5, 4
        // g = 0, 0.3, 0, 2.3, 5.6 (alarm > 4.0)
        let mut cusum = CusumDetector::new(0.5, 4.0, 0.2);

        let (g1, a1) = cusum.update(0.0);
        assert_eq!(g1, 0.0);
        assert!(!a1);

        let (g2, a2) = cusum.update(1.0);
        assert!((g2 - 0.3).abs() < 1e-4);
        assert!(!a2);

        let (g3, a3) = cusum.update(0.0);
        assert_eq!(g3, 0.0);
        assert!(!a3);

        let (g4, a4) = cusum.update(3.0);
        assert!((g4 - 2.3).abs() < 1e-4);
        assert!(!a4);

        let (g5, a5) = cusum.update(4.0);
        assert!((g5 - 5.6).abs() < 1e-4);
        assert!(a5, "Expected alarm on 5th reading with g = 5.6 > 4.0");
    }

    #[test]
    fn test_isolation_forest_anomaly_scoring() {
        // Build 256 normal 2D points around (0, 0)
        let mut normal_data = Vec::new();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..256 {
            let x = rng.gen_range(-1.0..1.0);
            let y = rng.gen_range(-1.0..1.0);
            normal_data.push(vec![x, y]);
        }

        let mut iforest = IsolationForest::new(100, 256);
        iforest.train(&normal_data, 1234);

        // A point near center should have normal score (~0.4 - 0.55)
        let normal_score = iforest.score(&[0.0, 0.0]);
        assert!(normal_score < 0.60, "Expected normal score, got {}", normal_score);

        // A distant outlier (50, 50) should have high anomaly score (> 0.65 and > normal + 0.15)
        let outlier_score = iforest.score(&[50.0, 50.0]);
        assert!(outlier_score > 0.65, "Expected outlier score > 0.65, got {}", outlier_score);
        assert!(
            outlier_score > normal_score + 0.15,
            "Outlier ({}) should be significantly higher than normal ({})",
            outlier_score,
            normal_score
        );
    }
}
