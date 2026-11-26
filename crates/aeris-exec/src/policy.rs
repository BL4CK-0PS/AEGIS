//! Policy re-check at execution time (§6.12: "re-evaluate Π against the
//! current state, not the state when the recommendation was made").
//!
//! The policy itself (Π = Auth ∧ Safety ∧ Mission ∧ Context, invariants
//! I1–I4) is owned by `aeris-policy` (Kesivarshini), which does not exist on
//! `main` yet. The executor therefore takes any [`PolicyRecheck`]. Until
//! `aeris-policy` lands, [`TopologyGuard`] provides the invariants that can be
//! decided from observable topology alone (I1, I2, I3). It is a defence-in-
//! depth guard, not a replacement for the policy crate.

use aeris_sim::{WorldConfig, capabilities_from_observation, expected_topology};
use aeris_types::{Action, Capability, Invariant};
use aeris_wire::{CommandOp, WorldObservation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A policy veto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("policy veto ({invariant:?}): {reason}")]
pub struct PolicyVeto {
    /// Violated invariant, if one applies.
    pub invariant: Option<Invariant>,
    /// Explanation.
    pub reason: String,
}

/// Re-evaluates policy for `action` (with its command plan) against the
/// current observable state.
pub trait PolicyRecheck: Send + Sync {
    /// `Ok(())` if execution may proceed.
    fn recheck(
        &self,
        action: &Action,
        plan: &[CommandOp],
        current: &WorldObservation,
    ) -> Result<(), PolicyVeto>;
}

/// Interim topology invariants (see module docs).
#[derive(Debug, Clone)]
pub struct TopologyGuard {
    cfg: Arc<WorldConfig>,
}

impl TopologyGuard {
    /// Guard for the given world.
    pub fn new(cfg: Arc<WorldConfig>) -> Self {
        Self { cfg }
    }
}

impl PolicyRecheck for TopologyGuard {
    fn recheck(
        &self,
        action: &Action,
        plan: &[CommandOp],
        current: &WorldObservation,
    ) -> Result<(), PolicyVeto> {
        // I3: every target exists and the plan is applicable.
        let after = expected_topology(&self.cfg, current, plan).map_err(|e| PolicyVeto {
            invariant: Some(Invariant::I3_EntityMustExistAndNotRedundant),
            reason: e.to_string(),
        })?;
        let est = |o: &WorldObservation| {
            capabilities_from_observation(&self.cfg, o).map_err(|e| PolicyVeto {
                invariant: Some(Invariant::I3_EntityMustExistAndNotRedundant),
                reason: e.to_string(),
            })
        };
        let before = est(current)?;
        let post = est(&after)?;
        // I1: never remove the last active communication path.
        if before.communication > 0.0 && post.communication <= 0.0 {
            return Err(PolicyVeto {
                invariant: Some(Invariant::I1_NoSeverLastCommPath),
                reason: format!(
                    "{} removes the last active path between the mission endpoints",
                    action.display_name()
                ),
            });
        }
        // I2: removing a node must not drop a required capability below its
        // minimum unless the same action degrades that capability explicitly.
        let removes_node = plan
            .iter()
            .any(|op| matches!(op, CommandOp::Isolate { .. } | CommandOp::Failover { .. }));
        if removes_node {
            let m = &self.cfg.mission;
            for c in [
                Capability::Navigation,
                Capability::Communication,
                Capability::Observation,
            ] {
                let degraded = after.degraded.contains(&c);
                let min = if degraded {
                    m.degraded_min.get(c)
                } else {
                    m.min.get(c)
                };
                if before.get(c) >= min && post.get(c) < min {
                    return Err(PolicyVeto {
                        invariant: Some(Invariant::I2_NoIsolateSoleCapabilityProvider),
                        reason: format!(
                            "{} drops {c} from {:.3} to {:.3} (< {min})",
                            action.display_name(),
                            before.get(c),
                            post.get(c)
                        ),
                    });
                }
            }
        }
        Ok(())
    }
}
