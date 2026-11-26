//! The executor: approved action → commands → verification → result.
//!
//! ```text
//! request ─▶ duplicate? ─▶ Γ(a) valid? ─▶ authorized? ─▶ observe current ─▶ policy re-check
//!        ─▶ submit commands in order (each with a deadline) ─▶ verify within T_v
//!        ─▶ VERIFIED  |  FAILED (+ replan signal; escalate after 3)
//! ```
//!
//! Every refusal happens before any command is sent. Every failure after a
//! command was sent is reported with what was applied and what was observed.

use crate::adapter::{Adapter, AdapterError};
use crate::authz::{Authorization, AuthzError, authorize};
use crate::policy::{PolicyRecheck, PolicyVeto};
use crate::verify::{ExpectedState, Verification, verify};
use aeris_sim::{Capabilities, WorldConfig, expected_topology};
use aeris_types::{Action, ImpactClass};
use aeris_wire::{AckStatus, CommandOp, PlanError, WorldObservation, commands_for_action};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// Executor parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ExecConfig {
    /// Deadline for each command's acknowledgement.
    pub cmd_timeout: Duration,
    /// Verification window T_v (spec: 3 s).
    pub verify_window: Duration,
    /// Poll interval inside the window.
    pub verify_poll: Duration,
    /// Tolerance ε on soft error components.
    pub epsilon: f64,
    /// Failed attempts per incident before escalating to a human (spec: 3).
    pub max_replans: u32,
    /// Bound on remembered decisions (duplicate detection).
    pub ledger_capacity: usize,
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            cmd_timeout: Duration::from_secs(2),
            verify_window: Duration::from_secs(3),
            verify_poll: Duration::from_millis(100),
            epsilon: 0.05,
            max_replans: 3,
            ledger_capacity: 10_000,
        }
    }
}

/// An approved action to execute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionRequest {
    /// Decision id (idempotency key).
    pub decision_id: String,
    /// Incident the decision answers.
    pub incident_id: String,
    /// The action.
    pub action: Action,
    /// Why it may run.
    pub authorization: Authorization,
    /// Criticality of the action's target (for the impact class).
    pub target_criticality: f64,
    /// Expected capabilities (median rollout of this action), if known.
    pub expected_capabilities: Option<Capabilities>,
    /// AERIS's node trust at request time, used for the capability estimate
    /// during verification (§6.7: a distrusted relay is a bad path). Empty
    /// means fully trusted.
    pub node_trust: Vec<(String, f64)>,
}

/// Decision lifecycle as seen by the executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Lifecycle {
    /// Commands are being sent.
    Executing,
    /// All commands acknowledged; verifying.
    Executed,
    /// Verification succeeded.
    Verified,
    /// Execution or verification failed.
    Failed,
    /// Refused before anything was sent.
    Refused,
}

/// Why a request was refused before execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Refusal {
    /// This decision was already executed (or is executing).
    #[error("duplicate execution of decision")]
    Duplicate,
    /// The action cannot be turned into commands.
    #[error("invalid action: {reason}")]
    Invalid {
        /// Why.
        reason: String,
    },
    /// Authorization failed.
    #[error("unauthorized: {0}")]
    Unauthorized(AuthzError),
    /// Policy re-check vetoed it.
    #[error("{0}")]
    PolicyVeto(PolicyVeto),
    /// The current state could not be observed, so policy cannot be re-checked.
    #[error("cannot observe current state: {reason}")]
    ObserveFailed {
        /// Why.
        reason: String,
    },
}

impl From<PlanError> for Refusal {
    fn from(e: PlanError) -> Self {
        Refusal::Invalid {
            reason: e.to_string(),
        }
    }
}

/// Final state of an execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecState {
    /// Commands applied and the world matches the expectation.
    Verified,
    /// Commands acknowledged, but the observed state does not match.
    VerificationFailed,
    /// The first command failed; nothing was applied.
    CommandFailed,
    /// Some commands applied, then one failed.
    PartiallyExecuted,
    /// An acknowledgement did not arrive in time.
    TimedOut,
    /// Refused before execution (see `refusal`).
    Refused,
}

/// What happened to one command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRecord {
    /// The command.
    pub op: CommandOp,
    /// Ack status, or `None` if no valid ack arrived.
    pub status: Option<AckStatus>,
    /// Ack reason or transport error.
    pub detail: String,
    /// Command id from the ack.
    pub command_id: Option<String>,
}

/// Instruction to re-run the decision from the new state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplanSignal {
    /// Failed decision.
    pub decision_id: String,
    /// Incident to re-plan.
    pub incident_id: String,
    /// Why.
    pub reason: String,
    /// Failed attempts for this incident so far (1-based).
    pub attempt: u32,
    /// `attempt >= max_replans`: stop re-planning and hand to a human.
    pub escalate_to_human: bool,
}

