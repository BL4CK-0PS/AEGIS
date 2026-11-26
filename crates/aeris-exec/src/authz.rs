//! Execution-time authorization: the last gate before commands leave.
//!
//! The executor never trusts "it was recommended". It requires one of:
//!
//! * [`Authorization::Auto`] — only if the autonomy gate said `AUTO` **and**
//!   the executor's own recomputation agrees the action is LOW impact and
//!   reversible. A buggy or compromised gate cannot auto-run a HIGH action.
//! * Any action containing `Recover` always needs a human, even though
//!   `aeris-types` classifies it LOW: re-admitting an isolated node is only
//!   safe after the §6.13 recovery validation, which is not built in v0.1.
//! * [`Authorization::Human`] — an approval record for *exactly this action*
//!   by a role at least as high as the action's impact class
//!   (HIGH → commander; MEDIUM → operator or commander).

use aeris_types::{Action, Gate, ImpactClass};
use serde::{Deserialize, Serialize};

/// Operator roles (`docs/v0.1/06` `operators.role`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// May approve MEDIUM actions.
    Operator,
    /// May approve HIGH actions.
    Commander,
}

impl Role {
    /// Highest impact class this role may approve.
    pub fn max_class(self) -> ImpactClass {
        match self {
            Role::Operator => ImpactClass::Medium,
            Role::Commander => ImpactClass::High,
        }
    }
}

/// Human verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Approved the recommended action.
    Approve,
    /// Approved a different candidate (re-checked by policy).
    Modify,
    /// Rejected. Never executable.
    Reject,
}

/// A recorded human decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    /// Operator id.
    pub operator: String,
    /// Operator role at approval time.
    pub role: Role,
    /// Verdict.
    pub verdict: Verdict,
    /// The action the human approved.
    pub approved_action: Action,
    /// Reason (mandatory for reject/modify in the server).
    pub reason: String,
    /// Approval time, ms since epoch.
    pub at_ms: i64,
}

/// Why execution was not authorized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AuthzError {
    /// The gate rejected the action.
    #[error("gate rejected the action")]
    GateRejected,
    /// Auto authorization for an action that needs a human.
    #[error("auto authorization refused: {reason}")]
    AutoNotAllowed {
        /// Why.
        reason: String,
    },
    /// Human rejected it.
    #[error("human verdict was reject")]
    HumanRejected,
    /// The approval names a different action.
    #[error("approval is for a different action")]
    ActionMismatch,
    /// The approver's role is too low.
    #[error("role {role:?} cannot approve {class:?} actions")]
    InsufficientRole {
        /// Approver role.
        role: Role,
        /// Action class.
        class: ImpactClass,
    },
}

/// How an execution request is authorized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authorization {
    /// The autonomy gate's output for this action.
    Auto {
        /// Gate result as computed by the policy layer.
        gate: Gate,
    },
    /// A human approval.
    Human(Approval),
}

/// Checks `auth` for executing `action` whose target has criticality
/// `target_criticality`. Pure and deterministic.
pub fn authorize(
    action: &Action,
    target_criticality: f64,
    auth: &Authorization,
) -> Result<ImpactClass, AuthzError> {
    // Non-finite criticality must not downgrade the class: treat as maximal.
    let crit = if target_criticality.is_finite() {
        target_criticality
    } else {
        1.0
    };
    let class = action.impact_class(crit);
    match auth {
        Authorization::Auto { gate } => match gate {
            Gate::Reject { .. } => Err(AuthzError::GateRejected),
            Gate::HumanReview { reason } => Err(AuthzError::AutoNotAllowed {
                reason: format!("gate requires human review: {reason}"),
            }),
            Gate::Auto => {
                if contains_recover(action) {
                    return Err(AuthzError::AutoNotAllowed {
                        reason: "recover re-admits an isolated node; recovery validation (§6.13) is not implemented in v0.1, so a human must approve".into(),
                    });
                }
                if class != ImpactClass::Low {
                    return Err(AuthzError::AutoNotAllowed {
                        reason: format!("{class:?} impact needs a human"),
                    });
                }
                if !action.is_reversible() {
                    return Err(AuthzError::AutoNotAllowed {
                        reason: "irreversible action needs a human".into(),
                    });
                }
                Ok(class)
            }
        },
        Authorization::Human(a) => {
            if a.verdict == Verdict::Reject {
                return Err(AuthzError::HumanRejected);
            }
            if &a.approved_action != action {
                return Err(AuthzError::ActionMismatch);
            }
            if class > a.role.max_class() {
                return Err(AuthzError::InsufficientRole {
                    role: a.role,
                    class,
                });
            }
            Ok(class)
        }
    }
}

fn contains_recover(a: &Action) -> bool {
    match a {
        Action::Recover(_) => true,
        Action::Combo(v) => v.iter().any(contains_recover),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recover_never_runs_on_auto() {
        let a = Action::Recover("node-b".into());
        assert_eq!(
            a.impact_class(0.0),
            ImpactClass::Low,
            "aeris-types says LOW"
        );
        assert!(matches!(
            authorize(&a, 0.0, &Authorization::Auto { gate: Gate::Auto }),
            Err(AuthzError::AutoNotAllowed { .. })
        ));
        let combo = Action::Combo(vec![Action::Monitor, a.clone()]);
        assert!(authorize(&combo, 0.0, &Authorization::Auto { gate: Gate::Auto }).is_err());
        let human = Authorization::Human(Approval {
            operator: "op".into(),
            role: Role::Operator,
            verdict: Verdict::Approve,
            approved_action: a.clone(),
            reason: "validated manually".into(),
            at_ms: 0,
        });
        assert_eq!(authorize(&a, 0.0, &human), Ok(ImpactClass::Low));
    }

    #[test]
    fn role_ordering_matches_impact_classes() {
        assert!(Role::Commander > Role::Operator);
        assert_eq!(Role::Operator.max_class(), ImpactClass::Medium);
        assert_eq!(Role::Commander.max_class(), ImpactClass::High);
    }
}
