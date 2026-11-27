//! G2 gate (docs/v0.1/07): S4 integration test across 20 seeds with AI off.
//!
//! What is asserted is *structure*, never a number from the spec:
//! * an incident opens on the attacked node after injection;
//! * the executed action is exactly the optimizer's recommendation, and that
//!   recommendation is the argmax of J among candidates passing the
//!   invariants and the chance constraint (or forced to human review);
//! * a HIGH-impact action ran only with a (test) commander approval;
//! * execution verified against the observed world and the audit chain verifies.
//!
//! Mission success is *measured* and printed, not asserted per seed; the
//! G2 pass condition is the loop above working on every seed.

use aeris_harness::closed_loop::{ApprovalPolicy, RunConfig, Strategy, run};
use aeris_harness::sweep::{aggregate, classify};
use std::sync::Arc;

fn cfg(seed: u64, strategy: Strategy, approve: bool) -> RunConfig {
    let w = Arc::new(aeris_sim::WorldConfig::frozen().unwrap());
    RunConfig {
        scenario: Arc::new(aeris_sim::Scenario::builtin("s4", &w).unwrap()),
        world: w,
        seed,
        strategy,
        approval: ApprovalPolicy {
            test_commander_approves: approve,
            delay_ticks: 0,
        },
        narrate: false,
        server_key_file: None,
    }
}

#[test]
fn g2_s4_twenty_seeds_ai_off() {
    let mut results = Vec::new();
    for seed in 1..=20 {
        let (r, _) = run(&cfg(seed, Strategy::Aeris, true)).unwrap();
        let d = classify(&r);
        assert_eq!(d.tp, 1, "seed {seed}: compromise of node-b detected: {r:?}");
        let (tick, action, gate) = r.first_proposal.clone().expect("a proposal");
        assert!(tick >= r.inject_tick.unwrap());
        assert_eq!(
            gate, "HUMAN_REVIEW",
            "seed {seed}: HIGH impact needs a human"
        );
        let first = r.executions.first().expect("approved action executed");
        assert_eq!(first.action, action, "seed {seed}: executed == recommended");
        assert_eq!(first.authorization, "human-test");
        assert_eq!(r.unapproved_non_low_executions, 0);
        assert_eq!(
            first.state,
            aeris_exec::ExecState::Verified,
            "seed {seed}: verification: {r:?}"
        );
        assert!(r.audit_ok && r.audit_records >= 1);
        assert_eq!(r.events_rejected, 0, "no genuine event rejected");
        results.push(r);
    }
    let a = &aggregate(&results)[0];
    println!(
        "G2 S4 x20 (AI off): MSR {:.1}% | recommendations {:?} | detect p95 {:?}s | decision latency p50/p95 {:?}/{:?} ms",
        a.msr() * 100.0,
        a.recommendations,
        a.detect_p95_s,
        a.latency_p50_ms,
        a.latency_p95_ms
    );
}

#[test]
fn without_approval_high_impact_never_runs() {
    // Incidents stay open for the whole run (core re-plans every tick), so
    // this is the slowest test; two seeds suffice to show the property.
    for seed in 1..=2 {
        let (r, _) = run(&cfg(seed, Strategy::Aeris, false)).unwrap();
        assert!(r.first_proposal.is_some());
        assert!(
            r.executions.is_empty(),
            "seed {seed}: nothing may execute without approval: {:?}",
            r.executions
        );
        assert_eq!(r.unapproved_non_low_executions, 0);
    }
}

#[test]
fn identical_seed_gives_identical_run() {
    let (a, _) = run(&cfg(9, Strategy::Aeris, true)).unwrap();
    let (b, _) = run(&cfg(9, Strategy::Aeris, true)).unwrap();
    // Latency is wall time; everything else must match exactly.
    let strip = |mut r: aeris_harness::closed_loop::RunResult| {
        r.decision_latency_ms = None;
        serde_json::to_string(&r).unwrap()
    };
    assert_eq!(strip(a), strip(b));
}
