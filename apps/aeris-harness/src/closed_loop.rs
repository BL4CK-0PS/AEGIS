//! In-process closed loop over the real components:
//!
//! ```text
//! aeris-sim World ─observations─▶ EventSigner (Ed25519, seq, S7 buffering)
//!   ─envelope bytes─▶ EventVerifier (exact-byte verify, replay window)
//!   ─VerifiedEvent─▶ aeris-core::step (detect, fuse, trust, risk, rollouts, optimizer, gate)
//!   ─Proposal─▶ approval (gate AUTO, or an *explicitly enabled* test approver)
//!   ─ExecutionRequest─▶ aeris-exec (authorize, policy re-check, Γ(a), LocalSimAdapter, verify)
//!   ─report─▶ BLAKE3 audit chain
//! ```
//!
//! Only transport is in-process (no NATS); every byte is still signed and
//! verified. Ground truth is read only for metrics, never fed back.

use aeris_core::{CoreState, NodeStatus};
use aeris_crypto::audit::{AuditChain, AuditEntry, verify_chain};
use aeris_crypto::ingest::{EventSigner, EventVerifier, IntegrityLedger};
use aeris_crypto::keys::{KeyRegistry, SigningIdentity};
use aeris_crypto::replay::{ReplayConfig, SequenceCounter};
use aeris_exec::{
    Approval, Authorization, ExecConfig, ExecState, ExecutionRequest, Executor, LocalSimAdapter,
    Role, TopologyGuard, Verdict,
};
use aeris_sim::{Delivery, Scenario, World, WorldConfig};
use aeris_types::{Action, Gate, ImpactClass, IncidentStatus, Proposal, VerifiedEvent};
use aeris_wire::{CommandOp, HlcClock, NodeMode, Provenance, WireEvent, WorldObservation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Sim clock epoch for event timestamps (2027-01-01T00:00:00Z).
pub const EPOCH_MS: i64 = 1_798_761_600_000;

/// What responds to incidents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    /// The full AERIS loop.
    Aeris,
    /// Baseline: raise alerts, never act.
    AlertOnly,
    /// Baseline: isolate the first alerted node, never reroute (bypasses policy).
    Naive,
}

impl std::fmt::Display for Strategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Strategy::Aeris => "aeris",
            Strategy::AlertOnly => "alert-only",
            Strategy::Naive => "naive",
        })
    }
}

/// How HUMAN_REVIEW proposals are answered in a headless run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    /// `true` = an explicitly configured *test* commander approves the
    /// recommended action after `delay_ticks`. `false` = nobody approves, so
    /// non-LOW actions never run.
    pub test_commander_approves: bool,
    /// Simulated human reaction time.
    pub delay_ticks: u64,
}

/// One run.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// World.
    pub world: Arc<WorldConfig>,
    /// Scenario.
    pub scenario: Arc<Scenario>,
    /// Seed (world and pipeline).
    pub seed: u64,
    /// Responder.
    pub strategy: Strategy,
    /// Approval behaviour.
    pub approval: ApprovalPolicy,
    /// Print a narrated trace.
    pub narrate: bool,
    /// Server secret key file for signing audit records; `None` = a fresh
    /// ephemeral key (the chain then verifies only within the run).
    pub server_key_file: Option<std::path::PathBuf>,
}

/// One execution in a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecSummary {
    /// Tick executed.
    pub tick: u64,
    /// Decision id.
    pub decision_id: String,
    /// Action.
    pub action: String,
    /// Final state.
    pub state: ExecState,
    /// Impact class.
    pub impact: Option<ImpactClass>,
    /// `auto` or `human-test`.
    pub authorization: String,
    /// Executor asked to replan.
    pub replan: bool,
}

