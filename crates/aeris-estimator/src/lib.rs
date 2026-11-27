//! Continuous Kalman state estimation with trust-weighted measurement noise,
//! innovation gating, model health tracking, and discrete 6-state HMM filtering.
//!
//! Defined in docs/v0.1/04-mathematical-model.md §6.3.
//! Owner: AMIRTHA
//!
//! Implements:
//! - Linear Kalman filter on state [position, velocity]^T
//! - Trust-weighted measurement noise: R_j = sigma_j^2 / max(T_j^obs, T_min)
//! - Outlier innovation gate: epsilon_j = nu_j^T S_j^(-1) nu_j with chi^2 cutoff
//! - Model health tracking across sliding window W = 50
//! - Discrete 6-state HMM forward filter over {Normal, Degraded, Suspicious, Compromised, Isolated, Recovering}

use aeris_types::{EntityId, Posterior};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

// ============================================================================
// 1. Continuous Kalman Filter with Trust Weighting (§6.3)
// ============================================================================

/// 2D continuous state [position, velocity] along one axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KalmanState1D {
    pub p: f64,
    pub v: f64,
    // 2x2 Covariance matrix: [ [p_00, p_01], [p_10, p_11] ]
    pub cov: [[f64; 2]; 2],
}

impl Default for KalmanState1D {
    fn default() -> Self {
        Self {
            p: 0.0,
            v: 0.0,
            cov: [[10.0, 0.0], [0.0, 5.0]],
        }
    }
}

/// Kalman filter configuration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KalmanParams {
    pub dt: f64,
    pub q_noise: f64,
    pub t_min: f64,
    pub chi2_gate_99: f64,
    pub chi2_health_95: f64,
    pub health_window_w: usize,
}

impl Default for KalmanParams {
    fn default() -> Self {
        Self {
            dt: 0.1, // 10 Hz
            q_noise: 0.5,
            t_min: 0.05,
            chi2_gate_99: 6.635,   // for 1-DOF at p=0.01 (9.21 for 2-DOF)
            chi2_health_95: 3.841, // for 1-DOF at p=0.05
            health_window_w: 50,
        }
    }
}

/// Kalman tracker for an entity's continuous physical state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeKalmanFilter {
    pub entity: EntityId,
    pub state: KalmanState1D,
    pub params: KalmanParams,
    pub innovation_history: VecDeque<f64>,
}

impl NodeKalmanFilter {
    pub fn new(entity: EntityId, initial_pos: f64, initial_vel: f64, params: KalmanParams) -> Self {
        Self {
            entity,
            state: KalmanState1D {
                p: initial_pos,
                v: initial_vel,
                cov: [[9.0, 0.0], [0.0, 4.0]],
            },
            params,
            innovation_history: VecDeque::with_capacity(params.health_window_w),
        }
    }

    /// Predict step (once per tick):
    /// F = [ [1, dt], [0, 1] ]
    /// Q = q * [ [dt^3/3, dt^2/2], [dt^2/2, dt] ]
    /// x^- = F * x
    /// P^- = F * P * F^T + Q
    pub fn predict(&mut self) {
        let dt = self.params.dt;
        let q = self.params.q_noise;

        // x^- = [p + v * dt, v]
        let p_pred = self.state.p + self.state.v * dt;
        let v_pred = self.state.v;

        // Process noise Q
        let q00 = q * (dt * dt * dt) / 3.0;
        let q01 = q * (dt * dt) / 2.0;
        let q11 = q * dt;

        // F * P * F^T
        let p00 = self.state.cov[0][0];
        let p01 = self.state.cov[0][1];
        let p10 = self.state.cov[1][0];
        let p11 = self.state.cov[1][1];

        let fp00 = p00 + p10 * dt;
        let fp01 = p01 + p11 * dt;
        let fp10 = p10;
        let fp11 = p11;

        let next_p00 = fp00 + fp01 * dt + q00;
        let next_p01 = fp01 + q01;
        let next_p10 = fp10 + fp11 * dt + q01;
        let next_p11 = fp11 + q11;

        self.state.p = p_pred;
        self.state.v = v_pred;
        self.state.cov = [[next_p00, next_p01], [next_p10, next_p11]];
    }

