//! End-to-end tests over a real NATS JetStream server.
//!
//! Ignored by default. Run against the compose stack:
//!
//! ```text
//! docker compose -f deploy/compose.yaml up -d --wait
//! set -a; . deploy/.env; set +a
//! AERIS_TEST_NATS_URL=nats://127.0.0.1:4222 \
//! AERIS_TEST_SIMD_USER=$NATS_SIMD_USER AERIS_TEST_SIMD_PASSWORD=$NATS_SIMD_PASSWORD \
//! AERIS_TEST_SERVER_USER=$NATS_SERVER_USER AERIS_TEST_SERVER_PASSWORD=$NATS_SERVER_PASSWORD \
//!   cargo test -p aeris-harness --test nats_e2e -- --ignored --test-threads=1 --nocapture
//! ```

use aeris_core::CoreState;
use aeris_crypto::ingest::{EventVerifier, RejectReason};
use aeris_crypto::keys::{Domain, KeyRegistry, SigningIdentity, ids};
use aeris_crypto::replay::{ReplayConfig, ReplayRejection};
use aeris_exec::nats::NatsSimAdapter;
use aeris_exec::{
    Adapter, Approval, Authorization, ExecConfig, ExecState, ExecutionRequest, Executor, Role,
    TopologyGuard, Verdict,
};
use aeris_harness::nats_ingest::{self, Ingested};
use aeris_simd::{NatsAuth, NatsSettings, SimdConfig, StreamSettings, Summary};
use aeris_types::{Gate, VerifiedEvent};
use aeris_wire::{AckStatus, CommandOp, NodeMode};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

struct Env {
    url: String,
    simd: NatsAuth,
    server: NatsAuth,
}

fn env() -> Option<Env> {
    let url = std::env::var("AERIS_TEST_NATS_URL").ok()?;
    let auth = |u: &str, p: &str| match (std::env::var(u), std::env::var(p)) {
        (Ok(user), Ok(password)) => NatsAuth::UserPassword { user, password },
        _ => NatsAuth::None,
    };
    Some(Env {
        url,
        simd: auth("AERIS_TEST_SIMD_USER", "AERIS_TEST_SIMD_PASSWORD"),
        server: auth("AERIS_TEST_SERVER_USER", "AERIS_TEST_SERVER_PASSWORD"),
    })
}

fn settings(url: &str, auth: NatsAuth) -> NatsSettings {
    NatsSettings {
        url: url.to_string(),
        auth,
        tls_ca: None,
        connect_attempts: 5,
    }
}

fn world() -> Arc<aeris_sim::WorldConfig> {
    Arc::new(aeris_sim::WorldConfig::frozen().unwrap())
}

struct Keys {
    _dir: tempfile::TempDir,
    dir: std::path::PathBuf,
    registry: KeyRegistry,
}

fn keys() -> Keys {
    let d = tempfile::tempdir().unwrap();
    let ids: Vec<String> = [
        "node-a",
        "node-b",
        "node-c",
        ids::SIM_CONTROLLER,
        ids::SERVER,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let registry = aeris_simd::keygen(d.path(), &ids).unwrap();
    Keys {
        dir: d.path().to_path_buf(),
        _dir: d,
        registry,
    }
}

fn simd_config(e: &Env, k: &Keys, scenario: &str, seed: u64, speed: f64, hold: bool) -> SimdConfig {
    let w = world();
    let (node_keys, sim_key, server_key) =
        aeris_simd::load_keys(&w, &k.dir, &k.dir.join("public-keys.toml")).unwrap();
    SimdConfig {
        scenario: Arc::new(aeris_sim::Scenario::builtin(scenario, &w).unwrap()),
        world: w,
        seed,
        nats: settings(&e.url, e.simd.clone()),
        stream: StreamSettings::default(),
        node_keys,
        sim_key,
        server_key,
        speed,
        max_buffer_per_node: 20_000,
        publish_timeout: Duration::from_secs(2),
        replay_after: Duration::from_secs(1),
        hold_after_end: hold,
        drain_timeout: Duration::from_secs(10),
    }
}

fn node_registry(k: &Keys) -> KeyRegistry {
    let mut r = KeyRegistry::new();
    for n in ["node-a", "node-b", "node-c"] {
        r.insert(n, *k.registry.get(n).unwrap()).unwrap();
    }
    r
}

async fn server_client(e: &Env) -> async_nats::Client {
    aeris_simd::connect(&settings(&e.url, e.server.clone()), "aeris-test-server")
        .await
        .unwrap()
}

async fn ingest(e: &Env, k: &Keys) -> mpsc::Receiver<Ingested> {
    let c = server_client(e).await;
    // The stream must exist before the consumer; simd creates it, but the
    // consumer is started first so it sees every event.
    let simd = aeris_simd::connect(&settings(&e.url, e.simd.clone()), "aeris-test-simd")
        .await
        .unwrap();
    aeris_simd::ensure_stream(
        &async_nats::jetstream::new(simd),
        &StreamSettings::default(),
    )
    .await
    .unwrap();
    nats_ingest::spawn(
        c,
        EventVerifier::new(node_registry(k), ReplayConfig::default()),
    )
    .await
    .unwrap()
}

/// Drains ingested results until `expected` admitted or `timeout`.
async fn collect(
    rx: &mut mpsc::Receiver<Ingested>,
    expected: u64,
    timeout: Duration,
) -> (
    Vec<aeris_crypto::ingest::Admitted>,
    Vec<aeris_crypto::ingest::Rejected>,
) {
    let (mut ok, mut bad) = (Vec::new(), Vec::new());
    let deadline = tokio::time::Instant::now() + timeout;
    while (ok.len() as u64) < expected {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(Ingested::Admitted(a))) => ok.push(*a),
            Ok(Some(Ingested::Rejected(r))) => bad.push(*r),
            _ => break,
        }
    }
    (ok, bad)
}