/// Measured outcome of one run. Every field is computed, none asserted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    /// Scenario id.
    pub scenario: String,
    /// Seed.
    pub seed: u64,
    /// Strategy.
    pub strategy: Strategy,
    /// First fault start tick (ground truth).
    pub inject_tick: Option<u64>,
    /// Primary target (ground truth): the node the scenario compromises, if
    /// any, else the node of the first node-targeted fault. Precision/recall
    /// are scored against it.
    pub fault_target: Option<String>,
    /// Every node a scenario fault targets (ground truth). Incidents on these
    /// after injection are not false positives (e.g. S6's GPS fault on A).
    #[serde(default)]
    pub fault_targets: Vec<String>,
    /// `(tick, entity)` of every incident opened.
    pub incidents: Vec<(u64, String)>,
    /// First proposal: tick, recommended action, gate.
    pub first_proposal: Option<(u64, String, String)>,
    /// Wall time of the core cycle that produced the first proposal, ms.
    pub decision_latency_ms: Option<f64>,
    /// Simulated success probability of the first recommendation.
    pub recommended_p_success: Option<f64>,
    /// Executions.
    pub executions: Vec<ExecSummary>,
    /// Tick of the first verified execution.
    pub verified_tick: Option<u64>,
    /// First tick after injection from which the mission stayed OK for 5 s.
    pub recovered_tick: Option<u64>,
    /// Fraction of ticks with every capability ≥ minimum in the final 10 s.
    pub final_window_ok: f64,
    /// Fraction of ticks ≥ minimum after injection (whole run if none).
    pub post_inject_ok: f64,
    /// Mission success (see `success` docs in the test report).
    pub success: bool,
    /// The compromise spread to another node (ground truth).
    pub lateral_spread: bool,
    /// Events admitted by the verifier.
    pub events_admitted: u64,
    /// Of which replayed (S7 buffers).
    pub events_replayed: u64,
    /// Events rejected by the verifier.
    pub events_rejected: u64,
    /// Non-LOW actions executed without a human approval. Must be 0.
    pub unapproved_non_low_executions: u64,
    /// Escalated to a human after repeated failures.
    pub escalated: bool,
    /// Audit chain records and verification result.
    pub audit_records: u64,
    /// Chain verified end to end.
    pub audit_ok: bool,
}

struct Pending {
    node: usize,
    ev: WireEvent,
}

