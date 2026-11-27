//! Executor behaviour against the in-process simulator, including injected
//! failures (refused commands, acks without effect, hangs, transport errors).

use aeris_exec::*;
use aeris_sim::{Scenario, World, WorldConfig};
use aeris_types::{Action as A, Gate, ImpactClass, Invariant};
use aeris_wire::{CommandAck, CommandOp, NodeMode, WorldObservation};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn cfg() -> Arc<WorldConfig> {
    Arc::new(WorldConfig::frozen().unwrap())
}

fn shared(scenario_toml: Option<&str>, ticks: u64) -> SharedWorld {
    let c = cfg();
    let s = match scenario_toml {
        Some(t) => Scenario::from_toml_str(t, &c).unwrap(),
        None => Scenario::builtin("s4", &c).unwrap(),
    };
    let mut w = World::new(c, Arc::new(s), 7).unwrap();
    for _ in 0..ticks {
        w.step();
    }
    Arc::new(Mutex::new(w))
}

fn combo() -> A {
    A::Combo(vec![
        A::Isolate("node-b".into()),
        A::Reroute("link-l2".into()),
    ])
}

fn commander(action: &A) -> Authorization {
    Authorization::Human(Approval {
        operator: "op-1".into(),
        role: Role::Commander,
        verdict: Verdict::Approve,
        approved_action: action.clone(),
        reason: "test approval".into(),
        at_ms: 1,
    })
}

fn operator(action: &A) -> Authorization {
    match commander(action) {
        Authorization::Human(mut a) => {
            a.role = Role::Operator;
            Authorization::Human(a)
        }
        other => other,
    }
}

fn req(id: &str, action: A, auth: Authorization) -> ExecutionRequest {
    ExecutionRequest {
        decision_id: id.into(),
        incident_id: "inc-1".into(),
        action,
        authorization: auth,
        target_criticality: 1.0,
        expected_capabilities: None,
        node_trust: Vec::new(),
    }
}

fn exec_cfg() -> ExecConfig {
    ExecConfig {
        cmd_timeout: Duration::from_millis(500),
        verify_window: Duration::from_millis(300),
        verify_poll: Duration::from_millis(50),
        ..ExecConfig::default()
    }
}

fn executor(w: &SharedWorld) -> Executor<LocalSimAdapter, TopologyGuard> {
    Executor::new(
        LocalSimAdapter::new(w.clone()),
        TopologyGuard::new(cfg()),
        cfg(),
        exec_cfg(),
    )
}

fn obs(w: &SharedWorld) -> WorldObservation {
    w.lock().unwrap().observe()
}

const ACTUATOR: &str =
    "[scenario]\nid = \"act\"\nname = \"n\"\ndescription = \"d\"\nduration_s = 60.0\n";

fn actuator(target: &str, behavior: &str) -> String {
    format!(
        "{ACTUATOR}[[faults]]\nkind = \"actuator_fault\"\ntarget = \"{target}\"\nbehavior = \"{behavior}\"\nstart_s = 0.0\n"
    )
}

// ---------------------------------------------------------------------------
// Success paths
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn isolate_and_reroute_is_verified_under_s4() {
    let w = shared(None, 305);
    assert!(!w.lock().unwrap().mission_ok());
    let mut ex = executor(&w);
    let r = ex.execute(req("dec-1", combo(), commander(&combo()))).await;
    assert_eq!(r.state, ExecState::Verified, "{r:#?}");
    assert_eq!(r.impact, Some(ImpactClass::High));
    // Γ order: path first, isolation second.
    assert_eq!(
        r.plan,
        vec![
            CommandOp::ActivateLink {
                link: "link-l2".into()
            },
            CommandOp::Isolate {
                node: "node-b".into()
            }
        ]
    );
    assert!(
        r.commands
            .iter()
            .all(|c| c.status.is_some_and(|s| s.is_success()))
    );
    let v = r.verification.unwrap();
    assert!(v.ok && v.error_norm == 0.0);
    assert_eq!(v.observed.node_mode("node-b"), Some(NodeMode::Isolated));
    assert!(r.replan.is_none());
    assert_eq!(ex.lifecycle("dec-1"), Some(Lifecycle::Verified));
    assert!(w.lock().unwrap().mission_ok(), "true mission recovered");
}

#[tokio::test(start_paused = true)]
async fn reroute_is_verified_with_operator_approval() {
    let w = shared(None, 10);
    let a = A::Reroute("link-l2".into());
    let r = executor(&w)
        .execute(req("dec-r", a.clone(), operator(&a)))
        .await;
    assert_eq!(r.state, ExecState::Verified, "{r:#?}");
    assert_eq!(r.impact, Some(ImpactClass::Medium));
    assert!(obs(&w).link("link-l2").unwrap().active);
}