/// Everything the executor did and saw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionReport {
    /// Decision id.
    pub decision_id: String,
    /// Incident id.
    pub incident_id: String,
    /// Action.
    pub action: Action,
    /// Impact class (if authorization got that far).
    pub impact: Option<ImpactClass>,
    /// Γ(a).
    pub plan: Vec<CommandOp>,
    /// Per-command results, in send order.
    pub commands: Vec<CommandRecord>,
    /// Final state.
    pub state: ExecState,
    /// Refusal reason when `state == Refused`.
    pub refusal: Option<Refusal>,
    /// Observation before execution.
    pub pre_observation: Option<WorldObservation>,
    /// Expected state.
    pub expected: Option<ExpectedState>,
    /// Last verification (also recorded on failures, for diagnostics).
    pub verification: Option<Verification>,
    /// Re-plan signal on failure.
    pub replan: Option<ReplanSignal>,
}

/// Executes approved actions through an [`Adapter`].
pub struct Executor<A: Adapter, P: PolicyRecheck> {
    adapter: A,
    policy: P,
    cfg: Arc<WorldConfig>,
    exec: ExecConfig,
    ledger: BTreeMap<String, (u64, Lifecycle)>,
    order: u64,
    failures: BTreeMap<String, u32>,
}

impl<A: Adapter, P: PolicyRecheck> Executor<A, P> {
    /// New executor.
    pub fn new(adapter: A, policy: P, cfg: Arc<WorldConfig>, exec: ExecConfig) -> Self {
        Self {
            adapter,
            policy,
            cfg,
            exec,
            ledger: BTreeMap::new(),
            order: 0,
            failures: BTreeMap::new(),
        }
    }

    /// Lifecycle state of a decision, if known.
    pub fn lifecycle(&self, decision_id: &str) -> Option<Lifecycle> {
        self.ledger.get(decision_id).map(|(_, l)| *l)
    }

    /// The adapter (e.g. to observe outside an execution).
    pub fn adapter_mut(&mut self) -> &mut A {
        &mut self.adapter
    }

    /// Failed attempts recorded for an incident.
    pub fn failures_for(&self, incident_id: &str) -> u32 {
        self.failures.get(incident_id).copied().unwrap_or(0)
    }

