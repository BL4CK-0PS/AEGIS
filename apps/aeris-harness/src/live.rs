//! `aeris-harness live`: the closed loop against a **running** stack.
//!
//! ```text
//! aeris-simd run --scenario s4 --seed 7 --key-dir deploy/keys     (terminal 1)
//! aeris-harness live --key-dir deploy/keys                        (terminal 2)
//! ```
//!
//! Events come from JetStream (published and signed by `aeris-simd`), are
//! verified by `aeris-crypto`, fed to `aeris-core` every 100 ms; a proposal
//! that needs a human **waits for a person to type the approval** on stdin
//! (or is never executed). Approved actions go through `aeris-exec` over the
//! signed NATS command channel and are verified against the observed world.
//! Every executed or refused decision is appended to the signed audit chain.
//!
//! This is the harness standing in for `aeris-server` + console (owner:
//! Kesivarshini / Kishanth), which do not exist yet. It decides nothing:
//! decisions come from `aeris-core`, authorization from a human or the gate.

use crate::nats_ingest::{self, Ingested};
use aeris_core::CoreState;
use aeris_crypto::audit::{AuditChain, AuditEntry};
use aeris_crypto::ingest::EventVerifier;
use aeris_crypto::keys::{KeyRegistry, SigningIdentity, ids};
use aeris_crypto::replay::ReplayConfig;
use aeris_exec::nats::NatsSimAdapter;
use aeris_exec::{
    Approval, Authorization, ExecConfig, ExecState, ExecutionRequest, Executor, Role,
    TopologyGuard, Verdict,
};
use aeris_simd::{NatsAuth, NatsSettings, StreamSettings};
use aeris_types::{Action, Gate, VerifiedEvent};
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// How HUMAN_REVIEW proposals are answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Approver {
    /// Ask the person at the terminal. Anything but `approve` rejects.
    Interactive {
        /// Operator id recorded in the approval.
        operator: String,
    },
    /// Never approve (observe-only).
    Never,
}

/// `live` settings.
#[derive(Debug, Clone)]
pub struct LiveConfig {
    /// NATS URL, user and password of the **server** (control-plane) user.
    pub nats_url: String,
    /// NATS credentials.
    pub auth: NatsAuth,
    /// Directory with `public-keys.toml` and `server.key`.
    pub key_dir: PathBuf,
    /// Stop after this many seconds of wall time.
    pub duration_s: f64,
    /// Seed handed to `aeris-core` (rollout streams).
    pub seed: u64,
    /// Approval source.
    pub approver: Approver,
    /// Write the audit chain as JSONL here.
    pub audit_out: Option<PathBuf>,
}

fn node_ids(reg: &KeyRegistry) -> Vec<String> {
    reg.ids()
        .filter(|i| i.starts_with("node-"))
        .map(str::to_string)
        .collect()
}

fn ask(question: &str) -> Option<String> {
    print!("{question}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_string()),
    }
}

