//! AERIS safety monitor and autonomy gate (docs/v0.1/04-mathematical-model.md §6.12).
//!
//! The optimizer proposes; policy disposes. This crate is deliberately
//! independent of `aeris-decision`: it never scores actions, it only decides
//! whether an action is allowed and who must approve it.
//!
//! Hard invariants are evaluated on the *structural* topology (every trust and
//! availability set to 1), so a veto depends on whether a provider exists at
//! all, never on how much we currently believe it.
//!
//! Owner: KESIVARSHINI (scaffolded by KISHANTH during G4 integration).

use aeris_mission::{ALL_CAPS, Topology};
use aeris_types::{Action, Gate, ImpactClass, Invariant};
use serde::{Deserialize, Serialize};

/// Autonomy-gate thresholds (`docs/spec/params.toml [policy]`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GateParams {
    /// θ_U: decision uncertainty at or above which a human must review.
    pub theta_u: f64,
    /// Model health below which AERIS is in FAILSAFE (I4).
    pub model_health_min: f64,
}

impl Default for GateParams {
    fn default() -> Self {
        Self {
            theta_u: 0.25,
            model_health_min: 0.90,
        }
    }
}

/// Everything the gate needs to know about one recommended action.
#[derive(Debug, Clone, PartialEq)]
pub struct GateInput<'a> {
    pub action: &'a Action,
    /// Criticality of the action's target, C_i.
    pub target_criticality: f64,
    /// u_t: max(trust std-dev, normalised posterior entropy).
    pub uncertainty: f64,
    pub model_health: f64,
    /// True when no candidate met the chance constraint (§6.11 fallback).
    pub no_option_meets_bound: bool,
}

/// Checks hard invariants I1–I3 for `action` against the current topology.
/// An empty result means the action may be scored.
pub fn check_invariants(topology: &Topology, action: &Action) -> Vec<Invariant> {
    let mut violations = Vec::new();

    if !targets_exist_and_change_state(topology, action) {
        violations.push(Invariant::I3_EntityMustExistAndNotRedundant);
        return violations;
    }

    let before = topology.structural();
    let after = before.apply(action);
    let caps_before = before.capabilities();
    let caps_after = after.capabilities();

    // I1: never remove the last active communication path between the endpoints.
    if caps_before.communication > 0.0 && caps_after.communication <= 0.0 {
        violations.push(Invariant::I1_NoSeverLastCommPath);
    }

    // I2: never isolate the only provider of a required capability unless the
    // same action restores it (it then still has a provider afterwards).
    if isolates_any(action)
        && ALL_CAPS
            .iter()
            .any(|c| caps_before.get(*c) > 0.0 && caps_after.get(*c) <= 0.0)
    {
        violations.push(Invariant::I2_NoIsolateSoleCapabilityProvider);
    }

    violations
}

fn isolates_any(action: &Action) -> bool {
    match action {
        Action::Isolate(_) => true,
        Action::Combo(acts) => acts.iter().any(isolates_any),
        _ => false,
    }
}

/// I3: the target exists and isn't already in the target state.
fn targets_exist_and_change_state(topology: &Topology, action: &Action) -> bool {
    match action {
        Action::Monitor | Action::Degrade(_) => true,
        Action::Restrict(id) | Action::Isolate(id) => topology.node(id).is_some_and(|n| n.active),
        Action::Recover(id) => topology.node(id).is_some_and(|n| !n.active),
        Action::Reroute(id) => topology.link(id).is_some_and(|l| !l.active),
        Action::Failover { from, to } => {
            from != to && topology.node(from).is_some() && topology.node(to).is_some()
        }
        Action::Combo(acts) => {
            !acts.is_empty()
                && acts
                    .iter()
                    .all(|a| targets_exist_and_change_state(topology, a))
        }
    }
}