    fn set(&mut self, id: &str, l: Lifecycle) {
        self.order += 1;
        self.ledger.insert(id.to_string(), (self.order, l));
        if self.ledger.len() > self.exec.ledger_capacity {
            // Evict the oldest finished entry; never forget an in-flight one.
            let oldest = self
                .ledger
                .iter()
                .filter(|(_, (_, l))| !matches!(l, Lifecycle::Executing | Lifecycle::Executed))
                .min_by_key(|(_, (o, _))| *o)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                self.ledger.remove(&k);
            }
        }
    }

    async fn observe(&mut self) -> Result<WorldObservation, AdapterError> {
        let t = self.exec.cmd_timeout;
        match tokio::time::timeout(t, self.adapter.observe()).await {
            Ok(r) => r,
            Err(_) => Err(AdapterError::Timeout(t.as_millis() as u64)),
        }
    }

    /// Executes `req`. Never panics on adapter or world failures; every
    /// outcome is in the report.
    pub async fn execute(&mut self, req: ExecutionRequest) -> ExecutionReport {
        let mut report = ExecutionReport {
            decision_id: req.decision_id.clone(),
            incident_id: req.incident_id.clone(),
            action: req.action.clone(),
            impact: None,
            plan: Vec::new(),
            commands: Vec::new(),
            state: ExecState::Refused,
            refusal: None,
            pre_observation: None,
            expected: None,
            verification: None,
            replan: None,
        };
        let refuse = |mut r: ExecutionReport, why: Refusal| {
            tracing::warn!(decision = %r.decision_id, reason = %why, "execution refused");
            r.refusal = Some(why);
            r.state = ExecState::Refused;
            r
        };

        if self.ledger.contains_key(&req.decision_id) {
            return refuse(report, Refusal::Duplicate);
        }
        let plan = match commands_for_action(&req.action) {
            Ok(p) => p,
            Err(e) => return refuse(report, e.into()),
        };
        report.plan = plan.clone();
        match authorize(&req.action, req.target_criticality, &req.authorization) {
            Ok(class) => report.impact = Some(class),
            Err(e) => return refuse(report, Refusal::Unauthorized(e)),
        }
        let pre = match self.observe().await {
            Ok(o) => o,
            Err(e) => {
                return refuse(
                    report,
                    Refusal::ObserveFailed {
                        reason: e.to_string(),
                    },
                );
            }
        };
        report.pre_observation = Some(pre.clone());
        if let Err(v) = self.policy.recheck(&req.action, &plan, &pre) {
            return refuse(report, Refusal::PolicyVeto(v));
        }
        let topology = match expected_topology(&self.cfg, &pre, &plan) {
            Ok(t) => t,
            Err(e) => {
                return refuse(
                    report,
                    Refusal::Invalid {
                        reason: e.to_string(),
                    },
                );
            }
        };
        let expected = ExpectedState {
            topology,
            capabilities: req.expected_capabilities,
        };
        report.expected = Some(expected.clone());

        // ---- point of no return: commands leave from here ----
        self.set(&req.decision_id, Lifecycle::Executing);
        tracing::info!(decision = %req.decision_id, action = %req.action.display_name(), commands = plan.len(), "executing");
        let mut failure: Option<(ExecState, String)> = None;
        let mut applied = 0usize;
        for op in &plan {
            let t = self.exec.cmd_timeout;
            let res = match tokio::time::timeout(t, self.adapter.submit(&req.decision_id, op)).await
            {
                Ok(r) => r,
                Err(_) => Err(AdapterError::Timeout(t.as_millis() as u64)),
            };
            match res {
                Ok(ack) => {
                    let ok = ack.status.is_success();
                    report.commands.push(CommandRecord {
                        op: op.clone(),
                        status: Some(ack.status),
                        detail: ack.reason.clone(),
                        command_id: Some(ack.command_id.clone()),
                    });
                    if ok {
                        applied += 1;
                    } else {
                        let st = if applied > 0 {
                            ExecState::PartiallyExecuted
                        } else {
                            ExecState::CommandFailed
                        };
                        failure = Some((st, format!("{op:?} -> {:?}: {}", ack.status, ack.reason)));
                        break;
                    }
                }
                Err(e) => {
                    report.commands.push(CommandRecord {
                        op: op.clone(),
                        status: None,
                        detail: e.to_string(),
                        command_id: None,
                    });
                    let st = match (&e, applied) {
                        (AdapterError::Timeout(_), _) => ExecState::TimedOut,
                        (_, 0) => ExecState::CommandFailed,
                        _ => ExecState::PartiallyExecuted,
                    };
                    failure = Some((st, format!("{op:?}: {e}")));
                    break;
                }
            }
        }

        if failure.is_none() {
            self.set(&req.decision_id, Lifecycle::Executed);
        }
        // Verify (or, after a failure, record what the world actually looks like).
        let deadline = tokio::time::Instant::now() + self.exec.verify_window;
        loop {
            match self.observe().await {
                Ok(obs) => match verify(
                    &self.cfg,
                    &expected,
                    &obs,
                    self.exec.epsilon,
                    &req.node_trust,
                ) {
                    Ok(v) => {
                        let ok = v.ok;
                        report.verification = Some(v);
                        if ok || failure.is_some() {
                            break;
                        }
                    }
                    Err(e) => {
                        failure.get_or_insert((ExecState::VerificationFailed, e.to_string()));
                        break;
                    }
                },
                Err(e) => {
                    if failure.is_none() && tokio::time::Instant::now() >= deadline {
                        failure = Some((ExecState::VerificationFailed, format!("observe: {e}")));
                        break;
                    }
                    if failure.is_some() {
                        break;
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(self.exec.verify_poll).await;
        }

        let verified = failure.is_none() && report.verification.as_ref().is_some_and(|v| v.ok);
        if verified {
            report.state = ExecState::Verified;
            self.set(&req.decision_id, Lifecycle::Verified);
            self.failures.remove(&req.incident_id);
            tracing::info!(decision = %req.decision_id, "verified");
            return report;
        }
        let (state, reason) = failure.unwrap_or_else(|| {
            let detail = report
                .verification
                .as_ref()
                .map(|v| {
                    v.failures()
                        .map(|c| {
                            format!(
                                "{} (expected {}, observed {})",
                                c.name, c.expected, c.observed
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_else(|| "no observation".into());
            (
                ExecState::VerificationFailed,
                format!("verification failed: {detail}"),
            )
        });
        report.state = state;
        self.set(&req.decision_id, Lifecycle::Failed);
        let attempt = {
            let c = self.failures.entry(req.incident_id.clone()).or_insert(0);
            *c = c.saturating_add(1);
            *c
        };
        let escalate = attempt >= self.exec.max_replans;
        tracing::warn!(decision = %req.decision_id, ?state, %reason, attempt, escalate, "execution failed; replan");
        report.replan = Some(ReplanSignal {
            decision_id: req.decision_id,
            incident_id: req.incident_id,
            reason,
            attempt,
            escalate_to_human: escalate,
        });
        report
    }
}
