//! Scenario × seed × strategy sweeps and their metrics (§6.16).
//!
//! Definitions (also in `docs/audit/dharshan-techgium-readiness.md`):
//! * **Mission success** of a run: every capability meets its minimum on at
//!   least 90 % of the ticks in the final 10 s (ground truth).
//! * **TP**: an incident on the primary target (the compromised node if the
//!   scenario compromises one, else the faulted node) opened within 5 s of
//!   injection. **FP**: an incident in a run with no fault, on a node no
//!   fault targets, or before injection. **FN**: a primary target with no TP.
//!   Link, partition and model-mismatch faults (S3, S7, S8) have no target
//!   node; their incidents are reported but not classified.
//! * **Detection delay**: TP incident tick − injection tick.
//! * **Decision latency**: wall time of the core cycle that produced the
//!   first proposal (AI off; includes the Monte Carlo rollouts).
//! * **Recovery time**: first tick after injection that starts 5 s of
//!   continuous mission OK − injection tick.

use crate::closed_loop::{RunResult, Strategy};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Detection classification of one run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detection {
    /// True positives.
    pub tp: u32,
    /// False positives.
    pub fp: u32,
    /// False negatives.
    pub fn_: u32,
    /// Delay of the TP, ticks.
    pub delay_ticks: Option<u64>,
}

/// Classifies one run's incidents.
pub fn classify(r: &RunResult) -> Detection {
    let mut d = Detection::default();
    let window = 50; // 5 s at 10 Hz
    match (&r.fault_target, r.inject_tick) {
        (Some(target), Some(inject)) => {
            for (tick, entity) in &r.incidents {
                let is = |t: &str| entity == t || entity.starts_with(&format!("{t}/"));
                let on_target = is(target);
                let on_any_fault = on_target || r.fault_targets.iter().any(|t| is(t));
                if on_target && *tick >= inject && *tick <= inject + window && d.tp == 0 {
                    d.tp = 1;
                    d.delay_ticks = Some(tick - inject);
                } else if !on_any_fault || *tick < inject {
                    d.fp += 1;
                }
            }
            if d.tp == 0 {
                d.fn_ = 1;
            }
        }
        (None, None) => d.fp = r.incidents.len() as u32,
        _ => {}
    }
    d
}