    /// Update with sensor observation z, measurement sigma, and sensor trust T_obs:
    /// R = sigma^2 / max(T_obs, T_min)
    /// H = [1, 0]
    /// nu = z - H * x^-
    /// S = H * P^- * H^T + R = P_00 + R
    /// K = P^- * H^T * S^(-1) = [ P_00 / S, P_10 / S ]^T
    /// x = x^- + K * nu
    /// P = (I - K * H) * P^-
    pub fn update_sensor(&mut self, z: f64, sigma: f64, t_obs: f64) -> (f64, bool) {
        let effective_t = t_obs.max(self.params.t_min);
        let r = (sigma * sigma) / effective_t;

        let nu = z - self.state.p;
        let p00 = self.state.cov[0][0];
        let s = p00 + r;

        // Innovation squared normalized: epsilon = nu^2 / S
        let epsilon = (nu * nu) / s.max(1e-9);

        // Record for health monitoring
        if self.innovation_history.len() >= self.params.health_window_w {
            self.innovation_history.pop_front();
        }
        self.innovation_history.push_back(epsilon);

        // Outlier gating
        if epsilon > self.params.chi2_gate_99 {
            // Rejected outlier: do not update state
            return (nu, false);
        }

        let k0 = p00 / s;
        let k1 = self.state.cov[1][0] / s;

        self.state.p += k0 * nu;
        self.state.v += k1 * nu;

        let p01 = self.state.cov[0][1];
        let p10 = self.state.cov[1][0];
        let p11 = self.state.cov[1][1];

        let next_p00 = (1.0 - k0) * p00;
        let next_p01 = (1.0 - k0) * p01;
        let next_p10 = p10 - k1 * p00;
        let next_p11 = p11 - k1 * p01;

        self.state.cov = [[next_p00, next_p01], [next_p10, next_p11]];
        (nu, true)
    }

    /// Computes model health in [0, 1] across sliding window W:
    /// health_t = (1 / W) * sum_{tau} 1[epsilon_tau <= chi^2_{m, 0.95}]
    pub fn model_health(&self) -> f64 {
        if self.innovation_history.is_empty() {
            return 1.0;
        }
        let in_bounds = self
            .innovation_history
            .iter()
            .filter(|&&e| e <= self.params.chi2_health_95)
            .count();
        (in_bounds as f64) / (self.innovation_history.len() as f64)
    }

    /// Information form fusion for multiple simultaneous sensors (§6.3 worked example):
    /// hat{x} = (hat{x}^- / P^- + sum_j z_j / R_j) / (1 / P^- + sum_j 1 / R_j)
    pub fn fuse_simultaneous_sensors_1d(
        x_prior: f64,
        p_prior: f64,
        sensors: &[(f64, f64, f64)], // (measurement, variance, trust)
        t_min: f64,
    ) -> f64 {
        let mut info_sum = x_prior / p_prior;
        let mut denom_sum = 1.0 / p_prior;

        for &(z, var, t) in sensors {
            let r = var / t.max(t_min);
            info_sum += z / r;
            denom_sum += 1.0 / r;
        }

        info_sum / denom_sum
    }
}

// ============================================================================
// 2. Discrete-State 6-State HMM Forward Filter (§6.3)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiscreteNodeState {
    Normal = 0,
    Degraded = 1,
    Suspicious = 2,
    Compromised = 3,
    Isolated = 4,
    Recovering = 5,
}

impl DiscreteNodeState {
    pub const ALL: [DiscreteNodeState; 6] = [
        DiscreteNodeState::Normal,
        DiscreteNodeState::Degraded,
        DiscreteNodeState::Suspicious,
        DiscreteNodeState::Compromised,
        DiscreteNodeState::Isolated,
        DiscreteNodeState::Recovering,
    ];
}

/// 6-state discrete forward filter per node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHmmFilter {
    pub entity: EntityId,
    pub belief: [f64; 6],
    pub transition_matrix: [[f64; 6]; 6],
}

impl NodeHmmFilter {
    pub fn new(entity: EntityId) -> Self {
        // Sticky transition matrix with ~0.97 on diagonal for active states
        let mut t = [[0.006; 6]; 6];
        for i in 0..6 {
            t[i][i] = 0.97;
        }

        // ISOLATED and RECOVERING are entered only by executed actions
        // Prevent random transition into ISOLATED/RECOVERING
        for i in 0..4 {
            t[i][4] = 0.0;
            t[i][5] = 0.0;
            // Normalize rows
            let sum: f64 = t[i].iter().sum();
            for j in 0..6 {
                t[i][j] /= sum;
            }
        }

        let mut init_b = [0.0; 6];
        init_b[DiscreteNodeState::Normal as usize] = 1.0;

        Self {
            entity,
            belief: init_b,
            transition_matrix: t,
        }
    }

    /// Predict and update HMM belief:
    /// b_t(q) proportional to P(o_t | q) * sum_{q'} P(q | q') * b_{t-1}(q')
    pub fn update(&mut self, posterior: &Posterior) {
        // Map evidence fusion posterior to emission likelihoods P(o_t | q)
        let mut obs_lik = [0.01; 6];
        obs_lik[DiscreteNodeState::Normal as usize] = posterior.normal.max(1e-4);
        obs_lik[DiscreteNodeState::Degraded as usize] =
            (posterior.sensor_failure + posterior.misconfig).max(1e-4);
        obs_lik[DiscreteNodeState::Suspicious as usize] =
            (posterior.compromise * 0.5 + posterior.sensor_failure * 0.5).max(1e-4);
        obs_lik[DiscreteNodeState::Compromised as usize] = posterior.compromise.max(1e-4);
        obs_lik[DiscreteNodeState::Isolated as usize] = 0.001;
        obs_lik[DiscreteNodeState::Recovering as usize] = 0.001;

        let mut next_b = [0.0f64; 6];
        for q in 0..6 {
            let mut pred_sum = 0.0;
            for q_prev in 0..6 {
                pred_sum += self.transition_matrix[q_prev][q] * self.belief[q_prev];
            }
            next_b[q] = obs_lik[q] * pred_sum;
        }

        // Normalize
        let sum: f64 = next_b.iter().sum();
        if sum > 1e-9 {
            for b in &mut next_b {
                *b /= sum;
            }
        } else {
            next_b = self.belief;
        }

        self.belief = next_b;
    }

