//! Decision optimization, counterfactual rollouts evaluation, CVaR tail-risk,
//! and constrained argmax action selection.
//!
//! Defined in docs/v0.1/04-mathematical-model.md §6.10, §6.11, §6.12.
//! Owner: KISHANTH

use aeris_types::{
    Action, Candidate, EntityId, Gate, ImpactClass, Invariant, LinkId, Proposal,
};
use serde::{Deserialize, Serialize};

/// Hyperparameters for decision scoring and constraints.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DecisionParams {
    /// CVaR tail-risk alpha level (default 0.90: worst 10% of outcomes)
    pub cvar_alpha: f64,
    /// CVaR penalty lambda in J(a) = E[U|a] - lambda * CVaR (default 0.50)
    pub lambda_penalty: f64,
    /// Chance constraint failure rate threshold epsilon (default 0.10)
    pub chance_epsilon: f64,
    /// Critical value z for upper confidence bound (1.645 for 95% one-sided UCB)
    pub ucb_z: f64,
    /// Uncertainty threshold theta_U for autonomy gate (default 0.25)
    pub theta_u: f64,
    /// Model health threshold (default 0.90)
    pub model_health_min: f64,
}

impl Default for DecisionParams {
    fn default() -> Self {
        Self {
            cvar_alpha: 0.90,
            lambda_penalty: 0.50,
            chance_epsilon: 0.10,
            ucb_z: 1.645,
            theta_u: 0.25,
            model_health_min: 0.90,
        }
    }
}

/// Computes the empirical CVaR_alpha from a list of loss samples.
/// CVaR_alpha is the average loss across the worst (1 - alpha) fraction of outcomes.
pub fn compute_cvar(losses: &[f64], alpha: f64) -> f64 {
    if losses.is_empty() {
        return 0.0;
    }
    let mut sorted = losses.to_vec();
    // Sort losses ascending (higher loss is worse)
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let n = sorted.len();
    let worst_count = ((1.0 - alpha) * n as f64).ceil() as usize;
    let worst_count = worst_count.max(1).min(n);

    // Take the worst (highest loss) samples from the tail
    let tail = &sorted[n - worst_count..];
    let sum: f64 = tail.iter().sum();
    sum / (worst_count as f64)
}

/// Evaluates rollout samples for an action:
/// computes p_success, standard error, expected utility, CVaR, and score J(a).
pub fn evaluate_rollout_samples(
    action: Action,
    successes: &[bool],
    params: &DecisionParams,
) -> Candidate {
    let n = successes.len();
    assert!(n > 0, "Rollout samples cannot be empty");

    let d = action.disruption_cost();
    let mut losses = Vec::with_capacity(n);
    let mut success_count = 0;

    for &succ in successes {
        let loss = if succ {
            success_count += 1;
            0.0 + d
        } else {
            1.0 + d
        };
        losses.push(loss);
    }

    let p_success = (success_count as f64) / (n as f64);
    let se = (p_success * (1.0 - p_success) / (n as f64)).sqrt();
    let exp_utility = p_success - d;

    let cvar = compute_cvar(&losses, params.cvar_alpha);
    let score = exp_utility - params.lambda_penalty * cvar;

    Candidate {
        action,
        p_success,
        se,
        exp_utility,
        cvar,
        score,
        violations: Vec::new(),
        disruption_cost: d,
    }
}

/// Evaluates a candidate directly from known summary statistics (used in verification and tests).
pub fn evaluate_summary(
    action: Action,
    p_success: f64,
    rollout_n: usize,
    params: &DecisionParams,
) -> Candidate {
    let n = rollout_n.max(1);
    let d = action.disruption_cost();
    let p = p_success.clamp(0.0, 1.0);
    let se = (p * (1.0 - p) / (n as f64)).sqrt();
    let exp_utility = p - d;

    // Synthetic rollouts consistent with p_success
    let success_count = (p * (n as f64)).round() as usize;
    let mut losses = Vec::with_capacity(n);
    for i in 0..n {
        if i < success_count {
            losses.push(0.0 + d);
        } else {
            losses.push(1.0 + d);
        }
    }

    let cvar = compute_cvar(&losses, params.cvar_alpha);
    let score = exp_utility - params.lambda_penalty * cvar;

    Candidate {
        action,
        p_success: p,
        se,
        exp_utility,
        cvar,
        score,
        violations: Vec::new(),
        disruption_cost: d,
    }
}