#[tokio::test(start_paused = true)]
async fn isolation_is_verified_when_a_backup_path_exists() {
    let w = shared(None, 10);
    w.lock()
        .unwrap()
        .apply_command(&CommandOp::ActivateLink {
            link: "link-l2".into(),
        })
        .unwrap();
    let a = A::Isolate("node-b".into());
    let r = executor(&w)
        .execute(req("dec-i", a.clone(), commander(&a)))
        .await;
    assert_eq!(r.state, ExecState::Verified, "{r:#?}");
    assert_eq!(obs(&w).node_mode("node-b"), Some(NodeMode::Isolated));
}

#[tokio::test(start_paused = true)]
async fn low_impact_action_may_run_on_auto_gate() {
    let w = shared(None, 10);
    let a = A::Restrict("node-b".into());
    let r = executor(&w)
        .execute(req("dec-a", a, Authorization::Auto { gate: Gate::Auto }))
        .await;
    assert_eq!(r.state, ExecState::Verified, "{r:#?}");
    assert_eq!(obs(&w).node_mode("node-b"), Some(NodeMode::Restricted));
}

// ---------------------------------------------------------------------------
// Authorization: nothing leaves without it
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn unauthorized_requests_send_nothing() {
    let w = shared(None, 305);
    let before = obs(&w);
    let mut ex = executor(&w);
    let other = A::Reroute("link-l2".into());
    let mut rejected = commander(&combo());
    if let Authorization::Human(a) = &mut rejected {
        a.verdict = Verdict::Reject;
    }
    let cases = vec![
        (
            "auto gate for HIGH",
            Authorization::Auto { gate: Gate::Auto },
        ),
        (
            "auto with review gate",
            Authorization::Auto {
                gate: Gate::HumanReview { reason: "x".into() },
            },
        ),
        (
            "gate rejected",
            Authorization::Auto {
                gate: Gate::Reject {
                    invariant: Invariant::I1_NoSeverLastCommPath,
                },
            },
        ),
        ("operator approving HIGH", operator(&combo())),
        ("approval for another action", commander(&other)),
        ("human rejected", rejected),
    ];
    for (i, (name, auth)) in cases.into_iter().enumerate() {
        let r = ex.execute(req(&format!("dec-u{i}"), combo(), auth)).await;
        assert_eq!(r.state, ExecState::Refused, "{name}");
        assert!(
            matches!(r.refusal, Some(Refusal::Unauthorized(_))),
            "{name}: {:?}",
            r.refusal
        );
        assert!(r.commands.is_empty(), "{name}: commands were sent");
        assert!(r.replan.is_none());
    }
    // Non-finite criticality must not make an isolation look LOW impact.
    let mut nan = req(
        "dec-nan",
        A::Isolate("node-b".into()),
        Authorization::Auto { gate: Gate::Auto },
    );
    nan.target_criticality = f64::NAN;
    assert_eq!(ex.execute(nan).await.state, ExecState::Refused);
    assert_eq!(obs(&w).nodes, before.nodes);
    assert_eq!(
        obs(&w).links.iter().map(|l| l.active).collect::<Vec<_>>(),
        before.links.iter().map(|l| l.active).collect::<Vec<_>>()
    );
}

#[tokio::test(start_paused = true)]
async fn policy_veto_blocks_isolating_the_last_relay() {
    let w = shared(None, 305);
    let a = A::Isolate("node-b".into());
    let r = executor(&w)
        .execute(req("dec-v", a.clone(), commander(&a)))
        .await;
    assert_eq!(r.state, ExecState::Refused);
    match r.refusal {
        Some(Refusal::PolicyVeto(v)) => {
            assert_eq!(v.invariant, Some(Invariant::I1_NoSeverLastCommPath))
        }
        other => panic!("{other:?}"),
    }
    assert!(r.commands.is_empty());
    assert_eq!(obs(&w).node_mode("node-b"), Some(NodeMode::Active));
}

#[tokio::test(start_paused = true)]
async fn invalid_actions_are_refused() {
    let w = shared(None, 10);
    let mut ex = executor(&w);
    let nested = A::Combo(vec![A::Combo(vec![A::Monitor])]);
    let r = ex
        .execute(req("dec-n", nested.clone(), commander(&nested)))
        .await;
    assert!(matches!(r.refusal, Some(Refusal::Invalid { .. })));
    let unknown = A::Reroute("link-l9".into());
    let r = ex
        .execute(req("dec-l9", unknown.clone(), commander(&unknown)))
        .await;
    assert!(
        matches!(r.refusal, Some(Refusal::PolicyVeto(_))),
        "{:?}",
        r.refusal
    );
    assert!(r.commands.is_empty());
}