/// Runs the live loop. Returns the audit entries written.
pub async fn run_live(cfg: LiveConfig) -> Result<Vec<AuditEntry>, String> {
    let reg_path = cfg.key_dir.join("public-keys.toml");
    let reg_text =
        std::fs::read_to_string(&reg_path).map_err(|e| format!("{}: {e}", reg_path.display()))?;
    let registry = KeyRegistry::from_toml(&reg_text).map_err(|e| e.to_string())?;
    let nodes = node_ids(&registry);
    let mut node_reg = KeyRegistry::new();
    for n in &nodes {
        node_reg
            .insert(n, *registry.get(n).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }
    let sim_key = *registry
        .get(ids::SIM_CONTROLLER)
        .map_err(|e| e.to_string())?;
    let server = SigningIdentity::load_secret_file("server", &cfg.key_dir.join("server.key"))
        .map_err(|e| format!("server key: {e}"))?;
    let audit_signer = SigningIdentity::load_secret_file("server", &cfg.key_dir.join("server.key"))
        .map_err(|e| format!("server key: {e}"))?;

    let settings = NatsSettings {
        url: cfg.nats_url.clone(),
        auth: cfg.auth.clone(),
        tls_ca: None,
        connect_attempts: 5,
    };
    let client = aeris_simd::connect(&settings, "aeris-harness-live")
        .await
        .map_err(|e| e.to_string())?;
    aeris_simd::ensure_stream(
        &async_nats::jetstream::new(client.clone()),
        &StreamSettings::default(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let verifier = Arc::new(std::sync::Mutex::new(EventVerifier::new(
        node_reg,
        ReplayConfig::default(),
    )));
    let mut rx = nats_ingest::spawn_shared(client.clone(), Arc::clone(&verifier)).await?;
    let world = Arc::new(aeris_sim::WorldConfig::frozen().map_err(|e| e.to_string())?);
    let mut exec = Executor::new(
        NatsSimAdapter::new(client, server, sim_key, Duration::from_secs(3)),
        TopologyGuard::new(Arc::clone(&world)),
        Arc::clone(&world),
        ExecConfig::default(),
    );
    let interactive_ok = std::io::stdin().is_terminal();
    println!(
        "AERIS live: consuming signed events from {} (nodes {}). Approval: {}. Start aeris-simd now if it is not running.",
        cfg.nats_url,
        nodes.join(","),
        match (&cfg.approver, interactive_ok) {
            (Approver::Interactive { .. }, true) => "typed by the operator at this terminal",
            (Approver::Interactive { .. }, false) =>
                "stdin is not a terminal: piped input is accepted, EOF rejects",
            (Approver::Never, _) => "never (observe only)",
        }
    );

    let mut core = CoreState::new();
    let mut chain = AuditChain::new();
    let mut entries = Vec::new();
    let mut admitted = 0u64;
    let mut rejected = 0u64;
    let mut decided: std::collections::BTreeSet<String> = Default::default();
    let mut seen_incidents: std::collections::BTreeSet<String> = Default::default();
    let started = tokio::time::Instant::now();
    let mut last_status = started;
    while started.elapsed() < Duration::from_secs_f64(cfg.duration_s) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut batch: Vec<VerifiedEvent> = Vec::new();
        while let Ok(x) = rx.try_recv() {
            match x {
                Ingested::Admitted(a) => {
                    admitted += 1;
                    batch.push(a.to_verified_event());
                }
                Ingested::Rejected(r) => {
                    rejected += 1;
                    if rejected <= 5 || rejected.is_multiple_of(500) {
                        println!(
                            "  REJECTED event #{rejected} from {:?}: {:?}",
                            r.claimed_source, r.reason
                        );
                    }
                }
            }
        }
        let out = aeris_core::step(&mut core, &batch, cfg.seed);
        let t = started.elapsed().as_secs_f64();
        if last_status.elapsed() >= Duration::from_secs(5) {
            last_status = tokio::time::Instant::now();
            let trust: Vec<String> = nodes
                .iter()
                .filter_map(|n| {
                    core.trust_registry
                        .node_trust(n)
                        .map(|v| format!("{n}={v:.3}"))
                })
                .collect();
            println!(
                "[{t:5.1}s] events admitted {admitted} rejected {rejected} | mode {:?} | trust {} | mission {:.3}",
                out.mode,
                trust.join(" "),
                out.mission_score
            );
        }
        for inc in &out.incidents {
            if seen_incidents.insert(inc.id.clone()) {
                println!(
                    "[{t:5.1}s] INCIDENT {} on {} | risk {:.3} | P(compromise) {:.3} | evidence {}",
                    inc.id,
                    inc.entity,
                    inc.risk,
                    inc.posterior.compromise,
                    inc.evidence_ids.len()
                );
            }
        }
        let Some(p) = out.proposal else { continue };
        if !decided.insert(p.incident_id.clone()) {
            continue;
        }
        println!(
            "[{t:5.1}s] PROPOSAL {} for {} — candidates (500 Monte Carlo rollouts each):",
            p.id, p.incident_id
        );
        println!(
            "  {:<44} {:>9} {:>7} {:>8} {:>8}  invariants",
            "action", "P(succ)", "±se", "CVaR", "J(a)"
        );
        for c in &p.candidates {
            let v = if c.violations.is_empty() {
                "ok".to_string()
            } else {
                c.violations
                    .iter()
                    .map(|v| format!("VETO {v}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            println!(
                "  {:<44} {:>9.3} {:>7.3} {:>8.3} {:>8.3}  {}",
                c.action.display_name(),
                c.p_success,
                c.se,
                c.cvar,
                c.score,
                v
            );
        }
        println!(
            "  RECOMMENDED: {} | gate {:?}",
            p.recommended_action.display_name(),
            p.gate
        );
        println!("  WHY: {}", p.explanation);

        let auth = match (&p.gate, &cfg.approver) {
            (Gate::Auto, _) => Some(Authorization::Auto { gate: Gate::Auto }),
            (Gate::Reject { .. }, _) | (_, Approver::Never) => None,
            (Gate::HumanReview { reason }, Approver::Interactive { operator }) => {
                println!("  HUMAN REVIEW REQUIRED: {reason}");
                // Blocking read off the async runtime; events keep queueing
                // in the ingest channel and are verified on receipt.
                let q = format!(
                    "  Type 'approve' to execute \"{}\" as commander {operator}, anything else rejects: ",
                    p.recommended_action.display_name()
                );
                let answer = tokio::task::spawn_blocking(move || ask(&q))
                    .await
                    .map_err(|e| e.to_string())?;
                let approved = answer.as_deref() == Some("approve");
                Some(Authorization::Human(Approval {
                    operator: operator.clone(),
                    role: Role::Commander,
                    verdict: if approved {
                        Verdict::Approve
                    } else {
                        Verdict::Reject
                    },
                    approved_action: p.recommended_action.clone(),
                    reason: if approved {
                        "approved at the aeris-harness live terminal".into()
                    } else {
                        format!("rejected at the terminal (answer {answer:?})")
                    },
                    at_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0),
                }))
            }
        };
        let Some(auth) = auth else {
            println!("  NOT EXECUTED: no authorization (gate {:?})", p.gate);
            continue;
        };
        let target = target_of(&p.recommended_action).unwrap_or_default();
        let req = ExecutionRequest {
            decision_id: p.id.clone(),
            incident_id: p.incident_id.clone(),
            action: p.recommended_action.clone(),
            authorization: auth.clone(),
            target_criticality: if target.is_empty() {
                0.0
            } else {
                core.compute_criticality(&target)
            },
            expected_capabilities: None,
            node_trust: nodes
                .iter()
                .filter_map(|n| core.trust_registry.node_trust(n).map(|v| (n.clone(), v)))
                .collect(),
        };
        let report = exec.execute(req).await;
        for c in &report.commands {
            println!("  COMMAND {:?} -> {:?} {}", c.op, c.status, c.detail);
        }
        match &report.verification {
            Some(v) => println!(
                "  VERIFY ‖e‖ = {:.4} -> {} | B mode {:?} | L2 active {:?}",
                v.error_norm,
                if v.ok { "OK" } else { "FAILED" },
                v.observed.node_mode("node-b"),
                v.observed.link("link-l2").map(|l| l.active)
            ),
            None => println!("  VERIFY: not reached"),
        }
        println!(
            "  EXECUTION STATE {:?}{}",
            report.state,
            report
                .refusal
                .as_ref()
                .map(|r| format!(" (refused: {r})"))
                .unwrap_or_default()
        );
        if report.state == ExecState::Verified {
            core.apply_action(&p.recommended_action);
            // Stop accepting an isolated node's key (security audit §3.4).
            if let Some(v) = &report.verification {
                for n in &nodes {
                    if v.observed.node_mode(n) == Some(aeris_wire::NodeMode::Isolated) {
                        if let Ok(mut g) = verifier.lock() {
                            g.keys_mut().revoke(n);
                        }
                        println!("  REVOKED {n}'s event key: its events are now rejected");
                    }
                }
            }
        } else {
            // Let core propose again for this incident.
            decided.remove(&p.incident_id);
        }
        let record = serde_json::json!({
            "decision_id": p.id,
            "incident_id": p.incident_id,
            "target": target,
            "source": "aeris-harness live (NATS)",
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
            .append_record(&audit_signer, &p.id, &record)
            .map_err(|e| e.to_string())?;
        println!("  AUDIT n={} hash {}", e.n, hex_short(&e.hash));
        entries.push(e);
    }
    println!(
        "AERIS live done: admitted {admitted}, rejected {rejected}, decisions audited {}",
        entries.len()
    );
    if let Some(p) = &cfg.audit_out {
        let text = aeris_crypto::audit::to_jsonl(&entries).map_err(|e| e.to_string())?;
        std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(entries)
}

fn hex_short(h: &[u8; 32]) -> String {
    h.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn target_of(a: &Action) -> Option<String> {
    match a {
        Action::Isolate(n) | Action::Restrict(n) | Action::Recover(n) => Some(n.clone()),
        Action::Failover { from, .. } => Some(from.clone()),
        Action::Combo(v) => v.iter().find_map(target_of),
        _ => None,
    }
}