async fn run_simd(cfg: SimdConfig) -> (tokio::task::JoinHandle<Summary>, watch::Sender<bool>) {
    let (tx, rx) = watch::channel(false);
    let h = tokio::spawn(async move { aeris_simd::run(cfg, rx).await.unwrap() });
    (h, tx)
}

fn assert_strictly_increasing(admitted: &[aeris_crypto::ingest::Admitted]) {
    let mut last: BTreeMap<String, u64> = BTreeMap::new();
    for a in admitted {
        let l = last.entry(a.event.source.clone()).or_insert(0);
        assert!(
            a.event.seq > *l,
            "{} seq {} after {}",
            a.event.source,
            a.event.seq,
            l
        );
        *l = a.event.seq;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs NATS: set AERIS_TEST_NATS_URL"]
async fn simd_events_are_signed_acked_and_verified_over_jetstream() {
    let Some(e) = env() else { return };
    let k = keys();
    let mut rx = ingest(&e, &k).await;
    let (h, _stop) = run_simd(simd_config(&e, &k, "s1", 11, 10.0, false)).await;
    let s = h.await.unwrap();
    assert_eq!(s.produced, s.published, "{s:?}");
    assert_eq!(s.undelivered_at_shutdown, 0);
    let (ok, bad) = collect(&mut rx, s.published, Duration::from_secs(30)).await;
    assert_eq!(
        ok.len() as u64,
        s.published,
        "every published event admitted"
    );
    assert!(bad.is_empty(), "{bad:?}");
    assert_strictly_increasing(&ok);
    println!("S1 over NATS: {s:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs NATS: set AERIS_TEST_NATS_URL"]
async fn s7_partition_buffer_replays_once_over_jetstream() {
    let Some(e) = env() else { return };
    let k = keys();
    let mut rx = ingest(&e, &k).await;
    let (h, _stop) = run_simd(simd_config(&e, &k, "s7", 3, 10.0, false)).await;
    let s = h.await.unwrap();
    assert!(s.replayed > 900, "C's 20 s partition buffer: {s:?}");
    assert_eq!(s.produced, s.published);
    let (ok, bad) = collect(&mut rx, s.published, Duration::from_secs(30)).await;
    assert!(bad.is_empty(), "{bad:?}");
    assert_eq!(ok.len() as u64, s.published);
    let replayed: Vec<_> = ok.iter().filter(|a| a.event.replayed).collect();
    assert_eq!(replayed.len() as u64, s.replayed);
    assert!(replayed.iter().all(|a| a.event.source == "node-c"));
    // Admitted under the S7 rule even though older than 2 s.
    assert!(
        replayed
            .iter()
            .any(|a| a.admission == aeris_crypto::replay::Admission::Replayed)
    );
    assert_strictly_increasing(&ok);
    // Exactly once: no (source, seq) twice.
    let mut seen = std::collections::BTreeSet::new();
    for a in &ok {
        assert!(seen.insert((a.event.source.clone(), a.event.seq)));
    }
    println!("S7 over NATS: {s:?}, replayed admitted {}", replayed.len());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs NATS: set AERIS_TEST_NATS_URL"]
async fn forged_and_replayed_envelopes_are_rejected_over_jetstream() {
    let Some(e) = env() else { return };
    let k = keys();
    let mut rx = ingest(&e, &k).await;
    // Capture one genuine event.
    let (h, _stop) = run_simd(simd_config(&e, &k, "s1", 5, 10.0, false)).await;
    let s = h.await.unwrap();
    let (ok, _) = collect(&mut rx, s.published, Duration::from_secs(30)).await;
    let genuine = ok.last().unwrap().clone();
    let simd = aeris_simd::connect(&settings(&e.url, e.simd.clone()), "attacker")
        .await
        .unwrap();
    let js = async_nats::jetstream::new(simd);
    let subject = aeris_wire::event_subject("node-a");
    // 1. Replay the captured envelope byte for byte (new Nats-Msg-Id so
    //    JetStream stores it; the verifier must still refuse it).
    let env_bytes = aeris_wire::encode_envelope(&genuine.body, &genuine.signature);
    js.publish(subject.clone(), env_bytes.into())
        .await
        .unwrap()
        .await
        .unwrap();
    // 2. A forged event: valid structure, signed by an attacker key.
    let mut forged = genuine.event.clone();
    forged.source = "node-a".into();
    forged.seq = 10_000_000;
    forged.t_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let body = aeris_wire::encode_body(&forged).unwrap();
    let sig = SigningIdentity::generate("node-a")
        .unwrap()
        .sign(Domain::Event, &body);
    js.publish(
        subject.clone(),
        aeris_wire::encode_envelope(&body, &sig).into(),
    )
    .await
    .unwrap()
    .await
    .unwrap();
    // 3. Garbage.
    js.publish(subject, vec![0xFF; 40].into())
        .await
        .unwrap()
        .await
        .unwrap();
    let mut reasons = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while reasons.len() < 3 {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(Ingested::Rejected(r))) => reasons.push(r.reason),
            Ok(Some(Ingested::Admitted(a))) => panic!("admitted {:?}", a.event),
            _ => break,
        }
    }
    assert!(matches!(
        reasons[0],
        RejectReason::Replay(ReplayRejection::Duplicate { .. })
    ));
    assert_eq!(reasons[1], RejectReason::BadSignature);
    assert!(matches!(reasons[2], RejectReason::Wire(_)));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs NATS: set AERIS_TEST_NATS_URL"]
async fn commands_are_authenticated_and_idempotent_over_nats() {
    let Some(e) = env() else { return };
    let k = keys();
    let (h, stop) = run_simd(simd_config(&e, &k, "s1", 2, 1.0, true)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let client = server_client(&e).await;
    let sim_key = *k.registry.get(ids::SIM_CONTROLLER).unwrap();
    let server = SigningIdentity::load_secret_file("server", &k.dir.join("server.key")).unwrap();
    let mut adapter = NatsSimAdapter::new(client.clone(), server, sim_key, Duration::from_secs(3));
    let obs = adapter.observe().await.unwrap();
    assert_eq!(obs.node_mode("node-b"), Some(NodeMode::Active));
    let ack = adapter
        .submit(
            "dec-t1",
            &CommandOp::ActivateLink {
                link: "link-l2".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(ack.status, AckStatus::Applied);
    let ack = adapter
        .submit(
            "dec-t1",
            &CommandOp::ActivateLink {
                link: "link-l2".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(ack.status, AckStatus::AlreadyApplied, "idempotent");
    let ack = adapter
        .submit(
            "dec-t1",
            &CommandOp::Isolate {
                node: "node-q".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(ack.status, AckStatus::Failed, "unknown entity is reported");
    // Forged command: right name, wrong key.
    let mut rogue = NatsSimAdapter::new(
        client.clone(),
        SigningIdentity::generate("server").unwrap(),
        sim_key,
        Duration::from_secs(3),
    );
    let ack = rogue
        .submit(
            "dec-evil",
            &CommandOp::Isolate {
                node: "node-a".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(ack.status, AckStatus::Rejected);
    assert!(ack.reason.contains("signature"), "{}", ack.reason);
    let obs = adapter.observe().await.unwrap();
    assert_eq!(
        obs.node_mode("node-a"),
        Some(NodeMode::Active),
        "forgery had no effect"
    );
    stop.send(true).unwrap();
    let s = h.await.unwrap();
    assert_eq!(s.commands_rejected, 1, "{s:?}");
    assert!(s.commands_applied >= 4);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs NATS: set AERIS_TEST_NATS_URL"]
async fn golden_s4_end_to_end_over_nats_with_ai_off() {
    let Some(e) = env() else { return };
    let k = keys();
    let speed = 2.0;
    let mut rx = ingest(&e, &k).await;
    let (h, stop) = run_simd(simd_config(&e, &k, "s4", 7, speed, true)).await;
    let client = server_client(&e).await;
    let sim_key = *k.registry.get(ids::SIM_CONTROLLER).unwrap();
    let server = SigningIdentity::load_secret_file("server", &k.dir.join("server.key")).unwrap();
    let w = world();
    let mut exec = Executor::new(
        NatsSimAdapter::new(client, server, sim_key, Duration::from_secs(3)),
        TopologyGuard::new(Arc::clone(&w)),
        Arc::clone(&w),
        ExecConfig::default(),
    );
    let mut core = CoreState::new();
    let cycle = Duration::from_secs_f64(0.1 / speed);
    let mut executed = None;
    let mut first_incident = None;
    let started = tokio::time::Instant::now();
    while started.elapsed() < Duration::from_secs_f64(60.0 / speed + 5.0) {
        tokio::time::sleep(cycle).await;
        let mut batch: Vec<VerifiedEvent> = Vec::new();
        while let Ok(x) = rx.try_recv() {
            match x {
                Ingested::Admitted(a) => batch.push(a.to_verified_event()),
                Ingested::Rejected(r) => panic!("rejected genuine event: {r:?}"),
            }
        }
        let out = aeris_core::step(&mut core, &batch, 7);
        if first_incident.is_none() && !out.incidents.is_empty() {
            first_incident = Some(out.incidents[0].clone());
        }
        if executed.is_none()
            && let Some(p) = out.proposal
        {
            assert!(
                matches!(p.gate, Gate::HumanReview { .. }),
                "HIGH impact needs a human"
            );
            // The executed action is exactly the optimizer's recommendation.
            let best = p
                .candidates
                .iter()
                .filter(|c| c.violations.is_empty())
                .max_by(|a, b| a.score.total_cmp(&b.score))
                .unwrap();
            println!(
                "recommended {} (J={:.3}, P={:.3})",
                p.recommended_action.display_name(),
                best.score,
                best.p_success
            );
            let target = "node-b".to_string();
            let req = ExecutionRequest {
                decision_id: p.id.clone(),
                incident_id: p.incident_id.clone(),
                action: p.recommended_action.clone(),
                authorization: Authorization::Human(Approval {
                    operator: "nats-e2e-test-commander".into(),
                    role: Role::Commander,
                    verdict: Verdict::Approve,
                    approved_action: p.recommended_action.clone(),
                    reason: "explicit test approval".into(),
                    at_ms: 0,
                }),
                target_criticality: core.compute_criticality(&target),
                expected_capabilities: None,
                node_trust: ["node-a", "node-b", "node-c"]
                    .iter()
                    .filter_map(|n| {
                        core.trust_registry
                            .node_trust(n)
                            .map(|t| (n.to_string(), t))
                    })
                    .collect(),
            };
            let report = exec.execute(req).await;
            println!("execution: {:?} {:?}", report.state, report.commands);
            assert_eq!(report.state, ExecState::Verified, "{report:#?}");
            core.apply_action(&p.recommended_action);
            executed = Some((p.recommended_action, report));
        }
    }
    stop.send(true).unwrap();
    let s = h.await.unwrap();
    let inc = first_incident.expect("incident opened over NATS");
    assert_eq!(inc.entity, "node-b");
    let (action, report) = executed.expect("an approved action was executed");
    let v = report.verification.unwrap();
    assert!(v.ok);
    println!(
        "GOLDEN S4 over NATS: incident {} on {}, executed {}, verified obs B={:?} L2 active={}, simd {:?}",
        inc.id,
        inc.entity,
        action.display_name(),
        v.observed.node_mode("node-b"),
        v.observed.link("link-l2").unwrap().active,
        s
    );
    assert_eq!(s.undelivered_at_shutdown, 0);
    assert_eq!(s.commands_rejected, 0);
}

#[tokio::test]
#[ignore = "needs NATS: set AERIS_TEST_NATS_URL"]
async fn unreachable_nats_fails_with_a_clear_error() {
    let s = NatsSettings {
        url: "nats://127.0.0.1:1".into(),
        auth: NatsAuth::None,
        tls_ca: None,
        connect_attempts: 2,
    };
    let err = aeris_simd::connect(&s, "x").await.unwrap_err().to_string();
    assert!(
        err.contains("could not connect") && err.contains("127.0.0.1:1"),
        "{err}"
    );
}

#[test]
fn stream_name_matches_simd() {
    assert_eq!(aeris_simd::STREAM_NAME, "AERIS_EVENTS");
    let _ = Path::new("");
}