/// Action generator for standard tactical cluster with nodes and links.
pub fn generate_candidates_for_incident(
    target_node: &EntityId,
    backup_link: &LinkId,
) -> Vec<Action> {
    vec![
        Action::Monitor,
        Action::Restrict(target_node.clone()),
        Action::Reroute(backup_link.clone()),
        Action::Isolate(target_node.clone()),
        Action::Combo(vec![
            Action::Isolate(target_node.clone()),
            Action::Reroute(backup_link.clone()),
        ]),
        Action::Degrade(aeris_types::Capability::Communication),
    ]
}

/// Safety policy checking hard invariants before scoring:
/// - Invariant I1: Never remove the last active communication path
/// - Invariant I2: Never isolate sole provider of required capability unless restored
pub fn check_hard_invariants(
    action: &Action,
    target_node: &str,
    primary_link_active: bool,
    backup_link_active: bool,
    target_is_sole_comm_relay: bool,
) -> Vec<Invariant> {
    let mut violations = Vec::new();

    match action {
        Action::Isolate(entity) if entity == target_node => {
            // Isolating target node B alone when L1 is active and L2 is not active
            // severs the last active communication path A-B-C!
            if target_is_sole_comm_relay && primary_link_active && !backup_link_active {
                violations.push(Invariant::I1_NoSeverLastCommPath);
                violations.push(Invariant::I2_NoIsolateSoleCapabilityProvider);
            }
        }
        Action::Combo(actions) => {
            let has_isolate = actions.iter().any(|a| matches!(a, Action::Isolate(e) if e == target_node));
            let has_reroute = actions.iter().any(|a| matches!(a, Action::Reroute(_)));

            // If isolating without rerouting, it violates I1
            if has_isolate && !has_reroute && target_is_sole_comm_relay && !backup_link_active {
                violations.push(Invariant::I1_NoSeverLastCommPath);
            }
            // If combo includes reroute, backup link is activated simultaneously, so I1 is satisfied!
        }
        _ => {}
    }

    violations
}

/// Checks the chance constraint on failure rate:
/// (1 - P(success|a)) + z * SE <= epsilon
pub fn passes_chance_constraint(cand: &Candidate, params: &DecisionParams) -> bool {
    let ucb_fail = (1.0 - cand.p_success) + params.ucb_z * cand.se;
    ucb_fail <= params.chance_epsilon
}