#[tokio::test(start_paused = true)]
async fn duplicate_execution_is_rejected() {
    let w = shared(None, 305);
    let mut ex = executor(&w);
    let first = ex.execute(req("dec-d", combo(), commander(&combo()))).await;
    assert_eq!(first.state, ExecState::Verified);
    // Undo out of band, then replay the same decision: must not re-run.
    w.lock()
        .unwrap()
        .apply_command(&CommandOp::Recover {
            node: "node-b".into(),
        })
        .unwrap();
    let second = ex.execute(req("dec-d", combo(), commander(&combo()))).await;
    assert_eq!(second.state, ExecState::Refused);
    assert_eq!(second.refusal, Some(Refusal::Duplicate));
    assert!(second.commands.is_empty());
    assert_eq!(obs(&w).node_mode("node-b"), Some(NodeMode::Active));
}

// ---------------------------------------------------------------------------
// Failures after commands were sent
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn refused_first_command_is_a_command_failure() {
    let w = shared(Some(&actuator("node-c", "refuse")), 5);
    w.lock()
        .unwrap()
        .apply_command(&CommandOp::ActivateLink {
            link: "link-l2".into(),
        })
        .unwrap();
    let a = A::Restrict("node-c".into());
    let r = executor(&w)
        .execute(req("dec-f", a.clone(), commander(&a)))
        .await;
    assert_eq!(r.state, ExecState::CommandFailed, "{r:#?}");
    assert_eq!(r.commands.len(), 1);
    assert_eq!(r.commands[0].status, Some(aeris_wire::AckStatus::Failed));
    let replan = r.replan.unwrap();
    assert_eq!(replan.attempt, 1);
    assert!(!replan.escalate_to_human);
    assert!(r.verification.is_some(), "actual state is still recorded");
}

#[tokio::test(start_paused = true)]
async fn partial_execution_is_reported() {
    // L2 activates, then isolating B is refused by the actuator.
    let w = shared(Some(&actuator("node-b", "refuse")), 5);
    let r = executor(&w)
        .execute(req("dec-p", combo(), commander(&combo())))
        .await;
    assert_eq!(r.state, ExecState::PartiallyExecuted, "{r:#?}");
    assert_eq!(r.commands.len(), 2);
    assert!(r.commands[0].status.unwrap().is_success());
    assert!(!r.commands[1].status.unwrap().is_success());
    let v = r.verification.unwrap();
    assert!(!v.ok);
    assert!(v.failures().any(|c| c.name.starts_with("node-b.mode")));
    assert!(
        obs(&w).link("link-l2").unwrap().active,
        "first command did apply"
    );
    assert!(r.replan.is_some());
}

#[tokio::test(start_paused = true)]
async fn ack_without_effect_fails_verification() {
    let w = shared(Some(&actuator("link-l2", "ack_no_effect")), 5);
    let a = A::Reroute("link-l2".into());
    let r = executor(&w)
        .execute(req("dec-x", a.clone(), operator(&a)))
        .await;
    assert_eq!(r.state, ExecState::VerificationFailed, "{r:#?}");
    assert!(r.commands[0].status.unwrap().is_success(), "the ack lied");
    let v = r.verification.unwrap();
    assert!(v.failures().any(|c| c.name == "link-l2.active=true"));
    assert!(r.replan.unwrap().reason.contains("link-l2.active"));
}

#[tokio::test(start_paused = true)]
async fn soft_error_beyond_epsilon_fails_verification() {
    let w = shared(None, 10);
    let a = A::Reroute("link-l2".into());
    let mut rq = req("dec-e", a.clone(), operator(&a));
    rq.expected_capabilities = Some(aeris_sim::Capabilities {
        navigation: 0.93,
        communication: 0.50, // the model predicted something very different
        observation: 0.92,
    });
    let r = executor(&w).execute(rq).await;
    assert_eq!(r.state, ExecState::VerificationFailed);
    let v = r.verification.unwrap();
    assert!(v.soft_norm > v.epsilon);
    assert!(v.error_norm >= v.soft_norm);
}

#[tokio::test(start_paused = true)]
async fn replans_escalate_to_a_human_after_three_failures() {
    let w = shared(Some(&actuator("link-l2", "ack_no_effect")), 5);
    let mut ex = executor(&w);
    let a = A::Reroute("link-l2".into());
    for attempt in 1..=3 {
        let r = ex
            .execute(req(&format!("dec-{attempt}"), a.clone(), operator(&a)))
            .await;
        let s = r.replan.unwrap();
        assert_eq!(s.attempt, attempt);
        assert_eq!(s.escalate_to_human, attempt == 3);
    }
    assert_eq!(ex.failures_for("inc-1"), 3);
}

