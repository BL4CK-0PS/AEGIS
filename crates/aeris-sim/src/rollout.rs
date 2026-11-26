//! Counterfactual Monte Carlo rollouts (§6.10).
//!
//! ```text
//! for each candidate a, for n = 1..N:
//!     s⁽ⁿ⁾ ~ b_t                      sample hidden state from belief
//!     W ← fork(base, rng(seed, n))     copy the world, independent streams
//!     apply Γ(a)                       same commands execution would send
//!     step K ticks: S ← F(S, a, W⁽ⁿ⁾)
//!     success⁽ⁿ⁾ = every capability ≥ its minimum on every tick
//! ```
//!
//! **Common random numbers.** Rollout `n` uses the stream `(seed, "rollout",
//! n)` for *every* candidate action, and each tick draws a fixed number of
//! values, so candidates are compared under identical hidden states and
//! disturbances. Differences in outcome come from the action alone.
//!
//! **Isolation.** The base world is only read (`&World`); each rollout works
//! on its own fork. Nothing from a rollout can reach the live world or
//! another candidate's rollouts.
//!
//! **No leaked truth.** The base should come from
//! [`World::from_observation`]; hidden state is sampled from the
//! [`Belief`] using the `[threat_model]` in `world.toml`, never from the
//! scenario's ground-truth attacker.

use crate::rng::{derive_seed, unit};
use crate::scenario::AttackerProfile;
use crate::world::{Compromise, World};
use crate::{Capabilities, SimError};
use aeris_types::Capability;
use aeris_wire::{CommandOp, NodeMode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Belief about one node's hidden state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeBelief {
    /// Node id.
    pub node: String,
    /// P(node is compromised).
    pub p_compromised: f64,
    /// P(node's GPS has failed).
    pub p_gps_failed: f64,
}

/// `b_t` restricted to what rollouts need. Nodes not listed are believed
/// healthy.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Belief {
    /// Per-node beliefs.
    pub nodes: Vec<NodeBelief>,
}

/// Rollout parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolloutSpec {
    /// Rollouts per candidate (N, 500 in v0.1).
    pub n: usize,
    /// Horizon in ticks (K, 100 in v0.1).
    pub horizon_ticks: u32,
    /// Tick at which the verification snapshot is taken (T_v; 30 = 3 s).
    pub verify_tick: u32,
    /// Base seed.
    pub seed: u64,
}

impl RolloutSpec {
    /// v0.1 defaults from `params.toml`: N = 500, K = 100, T_v = 3 s.
    pub fn v01(seed: u64) -> Self {
        Self {
            n: 500,
            horizon_ticks: 100,
            verify_tick: 30,
            seed,
        }
    }
}

/// One simulated future.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RolloutOutcome {
    /// Every capability met its minimum on every tick.
    pub success: bool,
    /// `1 − success`; the decision layer adds `d(a)`.
    pub mission_loss: f64,
    /// First failing tick (1-based) and capability.
    pub first_failure: Option<(u32, Capability)>,
    /// Minimum of each capability over the horizon.
    pub min_capabilities: Capabilities,
    /// Capabilities at `verify_tick`.
    pub verify_capabilities: Capabilities,
    /// Capabilities at the horizon.
    pub final_capabilities: Capabilities,
    /// Node modes at the horizon.
    pub final_modes: Vec<(String, NodeMode)>,
    /// Active links at the horizon.
    pub final_active_links: Vec<String>,
    /// Hidden state sampled for this rollout: compromised nodes at start.
    pub sampled_compromised: Vec<String>,
    /// Nodes compromised by the end (includes lateral spread).
    pub final_compromised: Vec<String>,
}

/// All rollouts for one candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RolloutBatch {
    /// Commands applied (Γ(a)).
    pub plan: Vec<CommandOp>,
    /// Outcomes, in rollout index order.
    pub outcomes: Vec<RolloutOutcome>,
}

impl RolloutBatch {
    /// Success flags, the input `aeris_decision::evaluate_rollout_samples` takes.
    pub fn successes(&self) -> Vec<bool> {
        self.outcomes.iter().map(|o| o.success).collect()
    }

    /// Empirical P(success).
    pub fn p_success(&self) -> f64 {
        if self.outcomes.is_empty() {
            return 0.0;
        }
        self.outcomes.iter().filter(|o| o.success).count() as f64 / self.outcomes.len() as f64
    }

    /// Per-capability median at `verify_tick`: the expected state used by
    /// execution verification (§6.13 "median rollout").
    pub fn median_verify_capabilities(&self) -> Capabilities {
        let med = |f: fn(&Capabilities) -> f64| {
            let mut v: Vec<f64> = self
                .outcomes
                .iter()
                .map(|o| f(&o.verify_capabilities))
                .collect();
            v.sort_by(f64::total_cmp);
            if v.is_empty() { 0.0 } else { v[v.len() / 2] }
        };
        Capabilities {
            navigation: med(|c| c.navigation),
            communication: med(|c| c.communication),
            observation: med(|c| c.observation),
        }
    }
}

fn check_prob(v: f64) -> Result<f64, SimError> {
    if v.is_finite() && (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(SimError::Rollout(format!(
            "belief probability {v} not in [0, 1]"
        )))
    }
}