/// Optimizes action selection across candidates given invariants, chance bounds, and autonomy gate.
pub fn optimize_decision(
    decision_id: String,
    incident_id: String,
    candidates: Vec<Candidate>,
    target_criticality: f64,
    decision_uncertainty: f64,
    model_health: f64,
    params: &DecisionParams,
) -> Proposal {
    // 1. Separate actions that pass hard invariants
    let mut feasible: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| c.violations.is_empty())
        .collect();

    // 2. Filter feasible by chance constraint
    let mut passing_chance: Vec<&Candidate> = feasible
        .iter()
        .copied()
        .filter(|c| passes_chance_constraint(c, params))
        .collect();

    let (chosen, fallback_forced_review) = if !passing_chance.is_empty() {
        // Argmax J(a) among passing
        passing_chance.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        (passing_chance[0].clone(), false)
    } else if !feasible.is_empty() {
        // Fallback: pick highest J among actions passing invariants, but force human review
        feasible.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        (feasible[0].clone(), true)
    } else {
        // Extreme fallback: Monitor
        (
            candidates
                .iter()
                .find(|c| matches!(c.action, Action::Monitor))
                .cloned()
                .unwrap_or_else(|| evaluate_summary(Action::Monitor, 0.0, 100, params)),
            true,
        )
    };

    // 3. Autonomy Gate evaluation
    let impact = chosen.action.impact_class(target_criticality);
    let is_reversible = chosen.action.is_reversible();

    let gate = if fallback_forced_review {
        Gate::HumanReview {
            reason: "No candidate option meets the failure bound epsilon".to_string(),
        }
    } else if impact != ImpactClass::Low {
        Gate::HumanReview {
            reason: format!(
                "Action has {:?} impact on mission (criticality {:.2})",
                impact, target_criticality
            ),
        }
    } else if decision_uncertainty >= params.theta_u {
        Gate::HumanReview {
            reason: format!(
                "Decision uncertainty {:.3} exceeds threshold {:.2}",
                decision_uncertainty, params.theta_u
            ),
        }
    } else if !is_reversible {
        Gate::HumanReview {
            reason: "Action is not fully reversible".to_string(),
        }
    } else if model_health < params.model_health_min {
        Gate::HumanReview {
            reason: format!(
                "Model health {:.2} is below threshold {:.2}",
                model_health, params.model_health_min
            ),
        }
    } else {
        Gate::Auto
    };

    Proposal {
        id: decision_id,
        incident_id,
        recommended_action: chosen.action,
        candidates,
        gate,
        explanation: String::new(),
        cited_evidence: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies the exact worked numbers from docs/v0.1/04-mathematical-model.md §6.11:
    ///
    /// | Action | P(success) | d(a) | E[U] | CVaR_0.9(L) | J(a) | Constraints |
    /// |---|---|---|---|---|---|---|
    /// | Monitor | 0.31 | 0.00 | 0.310 | 1.000 | -0.190 | fails chance |
    /// | Reroute only | 0.55 | 0.05 | 0.500 | 1.050 | -0.025 | fails chance |
    /// | Isolate B only | - | - | - | - | - | violates I1 |
    /// | Isolate B + reroute L2 | 0.94 | 0.15 | 0.790 | 0.750 | 0.415 | passes (UCB 0.078 <= 0.10) |
    /// | Degrade mission | 0.68 | 0.30 | 0.380 | 1.300 | -0.270 | fails chance |
    #[test]
    fn test_worked_example_decision_optimizer() {
        let params = DecisionParams::default();

        // 1. Monitor
        let c_mon = evaluate_summary(Action::Monitor, 0.31, 500, &params);
        assert!((c_mon.exp_utility - 0.310).abs() < 1e-3);
        assert!((c_mon.cvar - 1.000).abs() < 1e-3);
        assert!((c_mon.score - (-0.190)).abs() < 1e-3);
        assert!(!passes_chance_constraint(&c_mon, &params));

        // 2. Reroute only
        let c_reroute = evaluate_summary(Action::Reroute("link-l2".into()), 0.55, 500, &params);
        assert!((c_reroute.exp_utility - 0.500).abs() < 1e-3);
        assert!((c_reroute.cvar - 1.050).abs() < 1e-3);
        assert!((c_reroute.score - (-0.025)).abs() < 1e-3);
        assert!(!passes_chance_constraint(&c_reroute, &params));

        // 3. Isolate B only (violates I1)
        let mut c_isolate = evaluate_summary(Action::Isolate("node-b".into()), 0.0, 500, &params);
        c_isolate.violations = check_hard_invariants(
            &c_isolate.action,
            "node-b",
            true,  // primary link L1 active
            false, // backup link L2 inactive
            true,  // node B is sole comm relay
        );
        assert!(c_isolate.violations.contains(&Invariant::I1_NoSeverLastCommPath));

        // 4. Isolate B + Reroute L2
        let combo = Action::Combo(vec![
            Action::Isolate("node-b".into()),
            Action::Reroute("link-l2".into()),
        ]);
        let mut c_combo = evaluate_summary(combo.clone(), 0.94, 500, &params);
        c_combo.violations = check_hard_invariants(
            &c_combo.action,
            "node-b",
            true,
            false,
            true,
        );
        assert!(c_combo.violations.is_empty(), "Combo must satisfy I1 because L2 is activated simultaneously");
        assert!((c_combo.exp_utility - 0.790).abs() < 1e-3);
        assert!((c_combo.cvar - 0.750).abs() < 1e-3);
        assert!((c_combo.score - 0.415).abs() < 1e-3);
        assert!(passes_chance_constraint(&c_combo, &params));

        // 5. Degrade
        let c_degrade = evaluate_summary(Action::Degrade(aeris_types::Capability::Communication), 0.68, 500, &params);
        assert!((c_degrade.exp_utility - 0.380).abs() < 1e-3);
        assert!((c_degrade.cvar - 1.300).abs() < 1e-3);
        assert!((c_degrade.score - (-0.270)).abs() < 1e-3);
        assert!(!passes_chance_constraint(&c_degrade, &params));

        // Optimize across all 5
        let candidates = vec![c_mon, c_reroute, c_isolate, c_combo, c_degrade];
        let proposal = optimize_decision(
            "dec-01".into(),
            "inc-01".into(),
            candidates,
            1.0,  // B's criticality is 1.0 (High impact)
            0.13, // uncertainty
            1.0,  // model health
            &params,
        );

        // Best action is Isolate B + Reroute L2!
        assert_eq!(proposal.recommended_action, combo);
        // Gate is HumanReview because impact is High (criticality 1.0 >= 0.5)
        assert!(matches!(proposal.gate, Gate::HumanReview { .. }));
    }
}