/// Runs one scenario × seed × strategy. Returns the metrics and the audit chain.
pub fn run(cfg: &RunConfig) -> Result<(RunResult, Vec<AuditEntry>), String> {
    let world = World::new(Arc::clone(&cfg.world), Arc::clone(&cfg.scenario), cfg.seed)
        .map_err(|e| e.to_string())?;
    let shared = Arc::new(Mutex::new(world));
    let lock = || {
        shared
            .lock()
            .map_err(|_| "world mutex poisoned".to_string())
    };

    // Identities: fresh per run (keys never leave this process).
    let nodes: Vec<String> = cfg.world.nodes.iter().map(|n| n.id.clone()).collect();
    let mut reg = KeyRegistry::new();
    let mut signers = Vec::new();
    for n in &nodes {
        let id = SigningIdentity::generate(n.clone()).map_err(|e| e.to_string())?;
        reg.insert(n, id.verifying_key())
            .map_err(|e| e.to_string())?;
        signers.push(EventSigner::new(id));
    }
    let server = match &cfg.server_key_file {
        Some(p) => SigningIdentity::load_secret_file("server", p).map_err(|e| e.to_string())?,
        None => SigningIdentity::generate("server").map_err(|e| e.to_string())?,
    };
    let mut verifier = EventVerifier::new(reg, ReplayConfig::default());
    let mut ledger = IntegrityLedger::default();
    let mut seq: Vec<SequenceCounter> = vec![SequenceCounter::new(); nodes.len()];
    let mut hlc: Vec<HlcClock> = vec![HlcClock::new(2_000); nodes.len()];
    let mut buffers: Vec<VecDeque<Pending>> = (0..nodes.len()).map(|_| VecDeque::new()).collect();
    let mut id_rng = aeris_sim::rng::stream(cfg.seed, "harness-event-ids", 0);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|e| e.to_string())?;
    let exec_cfg = ExecConfig {
        // The world only advances between ticks, so one observation decides.
        verify_window: Duration::ZERO,
        ..ExecConfig::default()
    };
    let mut executor = Executor::new(
        LocalSimAdapter::new(Arc::clone(&shared)),
        TopologyGuard::new(Arc::clone(&cfg.world)),
        Arc::clone(&cfg.world),
        exec_cfg,
    );
    let mut core = CoreState::new();
    let mut chain = AuditChain::new();
    let mut entries = Vec::new();

    let hz = cfg.world.world.tick_hz;
    let duration = cfg.scenario.duration_ticks(hz);
    let first_fault = cfg
        .scenario
        .faults
        .iter()
        .map(|f| {
            (
                (f.window().0 * f64::from(hz)).round() as u64,
                f.target_node().map(str::to_string),
            )
        })
        .min_by_key(|(t, _)| *t);
    let inject_tick = first_fault.as_ref().map(|(t, _)| *t);
    let compromised_by_scenario: Vec<String> = cfg
        .scenario
        .faults
        .iter()
        .filter_map(|f| match f {
            aeris_sim::Fault::Compromise { node, .. } => Some(node.clone()),
            _ => None,
        })
        .collect();
    let fault_target = compromised_by_scenario.first().cloned().or_else(|| {
        cfg.scenario
            .faults
            .iter()
            .find_map(|f| f.target_node().map(str::to_string))
    });
    let mut fault_targets: Vec<String> = cfg
        .scenario
        .faults
        .iter()
        .filter_map(|f| f.target_node().map(str::to_string))
        .collect();
    fault_targets.sort();
    fault_targets.dedup();

    let mut res = RunResult {
        scenario: cfg.scenario.scenario.id.clone(),
        seed: cfg.seed,
        strategy: cfg.strategy,
        inject_tick,
        fault_target: fault_target.clone(),
        fault_targets,
        incidents: Vec::new(),
        first_proposal: None,
        decision_latency_ms: None,
        recommended_p_success: None,
        executions: Vec::new(),
        verified_tick: None,
        recovered_tick: None,
        final_window_ok: 0.0,
        post_inject_ok: 0.0,
        success: false,
        lateral_spread: false,
        events_admitted: 0,
        events_replayed: 0,
        events_rejected: 0,
        unapproved_non_low_executions: 0,
        escalated: false,
        audit_records: 0,
        audit_ok: false,
    };
    let mut ok_trace: Vec<bool> = Vec::with_capacity(duration as usize);
    let mut seen_incidents: BTreeMap<String, ()> = BTreeMap::new();
    let mut scheduled: Option<(u64, Proposal)> = None;
    let mut in_flight_incident: Option<String> = None;
    let mut escalated_incidents: BTreeMap<String, ()> = BTreeMap::new();
    // Incidents whose response verified: core may keep proposing for them
    // until its own verification stage closes them; those are not new decisions.
    let mut resolved_incidents: BTreeMap<String, ()> = BTreeMap::new();
    let mut naive_done = false;

    for _ in 0..duration {
        // ---- world tick and the signed event path ----
        let (obs, tick) = {
            let mut w = lock()?;
            let o = w.step();
            (o, w.tick())
        };
        let now = EPOCH_MS + tick as i64 * 100;
        let mut envelopes: Vec<Vec<u8>> = Vec::new();
        for o in obs {
            if o.delivery == Delivery::Quarantined {
                continue;
            }
            let node = nodes
                .iter()
                .position(|n| *n == o.node)
                .ok_or("unknown node")?;
            let s = seq[node].advance().map_err(|e| e.to_string())?;
            let h = hlc[node].send(now).map_err(|e| e.to_string())?;
            let mut rnd = [0u8; 10];
            rand::RngCore::fill_bytes(&mut id_rng, &mut rnd);
            let ev = WireEvent {
                id: uuid_v7(now, &rnd),
                source: o.node.clone(),
                seq: s,
                hlc: h,
                t_unix_ms: now,
                replayed: o.delivery == Delivery::Partitioned,
                confidence: o.confidence,
                provenance: Provenance {
                    producer: o.producer.clone(),
                    producer_version: aeris_sim::world::PRODUCER_VERSION.into(),
                },
                payload: o.payload.clone(),
            };
            if o.delivery == Delivery::Partitioned {
                buffers[node].push_back(Pending { node, ev });
                continue;
            }
            while let Some(p) = buffers[node].pop_front() {
                envelopes.push(signers[p.node].seal(&p.ev).map_err(|e| e.to_string())?);
            }
            envelopes.push(signers[node].seal(&ev).map_err(|e| e.to_string())?);
        }
        let mut events: Vec<VerifiedEvent> = Vec::with_capacity(envelopes.len());
        for env in &envelopes {
            match verifier.admit(env, now) {
                Ok(a) => {
                    res.events_admitted += 1;
                    if a.event.replayed {
                        res.events_replayed += 1;
                    }
                    events.push(a.to_verified_event());
                }
                Err(r) => {
                    res.events_rejected += 1;
                    ledger.record(&r, verifier.keys());
                }
            }
        }

        // ---- intelligence + decision (aeris-core) ----
        let t0 = Instant::now();
        let out = aeris_core::step(&mut core, &events, cfg.seed);
        let cycle_ms = t0.elapsed().as_secs_f64() * 1000.0;
        for inc in &out.incidents {
            if seen_incidents.insert(inc.id.clone(), ()).is_none() {
                res.incidents.push((tick, inc.entity.clone()));
                if cfg.narrate {
                    println!(
                        "[{:5.1}s] INCIDENT {} on {} (risk {:.3}, P(compromise) {:.3})",
                        tick as f64 / 10.0,
                        inc.id,
                        inc.entity,
                        inc.risk,
                        inc.posterior.compromise
                    );
                }
            }
        }
        if let Some(p) = &out.proposal
            && res.first_proposal.is_none()
        {
            res.first_proposal = Some((
                tick,
                p.recommended_action.display_name(),
                gate_name(&p.gate),
            ));
            res.decision_latency_ms = Some(cycle_ms);
            res.recommended_p_success = p
                .candidates
                .iter()
                .find(|c| c.action == p.recommended_action)
                .map(|c| c.p_success);
            if cfg.narrate {
                println!(
                    "[{:5.1}s] PROPOSAL {} gate={}",
                    tick as f64 / 10.0,
                    p.id,
                    gate_name(&p.gate)
                );
                for c in &p.candidates {
                    println!(
                        "          {:42} P(success)={:.3} se={:.3} CVaR={:.3} J={:+.3} {}",
                        c.action.display_name(),
                        c.p_success,
                        c.se,
                        c.cvar,
                        c.score,
                        if c.violations.is_empty() {
                            String::new()
                        } else {
                            format!("VETO {:?}", c.violations)
                        }
                    );
                }
                println!(
                    "          RECOMMENDED: {}",
                    p.recommended_action.display_name()
                );
                println!("          {}", p.explanation);
            }
        }

        // ---- response ----
        match cfg.strategy {
            Strategy::AlertOnly => {}
            Strategy::Naive => {
                if !naive_done
                    && let Some(inc) = out
                        .incidents
                        .iter()
                        .find(|i| i.status == IncidentStatus::Open)
                {
                    naive_done = true;
                    let mut w = lock()?;
                    let _ = w.apply_command(&CommandOp::Isolate {
                        node: inc.entity.clone(),
                    });
                    let o = w.observe();
                    drop(w);
                    sync_core_topology(&mut core, &o);
                }
            }
            Strategy::Aeris => {
                if in_flight_incident.is_none()
                    && let Some(p) = &out.proposal
                    && !escalated_incidents.contains_key(&p.incident_id)
                    && !resolved_incidents.contains_key(&p.incident_id)
                {
                    match &p.gate {
                        Gate::Auto => scheduled = Some((tick, p.clone())),
                        Gate::HumanReview { .. } if cfg.approval.test_commander_approves => {
                            scheduled = Some((tick + cfg.approval.delay_ticks, p.clone()));
                        }
                        _ => {}
                    }
                    if scheduled.is_some() {
                        in_flight_incident = Some(p.incident_id.clone());
                    }
                }
                if let Some((due, p)) = scheduled.clone()
                    && tick >= due
                {
                    scheduled = None;
                    let auth = match &p.gate {
                        Gate::Auto => Authorization::Auto { gate: Gate::Auto },
                        _ => Authorization::Human(Approval {
                            operator: "harness-test-commander".into(),
                            role: Role::Commander,
                            verdict: Verdict::Approve,
                            approved_action: p.recommended_action.clone(),
                            reason: "explicitly configured test approval (aeris-harness --approve)"
                                .into(),
                            at_ms: now,
                        }),
                    };
                    let target = action_target(&p.recommended_action).unwrap_or_default();
                    let crit = if target.is_empty() {
                        0.0
                    } else {
                        core.compute_criticality(&target)
                    };
                    let trust: Vec<(String, f64)> = nodes
                        .iter()
                        .filter_map(|n| core.trust_registry.node_trust(n).map(|t| (n.clone(), t)))
                        .collect();
                    let req = ExecutionRequest {
                        decision_id: p.id.clone(),
                        incident_id: p.incident_id.clone(),
                        action: p.recommended_action.clone(),
                        authorization: auth.clone(),
                        target_criticality: crit,
                        expected_capabilities: None,
                        node_trust: trust,
                    };
                    let report = rt.block_on(executor.execute(req));
                    let human = matches!(auth, Authorization::Human(_));
                    if report.state != ExecState::Refused
                        && report.impact.is_some_and(|c| c != ImpactClass::Low)
                        && !human
                    {
                        res.unapproved_non_low_executions += 1;
                    }
                    if cfg.narrate {
                        println!(
                            "[{:5.1}s] EXECUTE {} by {} -> {:?}",
                            tick as f64 / 10.0,
                            p.recommended_action.display_name(),
                            if human { "test commander" } else { "auto gate" },
                            report.state
                        );
                    }
                    res.executions.push(ExecSummary {
                        tick,
                        decision_id: p.id.clone(),
                        action: p.recommended_action.display_name(),
                        state: report.state,
                        impact: report.impact,
                        authorization: if human {
                            "human-test".into()
                        } else {
                            "auto".into()
                        },
                        replan: report.replan.is_some(),
                    });
                    // Audit record (§6.15) for every executed or refused decision.
                    let record = serde_json::json!({
                        "decision_id": p.id,
                        "incident_id": p.incident_id,
                        "target": target,
                        "tick": tick,
                        "seed": cfg.seed,
                        "scenario": cfg.scenario.scenario.id,
                        "recommended_action": p.recommended_action,
                        "gate": p.gate,
                        "candidates": p.candidates,
                        "authorization": auth,
                        "execution": {
                            "state": report.state,
                            "impact": report.impact,
                            "commands": report.commands,
                            "refusal": report.refusal,
                            "verification_ok": report.verification.as_ref().map(|v| v.ok),
                            "error_norm": report.verification.as_ref().map(|v| v.error_norm),
                        },
                        "model_ver": concat!("aeris-core/", env!("CARGO_PKG_VERSION")),
                        "policy_ver": "aeris-exec TopologyGuard (interim; aeris-policy pending)",
                    });
                    let e = chain
                        .append_record(&server, &p.id, &record)
                        .map_err(|e| e.to_string())?;
                    entries.push(e);
                    let post = lock()?.observe();
                    match report.state {
                        ExecState::Verified => {
                            core.apply_action(&p.recommended_action);
                            if let Action::Combo(_) | Action::Isolate(_) = &p.recommended_action {
                                for (n, m) in &post.nodes {
                                    if *m == NodeMode::Isolated {
                                        verifier.keys_mut().revoke(n);
                                    }
                                }
                            }
                            res.verified_tick.get_or_insert(tick);
                            resolved_incidents.insert(p.incident_id.clone(), ());
                            in_flight_incident = None;
                        }
                        _ => {
                            sync_core_topology(&mut core, &post);
                            if report.replan.as_ref().is_some_and(|r| r.escalate_to_human) {
                                escalated_incidents.insert(p.incident_id.clone(), ());
                                res.escalated = true;
                            }
                            in_flight_incident = None;
                        }
                    }
                }
            }
        }

        // ---- ground truth (metrics only) ----
        let gt = lock()?.ground_truth();
        ok_trace.push(gt.mission_ok);
        // Lateral spread: a node compromised that the scenario did not
        // compromise itself (S6 compromises only B; A's GPS fault is not one).
        let target_compromised = |n: &str| compromised_by_scenario.iter().any(|c| c == n);
        if gt
            .nodes
            .iter()
            .any(|n| n.compromised && !target_compromised(&n.id))
        {
            res.lateral_spread = true;
        }
    }

    // ---- metrics ----
    let n = ok_trace.len();
    let window = n.min(100);
    res.final_window_ok =
        ok_trace[n - window..].iter().filter(|x| **x).count() as f64 / window.max(1) as f64;
    let from = inject_tick.unwrap_or(0) as usize;
    let post = &ok_trace[from.min(n)..];
    res.post_inject_ok = post.iter().filter(|x| **x).count() as f64 / post.len().max(1) as f64;
    // Recovered: first tick after injection starting 50 consecutive OK ticks.
    let mut run_len = 0usize;
    for (i, ok) in ok_trace.iter().enumerate().skip(from) {
        run_len = if *ok { run_len + 1 } else { 0 };
        if run_len == 50 {
            res.recovered_tick = Some((i + 1 - 50) as u64 + 1);
            break;
        }
    }
    res.success = res.final_window_ok >= 0.9;
    let report = verify_chain(&entries, &server.verifying_key(), Some(chain.checkpoint()));
    res.audit_records = entries.len() as u64;
    res.audit_ok = report.ok;
    let _ = ledger.drain_evidence();
    Ok((res, entries))
}