/// Runs `spec.n` rollouts of `plan` from `base`.
///
/// Returns an error (and no outcomes) if a command in `plan` is invalid for
/// the base world, e.g. an unknown link: an action that cannot be executed
/// is infeasible, not a 0% success.
pub fn run_rollouts(
    base: &World,
    belief: &Belief,
    plan: &[CommandOp],
    spec: &RolloutSpec,
) -> Result<RolloutBatch, SimError> {
    if spec.n == 0 || spec.horizon_ticks == 0 || spec.verify_tick > spec.horizon_ticks {
        return Err(SimError::Rollout(
            "need n > 0, horizon > 0 and verify_tick <= horizon".into(),
        ));
    }
    let cfg = Arc::clone(base.config());
    let mut belief_idx = Vec::with_capacity(belief.nodes.len());
    for b in &belief.nodes {
        let i = cfg
            .node_index(&b.node)
            .ok_or_else(|| SimError::UnknownEntity(b.node.clone()))?;
        belief_idx.push((i, check_prob(b.p_compromised)?, check_prob(b.p_gps_failed)?));
    }
    let tm = &cfg.threat_model;
    let model_profile = Arc::new(AttackerProfile {
        relay_integrity: tm.relay_integrity,
        observation_integrity: tm.observation_integrity,
        lateral_hazard_per_tick: tm.lateral_hazard_per_tick,
        // Observation-side fields are irrelevant to rollouts (no events).
        cpu_extra_pct: 0.0,
        mem_extra_pct: 0.0,
        rogue_procs: 0,
        rogue_process_name: "model".into(),
        rogue_process_hash: String::new(),
        beacon_pkts_per_sec: 0.0,
        beacon_peers: 0,
        auth_fail_rate_hz: 0.0,
        auth_service: "model".into(),
        auth_principal: "model".into(),
        camera_report_offset_m: [0.0; 2],
    });
    // Validate the plan once on a scratch fork so an invalid action fails fast.
    {
        let mut probe = base.fork(derive_seed(spec.seed, "rollout-probe", 0));
        for op in plan {
            probe.apply_command(op)?;
        }
    }

    let n_nodes = cfg.nodes.len();
    let mut outcomes = Vec::with_capacity(spec.n);
    for n in 0..spec.n {
        let mut w = base.fork(derive_seed(spec.seed, "rollout", n as u64));
        // Sample hidden state: 2 uniforms per node, fixed order (CRN).
        let mut draws = vec![(1.0f64, 1.0f64); n_nodes];
        for d in draws.iter_mut() {
            *d = (unit(&mut w.rng_env), unit(&mut w.rng_env));
        }
        let mut sampled = Vec::new();
        for &(i, p_c, p_g) in &belief_idx {
            if draws[i].0 < p_c {
                w.nodes[i].compromise = Some(Compromise {
                    profile: Arc::clone(&model_profile),
                    since: w.tick,
                    announced: true,
                });
                sampled.push(cfg.nodes[i].id.clone());
            }
            if draws[i].1 < p_g {
                w.nodes[i].gps_failure_flag = true;
            }
        }
        for op in plan {
            // Validated above; the plan cannot fail differently per rollout
            // because commands do not depend on random state.
            w.apply_command(op)?;
        }
        let mut success = true;
        let mut first_failure = None;
        let mut min_caps = Capabilities {
            navigation: f64::INFINITY,
            communication: f64::INFINITY,
            observation: f64::INFINITY,
        };
        let mut verify_caps = None;
        let mut caps = w.capabilities();
        for k in 1..=spec.horizon_ticks {
            w.tick += 1;
            w.advance_truth();
            caps = w.capabilities();
            let mins = w.effective_minima();
            for c in [
                Capability::Navigation,
                Capability::Communication,
                Capability::Observation,
            ] {
                let v = caps.get(c);
                let m = min_caps.get(c).min(v);
                min_caps.set(c, m);
                if success && v < mins.get(c) {
                    success = false;
                    first_failure = Some((k, c));
                }
            }
            if k == spec.verify_tick {
                verify_caps = Some(caps);
            }
        }
        let mut final_modes: Vec<(String, NodeMode)> = cfg
            .nodes
            .iter()
            .zip(&w.nodes)
            .map(|(c, s)| (c.id.clone(), s.mode))
            .collect();
        final_modes.sort_by(|a, b| a.0.cmp(&b.0));
        let final_active_links = cfg
            .links
            .iter()
            .zip(&w.links)
            .filter(|(_, s)| s.active)
            .map(|(c, _)| c.id.clone())
            .collect();
        let final_compromised = cfg
            .nodes
            .iter()
            .zip(&w.nodes)
            .filter(|(_, s)| s.compromise.is_some())
            .map(|(c, _)| c.id.clone())
            .collect();
        outcomes.push(RolloutOutcome {
            success,
            mission_loss: if success { 0.0 } else { 1.0 },
            first_failure,
            min_capabilities: min_caps,
            verify_capabilities: verify_caps.unwrap_or(caps),
            final_capabilities: caps,
            final_modes,
            final_active_links,
            sampled_compromised: sampled,
            final_compromised,
        });
    }
    Ok(RolloutBatch {
        plan: plan.to_vec(),
        outcomes,
    })
}

/// Rollouts for several candidate plans under common random numbers.
pub fn evaluate_plans(
    base: &World,
    belief: &Belief,
    plans: &[Vec<CommandOp>],
    spec: &RolloutSpec,
) -> Vec<Result<RolloutBatch, SimError>> {
    plans
        .iter()
        .map(|p| run_rollouts(base, belief, p, spec))
        .collect()
}