    /// Force state transition on executed action (e.g. Isolate, Recover)
    pub fn force_state(&mut self, state: DiscreteNodeState) {
        self.belief = [0.0; 6];
        self.belief[state as usize] = 1.0;
    }

    pub fn most_likely_state(&self) -> DiscreteNodeState {
        let mut max_p = -1.0;
        let mut best = DiscreteNodeState::Normal;
        for (i, &p) in self.belief.iter().enumerate() {
            if p > max_p {
                max_p = p;
                best = DiscreteNodeState::ALL[i];
            }
        }
        best
    }
}

// ============================================================================
// 3. Multi-Node Estimator Pipeline
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmEstimator {
    pub kalman_filters: HashMap<EntityId, NodeKalmanFilter>,
    pub hmm_filters: HashMap<EntityId, NodeHmmFilter>,
}

impl Default for SwarmEstimator {
    fn default() -> Self {
        let mut kalman = HashMap::new();
        let mut hmm = HashMap::new();

        for id in &["node-a", "node-b", "node-c"] {
            kalman.insert(
                id.to_string(),
                NodeKalmanFilter::new(id.to_string(), 0.0, 0.0, KalmanParams::default()),
            );
            hmm.insert(id.to_string(), NodeHmmFilter::new(id.to_string()));
        }

        Self {
            kalman_filters: kalman,
            hmm_filters: hmm,
        }
    }
}

impl SwarmEstimator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn predict_all(&mut self) {
        for kf in self.kalman_filters.values_mut() {
            kf.predict();
        }
    }

    pub fn overall_model_health(&self) -> f64 {
        if self.kalman_filters.is_empty() {
            return 1.0;
        }
        let sum: f64 = self.kalman_filters.values().map(|kf| kf.model_health()).sum();
        sum / (self.kalman_filters.len() as f64)
    }
}

// ============================================================================
// Unit Tests (verifying §6.3 worked examples)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worked_example_spoofed_gps_loses_vote() {
        // §6.3 Worked example:
        // IMU prediction: x^- = 100.0 m with variance P^- = 9
        // GPS says 120.0 m (sigma^2 = 9)
        // Camera says 101.5 m (sigma^2 = 16)
        //
        // 1) With equal trust (T = 1): x ≈ 108.13 m
        let x_equal = NodeKalmanFilter::fuse_simultaneous_sensors_1d(
            100.0,
            9.0,
            &[(120.0, 9.0, 1.0), (101.5, 16.0, 1.0)],
            0.05,
        );
        assert!(
            (x_equal - 108.13).abs() < 0.05,
            "Expected x ≈ 108.13, got {}",
            x_equal
        );

        // 2) With T_gps = 0.21 and T_cam = 0.89: x ≈ 102.89 m
        let x_weighted = NodeKalmanFilter::fuse_simultaneous_sensors_1d(
            100.0,
            9.0,
            &[(120.0, 9.0, 0.21), (101.5, 16.0, 0.89)],
            0.05,
        );
        assert!(
            (x_weighted - 102.89).abs() < 0.05,
            "Expected x ≈ 102.89, got {}",
            x_weighted
        );
    }

    #[test]
    fn test_kalman_outlier_gating() {
        let mut kf = NodeKalmanFilter::new("node-b".into(), 100.0, 0.0, KalmanParams::default());
        // Normal update
        let (_nu, accepted) = kf.update_sensor(101.0, 3.0, 1.0);
        assert!(accepted);

        // Wild outlier (e.g. 500 m jump)
        let (_nu2, accepted2) = kf.update_sensor(500.0, 3.0, 1.0);
        assert!(!accepted2, "Expected wild outlier to be rejected by gate");
    }

    #[test]
    fn test_hmm_filter_transitions() {
        let mut hmm = NodeHmmFilter::new("node-b".into());
        assert_eq!(hmm.most_likely_state(), DiscreteNodeState::Normal);

        // Feed compromise posterior
        let comp_post = Posterior {
            normal: 0.013,
            compromise: 0.947,
            sensor_failure: 0.003,
            misconfig: 0.036,
            environment: 0.001,
        };

        for _ in 0..5 {
            hmm.update(&comp_post);
        }

        assert_eq!(hmm.most_likely_state(), DiscreteNodeState::Compromised);
    }
}