fn gate_name(g: &Gate) -> String {
    match g {
        Gate::Auto => "AUTO".into(),
        Gate::HumanReview { .. } => "HUMAN_REVIEW".into(),
        Gate::Reject { .. } => "REJECT".into(),
    }
}

fn action_target(a: &Action) -> Option<String> {
    match a {
        Action::Isolate(n) | Action::Restrict(n) | Action::Recover(n) => Some(n.clone()),
        Action::Failover { from, .. } => Some(from.clone()),
        Action::Combo(v) => v.iter().find_map(action_target),
        _ => None,
    }
}

/// Mirrors an observation into core's topology model (used when an execution
/// did not verify, so core plans from the state that actually exists).
fn sync_core_topology(core: &mut CoreState, o: &WorldObservation) {
    for (n, m) in &o.nodes {
        let st = match m {
            NodeMode::Isolated => NodeStatus::Isolated,
            NodeMode::Restricted => NodeStatus::Restricted,
            _ => match core.nodes.get(n) {
                Some(NodeStatus::Isolated | NodeStatus::Restricted) | None => NodeStatus::Healthy,
                Some(other) => other.clone(),
            },
        };
        core.nodes.insert(n.clone(), st);
    }
    for l in &o.links {
        if let Some(cl) = core.links.get_mut(&l.link) {
            cl.active = l.active;
        }
    }
}

fn uuid_v7(ms: i64, rnd: &[u8; 10]) -> String {
    uuid::Builder::from_unix_timestamp_millis(ms.max(0) as u64, rnd)
        .into_uuid()
        .hyphenated()
        .to_string()
}