/// Wraps the local adapter and misbehaves on request.
struct Flaky {
    inner: LocalSimAdapter,
    hang_submit: bool,
    transport_error_submit_after: Option<usize>,
    fail_observe: bool,
    submits: usize,
}

impl Adapter for Flaky {
    fn submit(
        &mut self,
        decision_id: &str,
        op: &CommandOp,
    ) -> impl Future<Output = Result<CommandAck, AdapterError>> + Send {
        self.submits += 1;
        let n = self.submits;
        let hang = self.hang_submit;
        let err = self.transport_error_submit_after.is_some_and(|k| n > k);
        let res = if hang || err {
            None
        } else {
            Some(self.inner.submit(decision_id, op))
        };
        async move {
            if hang {
                std::future::pending::<()>().await;
            }
            if err {
                return Err(AdapterError::Transport("connection reset".into()));
            }
            res.expect("set above").await
        }
    }

    fn observe(&mut self) -> impl Future<Output = Result<WorldObservation, AdapterError>> + Send {
        let fail = self.fail_observe;
        let f = self.inner.observe();
        async move {
            if fail {
                Err(AdapterError::Transport("no responders".into()))
            } else {
                f.await
            }
        }
    }
}

fn flaky(w: &SharedWorld) -> Flaky {
    Flaky {
        inner: LocalSimAdapter::new(w.clone()),
        hang_submit: false,
        transport_error_submit_after: None,
        fail_observe: false,
        submits: 0,
    }
}

#[tokio::test(start_paused = true)]
async fn command_timeout_is_reported_not_assumed() {
    let w = shared(None, 305);
    let mut ad = flaky(&w);
    ad.hang_submit = true;
    let mut ex = Executor::new(ad, TopologyGuard::new(cfg()), cfg(), exec_cfg());
    let r = ex.execute(req("dec-t", combo(), commander(&combo()))).await;
    assert_eq!(r.state, ExecState::TimedOut, "{r:#?}");
    assert_eq!(
        r.commands.len(),
        1,
        "stops at the first unacknowledged command"
    );
    assert!(r.commands[0].status.is_none());
    assert!(r.commands[0].detail.contains("timed out"));
    assert!(r.replan.is_some());
    assert_eq!(ex.lifecycle("dec-t"), Some(Lifecycle::Failed));
}

#[tokio::test(start_paused = true)]
async fn transport_error_mid_plan_is_partial() {
    let w = shared(None, 305);
    let mut ad = flaky(&w);
    ad.transport_error_submit_after = Some(1);
    let mut ex = Executor::new(ad, TopologyGuard::new(cfg()), cfg(), exec_cfg());
    let r = ex
        .execute(req("dec-tr", combo(), commander(&combo())))
        .await;
    assert_eq!(r.state, ExecState::PartiallyExecuted, "{r:#?}");
    assert!(r.commands[1].detail.contains("connection reset"));
}

#[tokio::test(start_paused = true)]
async fn unobservable_world_refuses_before_sending() {
    let w = shared(None, 305);
    let mut ad = flaky(&w);
    ad.fail_observe = true;
    let mut ex = Executor::new(ad, TopologyGuard::new(cfg()), cfg(), exec_cfg());
    let r = ex.execute(req("dec-o", combo(), commander(&combo()))).await;
    assert!(matches!(r.refusal, Some(Refusal::ObserveFailed { .. })));
    assert!(r.commands.is_empty());
    assert_eq!(obs(&w).node_mode("node-b"), Some(NodeMode::Active));
}

#[tokio::test(start_paused = true)]
async fn monitor_sends_nothing_and_is_judged_on_the_mission() {
    // Monitor under an active compromise: nothing to send, mission still
    // below minimum, so verification fails and a replan is requested.
    let w = shared(None, 305);
    let mut rq = req(
        "dec-m",
        A::Monitor,
        Authorization::Auto { gate: Gate::Auto },
    );
    // AERIS distrusts B (as the pipeline would after S4 evidence).
    rq.node_trust = vec![("node-b".into(), 0.5)];
    let r = executor(&w).execute(rq).await;
    assert!(r.plan.is_empty() && r.commands.is_empty());
    assert_eq!(r.state, ExecState::VerificationFailed);
    assert!(r.replan.is_some());
}

#[tokio::test(start_paused = true)]
async fn verification_uses_trust_not_ground_truth() {
    // Same compromised world: without a trust input the topology looks fine,
    // which is exactly why the pipeline's trust must be supplied.
    let w = shared(None, 305);
    let r = executor(&w)
        .execute(req(
            "dec-m2",
            A::Monitor,
            Authorization::Auto { gate: Gate::Auto },
        ))
        .await;
    assert_eq!(r.state, ExecState::Verified);
    assert!(
        !w.lock().unwrap().mission_ok(),
        "verification cannot see ground truth; it judges AERIS's own estimate"
    );
}