fn pct(v: &mut [f64], p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let idx = ((p * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1;
    Some(v[idx])
}

/// Aggregate for one (scenario, strategy).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Aggregate {
    /// Scenario.
    pub scenario: String,
    /// Strategy.
    pub strategy: String,
    /// Runs.
    pub runs: u32,
    /// Mission successes.
    pub successes: u32,
    /// TP/FP/FN totals.
    pub tp: u32,
    /// FP total.
    pub fp: u32,
    /// FN total.
    pub fn_: u32,
    /// Detection delay p50/p95, s.
    pub detect_p50_s: Option<f64>,
    /// p95.
    pub detect_p95_s: Option<f64>,
    /// Decision latency p50/p95, ms.
    pub latency_p50_ms: Option<f64>,
    /// p95.
    pub latency_p95_ms: Option<f64>,
    /// Recovery time p50/p95, s.
    pub recovery_p50_s: Option<f64>,
    /// p95.
    pub recovery_p95_s: Option<f64>,
    /// Runs with ≥1 verified execution.
    pub verified_runs: u32,
    /// Runs with any failed execution.
    pub failed_exec_runs: u32,
    /// Runs escalated to a human.
    pub escalated_runs: u32,
    /// Runs where the compromise spread.
    pub lateral_runs: u32,
    /// Sum of non-LOW executions without human approval (must be 0).
    pub unapproved_non_low: u64,
    /// Runs whose audit chain verified.
    pub audit_ok_runs: u32,
    /// Events rejected by the verifier, total.
    pub events_rejected: u64,
    /// Replayed (S7) events admitted, total.
    pub events_replayed: u64,
    /// First recommendation → count.
    pub recommendations: BTreeMap<String, u32>,
}

impl Aggregate {
    /// Mission success rate.
    pub fn msr(&self) -> f64 {
        if self.runs == 0 {
            0.0
        } else {
            f64::from(self.successes) / f64::from(self.runs)
        }
    }
    /// Precision (None when undefined).
    pub fn precision(&self) -> Option<f64> {
        let d = self.tp + self.fp;
        (d > 0).then(|| f64::from(self.tp) / f64::from(d))
    }
    /// Recall (None when undefined).
    pub fn recall(&self) -> Option<f64> {
        let d = self.tp + self.fn_;
        (d > 0).then(|| f64::from(self.tp) / f64::from(d))
    }
    /// F1 (None when undefined).
    pub fn f1(&self) -> Option<f64> {
        match (self.precision(), self.recall()) {
            (Some(p), Some(r)) if p + r > 0.0 => Some(2.0 * p * r / (p + r)),
            (Some(_), Some(_)) => Some(0.0),
            _ => None,
        }
    }
}

/// Aggregates runs grouped by (scenario, strategy), in input order.
pub fn aggregate(results: &[RunResult]) -> Vec<Aggregate> {
    let mut keys: Vec<(String, Strategy)> = Vec::new();
    for r in results {
        let k = (r.scenario.clone(), r.strategy);
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    keys.into_iter()
        .map(|(sc, st)| {
            let rs: Vec<&RunResult> = results
                .iter()
                .filter(|r| r.scenario == sc && r.strategy == st)
                .collect();
            let mut a = Aggregate {
                scenario: sc.clone(),
                strategy: st.to_string(),
                ..Default::default()
            };
            let (mut det, mut lat, mut rec) = (Vec::new(), Vec::new(), Vec::new());
            for r in rs {
                a.runs += 1;
                a.successes += u32::from(r.success);
                let d = classify(r);
                a.tp += d.tp;
                a.fp += d.fp;
                a.fn_ += d.fn_;
                if let Some(t) = d.delay_ticks {
                    det.push(t as f64 / 10.0);
                }
                if let Some(l) = r.decision_latency_ms {
                    lat.push(l);
                }
                if let (Some(rt), Some(it)) = (r.recovered_tick, r.inject_tick) {
                    rec.push(rt.saturating_sub(it) as f64 / 10.0);
                }
                a.verified_runs += u32::from(r.verified_tick.is_some());
                a.failed_exec_runs += u32::from(
                    r.executions
                        .iter()
                        .any(|e| e.state != aeris_exec::ExecState::Verified),
                );
                a.escalated_runs += u32::from(r.escalated);
                a.lateral_runs += u32::from(r.lateral_spread);
                a.unapproved_non_low += r.unapproved_non_low_executions;
                a.audit_ok_runs += u32::from(r.audit_ok);
                a.events_rejected += r.events_rejected;
                a.events_replayed += r.events_replayed;
                if let Some((_, act, _)) = &r.first_proposal {
                    *a.recommendations.entry(act.clone()).or_default() += 1;
                }
            }
            a.detect_p50_s = pct(&mut det, 0.5);
            a.detect_p95_s = pct(&mut det, 0.95);
            a.latency_p50_ms = pct(&mut lat, 0.5);
            a.latency_p95_ms = pct(&mut lat, 0.95);
            a.recovery_p50_s = pct(&mut rec, 0.5);
            a.recovery_p95_s = pct(&mut rec, 0.95);
            a
        })
        .collect()
}

fn f(v: Option<f64>, digits: usize) -> String {
    v.map_or("n/a".into(), |x| format!("{x:.digits$}"))
}

/// One CSV row per run.
pub fn runs_csv(results: &[RunResult]) -> String {
    let mut s = String::from(
        "scenario,strategy,seed,success,final_window_ok,post_inject_ok,inject_tick,first_incident_tick,first_incident_entity,tp,fp,fn,detect_delay_s,recommended,gate,rec_p_success,decision_latency_ms,exec_states,verified_tick,recovered_tick,lateral_spread,events_admitted,events_replayed,events_rejected,unapproved_non_low,audit_records,audit_ok\n",
    );
    for r in results {
        let d = classify(r);
        let (fi_t, fi_e) = r
            .incidents
            .first()
            .map_or((String::new(), String::new()), |(t, e)| {
                (t.to_string(), e.clone())
            });
        let (rec, gate) = r
            .first_proposal
            .as_ref()
            .map_or((String::new(), String::new()), |(_, a, g)| {
                (a.clone(), g.clone())
            });
        let states: Vec<String> = r
            .executions
            .iter()
            .map(|e| format!("{:?}", e.state))
            .collect();
        s.push_str(&format!(
            "{},{},{},{},{:.3},{:.3},{},{},{},{},{},{},{},\"{}\",{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            r.scenario,
            r.strategy,
            r.seed,
            r.success,
            r.final_window_ok,
            r.post_inject_ok,
            r.inject_tick.map_or(String::new(), |t| t.to_string()),
            fi_t,
            fi_e,
            d.tp,
            d.fp,
            d.fn_,
            d.delay_ticks.map_or(String::new(), |t| format!("{:.1}", t as f64 / 10.0)),
            rec,
            gate,
            r.recommended_p_success.map_or(String::new(), |p| format!("{p:.3}")),
            r.decision_latency_ms.map_or(String::new(), |l| format!("{l:.2}")),
            states.join("|"),
            r.verified_tick.map_or(String::new(), |t| t.to_string()),
            r.recovered_tick.map_or(String::new(), |t| t.to_string()),
            r.lateral_spread,
            r.events_admitted,
            r.events_replayed,
            r.events_rejected,
            r.unapproved_non_low_executions,
            r.audit_records,
            r.audit_ok,
        ));
    }
    s
}

/// Markdown summary table.
pub fn markdown(aggs: &[Aggregate], header: &str) -> String {
    let mut s = format!(
        "{header}\n\n| Scenario | Strategy | Runs | MSR | Precision | Recall | F1 | Detect p50/p95 (s) | Decision latency p50/p95 (ms) | Recovery p50/p95 (s) | Verified runs | Failed-exec runs | Escalated | Lateral spread | Unapproved non-LOW | Audit OK | First recommendation |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n"
    );
    for a in aggs {
        let recs: Vec<String> = a
            .recommendations
            .iter()
            .map(|(k, v)| format!("{k} ×{v}"))
            .collect();
        s.push_str(&format!(
            "| {} | {} | {} | {:.1}% | {} | {} | {} | {} / {} | {} / {} | {} / {} | {} | {} | {} | {} | {} | {}/{} | {} |\n",
            a.scenario,
            a.strategy,
            a.runs,
            a.msr() * 100.0,
            f(a.precision(), 3),
            f(a.recall(), 3),
            f(a.f1(), 3),
            f(a.detect_p50_s, 1),
            f(a.detect_p95_s, 1),
            f(a.latency_p50_ms, 1),
            f(a.latency_p95_ms, 1),
            f(a.recovery_p50_s, 1),
            f(a.recovery_p95_s, 1),
            a.verified_runs,
            a.failed_exec_runs,
            a.escalated_runs,
            a.lateral_runs,
            a.unapproved_non_low,
            a.audit_ok_runs,
            a.runs,
            if recs.is_empty() { "none".into() } else { recs.join("; ") },
        ));
    }
    s
}