/// The autonomy gate H(a) (§6.12):
/// REJECT for any hard-invariant violation; HUMAN_REVIEW if no option met the
/// failure bound, class(a) ≠ LOW, u ≥ θ_U, a is irreversible, or model health
/// is degraded (I4); otherwise AUTO.
pub fn autonomy_gate(input: &GateInput<'_>, violations: &[Invariant], p: &GateParams) -> Gate {
    if let Some(inv) = violations.first() {
        return Gate::Reject {
            invariant: inv.clone(),
        };
    }
    if input.no_option_meets_bound {
        return Gate::HumanReview {
            reason: "No candidate option meets the failure bound epsilon".to_string(),
        };
    }
    let impact = input.action.impact_class(input.target_criticality);
    if impact != ImpactClass::Low {
        return Gate::HumanReview {
            reason: format!(
                "Action has {:?} impact on mission (criticality {:.2})",
                impact, input.target_criticality
            ),
        };
    }
    if input.uncertainty >= p.theta_u {
        return Gate::HumanReview {
            reason: format!(
                "Decision uncertainty {:.3} exceeds threshold {:.2}",
                input.uncertainty, p.theta_u
            ),
        };
    }
    if !input.action.is_reversible() {
        return Gate::HumanReview {
            reason: "Action is not fully reversible".to_string(),
        };
    }
    if input.model_health < p.model_health_min {
        return Gate::HumanReview {
            reason: format!(
                "Model health {:.2} is below threshold {:.2} (I4)",
                input.model_health, p.model_health_min
            ),
        };
    }
    Gate::Auto
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeris_mission::{LinkView, Minima, NodeView};

    fn node(id: &str) -> NodeView {
        NodeView {
            id: id.into(),
            active: true,
            trust: 0.5,
            coverage: 0.6,
            obs_trust: 0.948,
            position_variance: 4.0,
        }
    }

    fn s4_world() -> Topology {
        let minima = Minima {
            navigation: 0.85,
            communication: 0.90,
            observation: 0.80,
        };
        Topology {
            nodes: vec![node("node-a"), node("node-b"), node("node-c")],
            links: vec![
                LinkView {
                    id: "link-l1".into(),
                    endpoints: ["node-a".into(), "node-c".into()],
                    via: vec!["node-b".into()],
                    hop_availability: vec![0.99, 0.99],
                    active: true,
                },
                LinkView {
                    id: "link-l2".into(),
                    endpoints: ["node-a".into(), "node-c".into()],
                    via: vec![],
                    hop_availability: vec![0.93],
                    active: false,
                },
            ],
            endpoints: ["node-a".into(), "node-c".into()],
            minima,
            degraded_minima: minima,
            sigma_max: 30.0,
        }
    }

    fn isolate_b() -> Action {
        Action::Isolate("node-b".into())
    }

    fn isolate_b_reroute_l2() -> Action {
        Action::Combo(vec![isolate_b(), Action::Reroute("link-l2".into())])
    }

    #[test]
    fn isolating_sole_relay_violates_i1_and_i2() {
        assert_eq!(
            check_invariants(&s4_world(), &isolate_b()),
            vec![
                Invariant::I1_NoSeverLastCommPath,
                Invariant::I2_NoIsolateSoleCapabilityProvider
            ]
        );
    }

    #[test]
    fn isolate_with_reroute_is_allowed() {
        assert_eq!(
            check_invariants(&s4_world(), &isolate_b_reroute_l2()),
            vec![]
        );
    }

    #[test]
    fn isolating_relay_is_allowed_once_backup_is_on() {
        let w = s4_world().apply(&Action::Reroute("link-l2".into()));
        assert_eq!(check_invariants(&w, &isolate_b()), vec![]);
    }

    #[test]
    fn isolating_an_endpoint_violates_i1() {
        let v = check_invariants(&s4_world(), &Action::Isolate("node-a".into()));
        assert!(
            v.contains(&Invariant::I1_NoSeverLastCommPath),
            "expected I1, got {v:?}"
        );
    }

    #[test]
    fn low_trust_does_not_cause_a_veto() {
        // A distrusted relay still *exists*: vetoes are structural.
        let mut w = s4_world();
        for n in &mut w.nodes {
            n.trust = 0.01;
        }
        assert_eq!(check_invariants(&w, &Action::Monitor), vec![]);
        assert_eq!(check_invariants(&w, &isolate_b_reroute_l2()), vec![]);
    }

    #[test]
    fn redundant_or_unknown_targets_violate_i3() {
        let i3 = vec![Invariant::I3_EntityMustExistAndNotRedundant];
        assert_eq!(
            check_invariants(&s4_world(), &Action::Isolate("node-z".into())),
            i3
        );
        assert_eq!(
            check_invariants(&s4_world(), &Action::Reroute("link-l1".into())),
            i3
        );
        assert_eq!(
            check_invariants(&s4_world(), &Action::Recover("node-b".into())),
            i3
        );
        let isolated = s4_world().apply(&isolate_b_reroute_l2());
        assert_eq!(check_invariants(&isolated, &isolate_b()), i3);
    }

    fn input(action: &Action, crit: f64, u: f64, health: f64) -> GateInput<'_> {
        GateInput {
            action,
            target_criticality: crit,
            uncertainty: u,
            model_health: health,
            no_option_meets_bound: false,
        }
    }

    fn review_reason(g: &Gate) -> &str {
        match g {
            Gate::HumanReview { reason } => reason,
            other => panic!("expected HumanReview, got {other:?}"),
        }
    }

    #[test]
    fn gate_rejects_invariant_violations() {
        let a = isolate_b();
        let g = autonomy_gate(
            &input(&a, 1.0, 0.0, 1.0),
            &[Invariant::I1_NoSeverLastCommPath],
            &GateParams::default(),
        );
        assert_eq!(
            g,
            Gate::Reject {
                invariant: Invariant::I1_NoSeverLastCommPath
            }
        );
    }

    #[test]
    fn gate_sends_high_impact_to_human() {
        let a = isolate_b_reroute_l2();
        let g = autonomy_gate(&input(&a, 1.0, 0.0, 1.0), &[], &GateParams::default());
        assert!(review_reason(&g).contains("High impact"), "{g:?}");
    }

    #[test]
    fn gate_allows_low_impact_when_confident_and_healthy() {
        let a = Action::Monitor;
        let g = autonomy_gate(&input(&a, 1.0, 0.1, 1.0), &[], &GateParams::default());
        assert_eq!(g, Gate::Auto);
    }

    #[test]
    fn more_doubt_means_less_autonomy() {
        let a = Action::Monitor;
        let g = autonomy_gate(&input(&a, 0.0, 0.25, 1.0), &[], &GateParams::default());
        assert!(review_reason(&g).contains("uncertainty"), "{g:?}");
    }

    #[test]
    fn degraded_model_forces_review_i4() {
        let a = Action::Monitor;
        let g = autonomy_gate(&input(&a, 0.0, 0.0, 0.89), &[], &GateParams::default());
        assert!(review_reason(&g).contains("I4"), "{g:?}");
    }

    #[test]
    fn failure_bound_fallback_forces_review() {
        let a = Action::Monitor;
        let mut i = input(&a, 0.0, 0.0, 1.0);
        i.no_option_meets_bound = true;
        let g = autonomy_gate(&i, &[], &GateParams::default());
        assert!(review_reason(&g).contains("failure bound"), "{g:?}");
    }
}
