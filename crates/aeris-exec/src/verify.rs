//! Execution verification (§6.13).
//!
//! ```text
//! e = S_obs(t+T_v) − Ŝ(t+T_v)
//! success ⇔ every hard component of e is 0  ∧  ‖e_soft‖∞ ≤ ε
//! ```
//!
//! Hard components (must match exactly):
//! * each node's mode and each link's `active` flag equal the expected topology,
//!   and every link expected active is physically `up`;
//! * the set of degraded capabilities equals the expected one;
//! * every capability meets its (possibly degraded) minimum — the error is the
//!   deficit `max(0, c_min − c_obs)`.
//!
//! Soft components (tolerance ε): `|c_obs − ĉ|` against the expected
//! capabilities (the median rollout of the chosen action), when given.
//!
//! Observed capabilities are AERIS's estimate (§6.7) from the observation
//! and the caller's current node trust ([`aeris_sim::estimate_capabilities`]);
//! verification never reads simulator ground truth.

use aeris_sim::{Capabilities, CapabilityMinima, SimError, WorldConfig, estimate_capabilities};
use aeris_types::Capability;
use aeris_wire::WorldObservation;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// What the world should look like after the action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpectedState {
    /// Expected topology (from `aeris_sim::expected_topology`).
    pub topology: WorldObservation,
    /// Expected capability levels (median rollout), if known.
    pub capabilities: Option<Capabilities>,
}

/// Hard components must be exactly zero; soft ones within ε.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// Exact.
    Hard,
    /// Within ε.
    Soft,
}

/// One component of the error vector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorComponent {
    /// What is compared (`node-b.mode`, `link-l2.active`, `communication.min`).
    pub name: String,
    /// Hard or soft.
    pub kind: ComponentKind,
    /// Expected value (booleans/modes encoded as 0/1 match indicators).
    pub expected: f64,
    /// Observed value.
    pub observed: f64,
    /// |error| for this component.
    pub error: f64,
}

/// Verification outcome with full diagnostics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verification {
    /// Success.
    pub ok: bool,
    /// ‖e‖∞ over all components.
    pub error_norm: f64,
    /// ‖e_soft‖∞.
    pub soft_norm: f64,
    /// Tolerance used.
    pub epsilon: f64,
    /// Every component (non-zero ones first is not guaranteed; filter on `error`).
    pub components: Vec<ErrorComponent>,
    /// Observation verified against.
    pub observed: WorldObservation,
    /// Capabilities estimated from it.
    pub observed_capabilities: Capabilities,
}

impl Verification {
    /// Components with non-zero error.
    pub fn failures(&self) -> impl Iterator<Item = &ErrorComponent> {
        self.components.iter().filter(|c| c.error > 0.0)
    }
}

fn minima_for(cfg: &WorldConfig, degraded: &[Capability]) -> CapabilityMinima {
    let m = &cfg.mission;
    let pick = |c: Capability| {
        if degraded.contains(&c) {
            m.degraded_min.get(c)
        } else {
            m.min.get(c)
        }
    };
    CapabilityMinima {
        navigation: pick(Capability::Navigation),
        communication: pick(Capability::Communication),
        observation: pick(Capability::Observation),
    }
}

/// Compares `observed` with `expected`. `node_trust` is AERIS's current
/// node trust (empty = fully trusted).
pub fn verify(
    cfg: &Arc<WorldConfig>,
    expected: &ExpectedState,
    observed: &WorldObservation,
    epsilon: f64,
    node_trust: &[(String, f64)],
) -> Result<Verification, SimError> {
    let caps = estimate_capabilities(cfg, observed, node_trust)?;
    let mut comps = Vec::new();
    let hard = |name: String, matches: bool| ErrorComponent {
        name,
        kind: ComponentKind::Hard,
        expected: 1.0,
        observed: if matches { 1.0 } else { 0.0 },
        error: if matches { 0.0 } else { 1.0 },
    };
    for (node, mode) in &expected.topology.nodes {
        comps.push(hard(
            format!("{node}.mode={mode:?}"),
            observed.node_mode(node) == Some(*mode),
        ));
    }
    for l in &expected.topology.links {
        let o = observed.link(&l.link);
        comps.push(hard(
            format!("{}.active={}", l.link, l.active),
            o.is_some_and(|o| o.active == l.active),
        ));
        if l.active {
            comps.push(hard(format!("{}.up", l.link), o.is_some_and(|o| o.up)));
        }
    }
    let mut exp_deg = expected.topology.degraded.clone();
    let mut obs_deg = observed.degraded.clone();
    exp_deg.sort_by_key(|c| c.to_string());
    obs_deg.sort_by_key(|c| c.to_string());
    comps.push(hard("degraded".into(), exp_deg == obs_deg));
    let mins = minima_for(cfg, &observed.degraded);
    for c in [
        Capability::Navigation,
        Capability::Communication,
        Capability::Observation,
    ] {
        let deficit = (mins.get(c) - caps.get(c)).max(0.0);
        comps.push(ErrorComponent {
            name: format!("{c}>=min"),
            kind: ComponentKind::Hard,
            expected: mins.get(c),
            observed: caps.get(c),
            error: deficit,
        });
        if let Some(exp) = &expected.capabilities {
            comps.push(ErrorComponent {
                name: format!("{c}~expected"),
                kind: ComponentKind::Soft,
                expected: exp.get(c),
                observed: caps.get(c),
                error: (caps.get(c) - exp.get(c)).abs(),
            });
        }
    }
    let error_norm = comps.iter().map(|c| c.error).fold(0.0, f64::max);
    let soft_norm = comps
        .iter()
        .filter(|c| c.kind == ComponentKind::Soft)
        .map(|c| c.error)
        .fold(0.0, f64::max);
    let hard_ok = comps
        .iter()
        .filter(|c| c.kind == ComponentKind::Hard)
        .all(|c| c.error == 0.0);
    Ok(Verification {
        ok: hard_ok && soft_norm <= epsilon,
        error_norm,
        soft_norm,
        epsilon,
        components: comps,
        observed: observed.clone(),
        observed_capabilities: caps,
    })
}
