use crate::CryptoError;
use crate::audit::*;
use crate::command_auth::*;
use crate::ingest::*;
use crate::keys::*;
use crate::replay::*;
use aeris_types::Hlc;
use aeris_wire::{AckStatus, CommandAck, CommandOp, Provenance, WireEvent, WirePayload};
use prost::Message;

const NOW: i64 = 1_798_761_600_000;

fn ident(id: &str, b: u8) -> SigningIdentity {
    // Test-only deterministic seeds; production keys come from keygen.
    SigningIdentity::from_seed(id, &[b; 32]).unwrap()
}

fn uuid7(ms: u64, r: u8) -> String {
    uuid::Builder::from_unix_timestamp_millis(ms, &[r; 10])
        .into_uuid()
        .to_string()
}

fn event(source: &str, seq: u64, t: i64, replayed: bool) -> WireEvent {
    WireEvent {
        id: uuid7(t as u64, seq as u8),
        source: source.into(),
        seq,
        hlc: Hlc { l: t, c: 0 },
        t_unix_ms: t,
        replayed,
        confidence: 1.0,
        provenance: Provenance {
            producer: format!("test/{source}"),
            producer_version: "0".into(),
        },
        payload: WirePayload::Telemetry {
            cpu_pct: 40.0,
            mem_pct: 30.0,
            proc_count: 10,
        },
    }
}

fn setup() -> (EventSigner, EventSigner, EventVerifier) {
    let a = ident("node-a", 1);
    let b = ident("node-b", 2);
    let mut reg = KeyRegistry::new();
    reg.insert("node-a", a.verifying_key()).unwrap();
    reg.insert("node-b", b.verifying_key()).unwrap();
    (
        EventSigner::new(a),
        EventSigner::new(b),
        EventVerifier::new(reg, ReplayConfig::default()),
    )
}

#[test]
fn signed_event_verifies_and_projects() {
    let (mut a, _, mut v) = setup();
    let seq = a.next_seq().unwrap();
    let ev = event("node-a/gps", seq, NOW, false);
    let env = a.seal(&ev).unwrap();
    let ok = v.admit(&env, NOW + 50).unwrap();
    assert_eq!(ok.event, ev);
    assert_eq!(ok.admission, Admission::Fresh);
    let ve = ok.to_verified_event();
    assert!(ve.signature_valid);
    assert_eq!(ve.source, "node-a/gps");
}

#[test]
fn signature_covers_exact_received_bytes() {
    let (mut a, _, mut v) = setup();
    let ev = event("node-a", a.next_seq().unwrap(), NOW, false);
    let env = a.seal(&ev).unwrap();
    let raw = aeris_wire::decode_envelope(&env).unwrap();
    // Append an unknown field: decodes to the same struct, but the bytes
    // differ, so the signature must fail. Verifying a re-encoding would
    // wrongly accept this.
    let mut body = raw.body.clone();
    body.extend_from_slice(&[0x98, 0x06, 0x01]);
    assert_eq!(aeris_wire::decode_body(&body).unwrap(), ev);
    let tampered = aeris_wire::encode_envelope(&body, &raw.signature);
    let r = v.admit(&tampered, NOW).unwrap_err();
    assert_eq!(r.reason, RejectReason::BadSignature);
}

#[test]
fn forged_events_are_rejected_and_attributed() {
    let (mut a, mut b, mut v) = setup();
    let mut ledger = IntegrityLedger::default();

    // 1. B signs an event claiming to be A (insider forgery).
    let ev = event("node-a", 1, NOW, false);
    let body = aeris_wire::encode_body(&ev).unwrap();
    let sig = ident("node-b", 2).sign(Domain::Event, &body);
    let r = v
        .admit(&aeris_wire::encode_envelope(&body, &sig), NOW)
        .unwrap_err();
    assert_eq!(r.reason, RejectReason::BadSignature);
    assert_eq!(r.claimed_source.as_deref(), Some("node-a"));
    ledger.record(&r, v.keys());

    // 2. Flip one payload byte of a genuine envelope.
    let env = {
        let seq = a.next_seq().unwrap();
        a.seal(&event("node-a", seq, NOW, false))
    }
    .unwrap();
    let mut raw = aeris_wire::decode_envelope(&env).unwrap();
    let last = raw.body.len() - 1;
    raw.body[last] ^= 0x01;
    let r = v
        .admit(&aeris_wire::encode_envelope(&raw.body, &raw.signature), NOW)
        .unwrap_err();
    assert!(matches!(
        r.reason,
        RejectReason::BadSignature | RejectReason::Wire(_)
    ));

    // 3. Flip a signature bit.
    let mut raw = aeris_wire::decode_envelope(&env).unwrap();
    raw.signature[0] ^= 0x80;
    let r = v
        .admit(&aeris_wire::encode_envelope(&raw.body, &raw.signature), NOW)
        .unwrap_err();
    assert_eq!(r.reason, RejectReason::BadSignature);
    ledger.record(&r, v.keys());

    // 4. A signature from another domain (a command signature) is refused.
    let ev = event("node-b", b.next_seq().unwrap(), NOW, false);
    let body = aeris_wire::encode_body(&ev).unwrap();
    let wrong_domain = ident("node-b", 2).sign(Domain::Command, &body);
    let r = v
        .admit(&aeris_wire::encode_envelope(&body, &wrong_domain), NOW)
        .unwrap_err();
    assert_eq!(r.reason, RejectReason::BadSignature);

    // 5. Unknown node: not attributable.
    let ghost = ident("node-z", 9);
    let env = EventSigner::new(ghost)
        .seal(&event("node-z", 1, NOW, false))
        .unwrap();
    let r = v.admit(&env, NOW).unwrap_err();
    assert_eq!(r.reason, RejectReason::UnknownSource);
    ledger.record(&r, v.keys());

    // 6. Garbage bytes.
    let r = v.admit(&[1, 2, 3], NOW).unwrap_err();
    assert!(matches!(r.reason, RejectReason::Wire(_)));
    ledger.record(&r, v.keys());

    assert_eq!(ledger.unattributed, 2);
    let ev = ledger.drain_evidence();
    assert_eq!(
        ev.len(),
        1,
        "two bad_signature rejections against node-a aggregate"
    );
    assert_eq!(ev[0].entity, "node-a");
    assert_eq!(
        ev[0].kind,
        aeris_types::EvidenceKind::Other("integrity:bad_signature".into())
    );
    assert!(
        ledger.drain_evidence().is_empty(),
        "drain empties the ledger"
    );
    // b is still usable after all that.
    let env = {
        let seq = b.next_seq().unwrap();
        b.seal(&event("node-b", seq, NOW, false))
    }
    .unwrap();
    assert!(v.admit(&env, NOW).is_ok());
}

#[test]
fn replayed_and_duplicate_envelopes_are_rejected() {
    let (mut a, _, mut v) = setup();
    let env1 = {
        let seq = a.next_seq().unwrap();
        a.seal(&event("node-a", seq, NOW, false))
    }
    .unwrap();
    let env2 = {
        let seq = a.next_seq().unwrap();
        a.seal(&event("node-a", seq, NOW + 100, false))
    }
    .unwrap();
    assert!(v.admit(&env1, NOW).is_ok());
    assert!(v.admit(&env2, NOW + 100).is_ok());
    // Exact duplicate delivery (at-least-once bus).
    let r = v.admit(&env2, NOW + 150).unwrap_err();
    assert_eq!(
        r.reason,
        RejectReason::Replay(ReplayRejection::Duplicate { seq: 2 })
    );
    // Attacker replays a captured old event later.
    let r = v.admit(&env1, NOW + 1_000).unwrap_err();
    assert!(matches!(r.reason, RejectReason::Replay(_)));
    // A valid but stale event (delayed > 2 s, not flagged replayed).
    let env3 = {
        let seq = a.next_seq().unwrap();
        a.seal(&event("node-a", seq, NOW, false))
    }
    .unwrap();
    let r = v.admit(&env3, NOW + 2_001).unwrap_err();
    assert!(matches!(
        r.reason,
        RejectReason::Replay(ReplayRejection::Stale { .. })
    ));
    // A stale rejection did not consume seq 3.
    assert_eq!(v.last_seq("node-a"), 2);
}

#[test]
fn s7_partition_buffer_is_accepted_once_in_order() {
    let (mut a, _, mut v) = setup();
    // 20 s partition at 10 Hz: 200 buffered events, then live traffic.
    let start = NOW - 20_000;
    let buffered: Vec<_> = (0..200)
        .map(|i| {
            {
                let seq = a.next_seq().unwrap();
                a.seal(&event("node-a", seq, start + i * 100, true))
            }
            .unwrap()
        })
        .collect();
    for env in &buffered {
        let ok = v.admit(env, NOW).unwrap();
        assert_eq!(ok.admission, Admission::Replayed);
    }
    // The simulator flushes again after a reconnect: all rejected.
    for env in &buffered {
        assert!(v.admit(env, NOW + 10).is_err());
    }
    let live = {
        let seq = a.next_seq().unwrap();
        a.seal(&event("node-a", seq, NOW, false))
    }
    .unwrap();
    assert_eq!(v.admit(&live, NOW).unwrap().admission, Admission::Fresh);
    // The same buffered events without the replayed flag would be stale.
    let (mut a2, _, mut v2) = setup();
    let old = {
        let seq = a2.next_seq().unwrap();
        a2.seal(&event("node-a", seq, start, false))
    }
    .unwrap();
    assert!(matches!(
        v2.admit(&old, NOW).unwrap_err().reason,
        RejectReason::Replay(ReplayRejection::Stale { .. })
    ));
}

#[test]
fn revoked_key_is_refused() {
    let (_, mut b, mut v) = setup();
    v.keys_mut().revoke("node-b");
    let env = {
        let seq = b.next_seq().unwrap();
        b.seal(&event("node-b", seq, NOW, false))
    }
    .unwrap();
    assert_eq!(
        v.admit(&env, NOW).unwrap_err().reason,
        RejectReason::RevokedSource
    );
    v.keys_mut().reinstate("node-b");
    assert!(v.admit(&env, NOW).is_ok());
}

#[test]
fn signer_refuses_other_nodes_sources() {
    let (a, _, _) = setup();
    assert!(a.seal(&event("node-b", 1, NOW, false)).is_err());
}

#[test]
fn key_registry_round_trip_and_rejects_bad_keys() {
    let (_, _, v) = setup();
    let text = v.keys().to_toml();
    let back = KeyRegistry::from_toml(&text).unwrap();
    assert_eq!(back.ids().collect::<Vec<_>>(), vec!["node-a", "node-b"]);
    // Identity point (small order) must be rejected.
    let mut weak = [0u8; 32];
    weak[0] = 1;
    assert!(parse_public_hex(&hex::encode(weak)).is_err());
    assert!(parse_public_hex("zz").is_err());
    assert!(KeyRegistry::from_toml("[keys]\n\"bad.id\" = \"00\"").is_err());
    assert!(KeyRegistry::from_toml("[other]\n").is_err());
    let mut r = KeyRegistry::new();
    let k = ident("node-a", 1).verifying_key();
    r.insert("node-a", k).unwrap();
    assert!(r.insert("node-a", k).is_err(), "duplicate id");
}

#[test]
fn identity_debug_never_prints_secret() {
    let i = ident("node-a", 0xAB);
    let dbg = format!("{i:?}");
    assert!(dbg.contains("<redacted>"));
    assert!(!dbg.contains(&"ab".repeat(32)));
}

#[cfg(unix)]
#[test]
fn key_files_are_created_0600_and_loose_modes_are_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("node-a.key");
    let i = SigningIdentity::generate("node-a").unwrap();
    i.write_secret_file(&p).unwrap();
    let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let back = SigningIdentity::load_secret_file("node-a", &p).unwrap();
    assert_eq!(back.public_hex(), i.public_hex());
    assert!(i.write_secret_file(&p).is_err(), "refuses to overwrite");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        SigningIdentity::load_secret_file("node-a", &p),
        Err(CryptoError::KeyFormat(_))
    ));
    let missing = dir.path().join("nope.key");
    assert!(matches!(
        SigningIdentity::load_secret_file("node-a", &missing),
        Err(CryptoError::Io(_))
    ));
    let bad = dir.path().join("bad.key");
    std::fs::write(&bad, "not hex").unwrap();
    std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(SigningIdentity::load_secret_file("node-a", &bad).is_err());
}

#[test]
fn generated_keys_are_distinct() {
    let a = SigningIdentity::generate("node-a").unwrap();
    let b = SigningIdentity::generate("node-a").unwrap();
    assert_ne!(a.public_hex(), b.public_hex());
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_setup() -> (CommandSigner, CommandVerifier) {
    let srv = ident("server", 5);
    let v = CommandVerifier::new("server", srv.verifying_key(), ReplayConfig::default());
    (CommandSigner::new(srv), v)
}

#[test]
fn commands_authenticate_and_bind_to_subject() {
    let (mut s, mut v) = cmd_setup();
    let (cmd, env) = s
        .issue(
            uuid7(NOW as u64, 1),
            "dec-1",
            CommandOp::Isolate {
                node: "node-b".into(),
            },
            NOW,
        )
        .unwrap();
    // Wrong subject: an attacker re-routing a genuine command.
    assert!(matches!(
        v.admit("aeris.cmd.node-a", &env, NOW),
        Err(CommandRejection::SubjectMismatch { .. })
    ));
    // The failed attempt did not consume the seq.
    assert_eq!(v.admit("aeris.cmd.node-b", &env, NOW).unwrap(), cmd);
    // Replay of the same command.
    assert!(matches!(
        v.admit("aeris.cmd.node-b", &env, NOW),
        Err(CommandRejection::Replay(ReplayRejection::Duplicate { .. }))
    ));
}

#[test]
fn forged_commands_are_rejected() {
    let (_, mut v) = cmd_setup();
    let mut rogue = CommandSigner::new(ident("server", 6)); // wrong key, right name
    let (_, env) = rogue
        .issue(
            uuid7(NOW as u64, 2),
            "dec-x",
            CommandOp::Recover {
                node: "node-b".into(),
            },
            NOW,
        )
        .unwrap();
    assert_eq!(
        v.admit("aeris.cmd.node-b", &env, NOW),
        Err(CommandRejection::BadSignature)
    );
    let mut other = CommandSigner::new(ident("operator", 5));
    let (_, env) = other
        .issue(uuid7(NOW as u64, 3), "d", CommandOp::Observe, NOW)
        .unwrap();
    assert!(matches!(
        v.admit("aeris.cmd.world", &env, NOW),
        Err(CommandRejection::UntrustedIssuer(_))
    ));
    // An event-domain signature by the real server key is not a command.
    let srv = ident("server", 5);
    let cmd = aeris_wire::Command {
        id: uuid7(NOW as u64, 4),
        decision_id: "d".into(),
        issuer: "server".into(),
        seq: 1,
        t_unix_ms: NOW,
        op: CommandOp::Observe,
    };
    let body = aeris_wire::encode_command(&cmd).unwrap();
    let sig = srv.sign(Domain::Event, &body);
    let env = aeris_wire::encode_command_envelope(&body, &sig);
    assert_eq!(
        v.admit("aeris.cmd.world", &env, NOW),
        Err(CommandRejection::BadSignature)
    );
    // Stale command.
    let (mut s, mut v) = cmd_setup();
    let (_, env) = s
        .issue(uuid7(NOW as u64, 5), "d", CommandOp::Observe, NOW - 5_000)
        .unwrap();
    assert!(matches!(
        v.admit("aeris.cmd.world", &env, NOW),
        Err(CommandRejection::Replay(ReplayRejection::Stale { .. }))
    ));
}

#[test]
fn acks_are_signed_and_bound_to_command() {
    let sim = ident("sim-controller", 8);
    let ack = CommandAck {
        command_id: "c-1".into(),
        status: AckStatus::Applied,
        reason: String::new(),
        observation: None,
        t_unix_ms: NOW,
    };
    let env = seal_ack(&sim, &ack).unwrap();
    assert_eq!(open_ack(&sim.verifying_key(), &env, "c-1").unwrap(), ack);
    assert!(matches!(
        open_ack(&sim.verifying_key(), &env, "c-2"),
        Err(CryptoError::AckMismatch { .. })
    ));
    assert_eq!(
        open_ack(&ident("x", 1).verifying_key(), &env, "c-1"),
        Err(CryptoError::BadSignature)
    );
}

// ---------------------------------------------------------------------------
// Audit chain
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct Rec {
    decision: String,
    action: String,
    p_success: f64,
    nested: std::collections::HashMap<String, f64>,
}

fn rec(i: u32) -> Rec {
    let mut nested = std::collections::HashMap::new();
    for k in ["zeta", "alpha", "mid"] {
        nested.insert(k.to_string(), f64::from(i) * 0.1);
    }
    Rec {
        decision: format!("dec-{i}"),
        action: "isolate node-b + reroute link-l2".into(),
        p_success: 0.5 + f64::from(i) / 100.0,
        nested,
    }
}

fn build_chain(n: u32) -> (SigningIdentity, Vec<AuditEntry>, Checkpoint) {
    let srv = ident("server", 5);
    let mut c = AuditChain::new();
    let entries = (1..=n)
        .map(|i| c.append_record(&srv, &format!("dec-{i}"), &rec(i)).unwrap())
        .collect();
    (srv, entries, c.checkpoint())
}

#[test]
fn canonical_json_is_deterministic_and_sorted() {
    // HashMap iteration order varies; output must not.
    let a = canonical_json(&rec(3)).unwrap();
    for _ in 0..20 {
        assert_eq!(canonical_json(&rec(3)).unwrap(), a);
    }
    let s = String::from_utf8(a).unwrap();
    assert!(s.find("\"action\"").unwrap() < s.find("\"decision\"").unwrap());
    assert!(s.find("\"alpha\"").unwrap() < s.find("\"zeta\"").unwrap());
    // Compact: no newlines and no space after separators.
    assert!(!s.contains('\n') && !s.contains("\": ") && !s.contains(", \""));
    let mut bad = rec(1);
    bad.p_success = f64::NAN;
    assert!(canonical_json(&bad).is_err(), "NaN must not become null");
    let mut bad = rec(1);
    bad.nested.insert("x".into(), f64::INFINITY);
    assert!(canonical_json(&bad).is_err());
}

#[test]
fn chain_matches_formula() {
    let (srv, e, cp) = build_chain(3);
    let mut h = GENESIS;
    for entry in &e {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&h);
        hasher.update(&entry.record);
        h = *hasher.finalize().as_bytes();
        assert_eq!(entry.hash, h);
    }
    assert_eq!(cp.head, h);
    let r = verify_chain(&e, &srv.verifying_key(), Some(cp));
    assert!(r.ok, "{r:?}");
    assert_eq!(r.verified, 3);
}

#[test]
fn empty_chain_verifies_to_genesis() {
    let srv = ident("server", 5);
    let r = verify_chain(&[], &srv.verifying_key(), None);
    assert!(r.ok);
    assert_eq!(r.head, GENESIS);
    assert_eq!(r.verified, 0);
    let r = verify_chain(
        &[],
        &srv.verifying_key(),
        Some(Checkpoint {
            n: 0,
            head: GENESIS,
        }),
    );
    assert!(r.ok);
}

#[test]
fn tampered_record_is_detected_at_its_index() {
    let (srv, mut e, cp) = build_chain(5);
    // Change one digit inside record 3 (e.g. p_success 0.53 -> 0.93).
    let s = String::from_utf8(e[2].record.clone()).unwrap();
    e[2].record = s.replace("0.53", "0.93").into_bytes();
    let r = verify_chain(&e, &srv.verifying_key(), Some(cp));
    assert!(!r.ok);
    assert_eq!(r.broken_at, Some(3));
    assert_eq!(r.failure, Some(AuditFailure::HashMismatch { n: 3 }));
    assert_eq!(r.verified, 2);
}

#[test]
fn tamper_with_recomputed_hash_is_caught_by_signature_or_next_link() {
    let (srv, mut e, cp) = build_chain(5);
    // Attacker edits record 2 and recomputes its hash (but cannot re-sign).
    e[1].record = b"{\"forged\":true}".to_vec();
    e[1].hash = chain_hash(&e[0].hash, &e[1].record);
    let r = verify_chain(&e, &srv.verifying_key(), Some(cp));
    assert_eq!(r.failure, Some(AuditFailure::BadSignature { n: 2 }));
    // Re-signing with any key other than the server's is also refused.
    let rogue = ident("server", 99);
    e[1].sig = rogue.sign(Domain::Audit, &e[1].hash);
    let r = verify_chain(&e, &srv.verifying_key(), Some(cp));
    assert_eq!(r.failure, Some(AuditFailure::BadSignature { n: 2 }));
}

#[test]
fn missing_record_is_detected() {
    let (srv, mut e, cp) = build_chain(5);
    e.remove(2);
    let r = verify_chain(&e, &srv.verifying_key(), Some(cp));
    assert_eq!(
        r.failure,
        Some(AuditFailure::Sequence {
            position: 3,
            expected: 3,
            found: 4
        })
    );
    // Renumbering after deletion is caught by the hash link instead.
    for (i, x) in e.iter_mut().enumerate() {
        x.n = i as u64 + 1;
    }
    let r = verify_chain(&e, &srv.verifying_key(), Some(cp));
    assert_eq!(r.failure, Some(AuditFailure::HashMismatch { n: 3 }));
}

#[test]
fn reordered_records_are_detected() {
    let (srv, mut e, cp) = build_chain(5);
    e.swap(1, 3);
    let r = verify_chain(&e, &srv.verifying_key(), Some(cp));
    assert!(matches!(
        r.failure,
        Some(AuditFailure::Sequence { position: 2, .. })
    ));
    // Swap contents but keep n in order.
    let (srv, mut e, cp) = build_chain(5);
    let (r1, r3) = (e[1].record.clone(), e[3].record.clone());
    e[1].record = r3;
    e[3].record = r1;
    let r = verify_chain(&e, &srv.verifying_key(), Some(cp));
    assert_eq!(r.failure, Some(AuditFailure::HashMismatch { n: 2 }));
}

#[test]
fn invalid_signature_is_detected() {
    let (srv, mut e, cp) = build_chain(4);
    e[3].sig[10] ^= 1;
    let r = verify_chain(&e, &srv.verifying_key(), Some(cp));
    assert_eq!(r.failure, Some(AuditFailure::BadSignature { n: 4 }));
    // Wrong verification key fails at the first record.
    let r = verify_chain(&e, &ident("server", 1).verifying_key(), None);
    assert_eq!(r.broken_at, Some(1));
}

#[test]
fn truncation_needs_a_checkpoint_and_is_then_detected() {
    let (srv, mut e, cp) = build_chain(5);
    e.truncate(3);
    // Without a checkpoint a shorter chain is internally consistent.
    assert!(verify_chain(&e, &srv.verifying_key(), None).ok);
    let r = verify_chain(&e, &srv.verifying_key(), Some(cp));
    assert_eq!(
        r.failure,
        Some(AuditFailure::Truncated {
            expected: 5,
            found: 3
        })
    );
    // A checkpoint from a different chain of the same length.
    let (srv2, e2, _) = build_chain(3);
    let r = verify_chain(
        &e2,
        &srv2.verifying_key(),
        Some(Checkpoint {
            n: 3,
            head: [7; 32],
        }),
    );
    assert_eq!(r.failure, Some(AuditFailure::CheckpointMismatch { n: 3 }));
}

#[test]
fn jsonl_round_trip_and_resume() {
    let (srv, e, cp) = build_chain(4);
    let text = to_jsonl(&e).unwrap();
    let back = parse_jsonl(&text).unwrap();
    assert_eq!(back, e);
    let mut c = AuditChain::resume(&back, &srv.verifying_key()).unwrap();
    assert_eq!(c.checkpoint(), cp);
    let e5 = c.append_record(&srv, "dec-5", &rec(5)).unwrap();
    assert_eq!(e5.n, 5);
    let mut all = back.clone();
    all.push(e5);
    assert!(verify_chain(&all, &srv.verifying_key(), Some(cp)).ok);
    // Resume refuses a tampered chain.
    let mut bad = back;
    bad[0].record.push(b' ');
    assert!(AuditChain::resume(&bad, &srv.verifying_key()).is_err());
    assert!(parse_jsonl("{not json}").is_err());
}

#[test]
fn sequence_counter_overflow_surfaces_through_signer() {
    let mut s = EventSigner::new(ident("node-a", 1));
    // Not reachable in practice; exercised through the public error type.
    for _ in 0..3 {
        assert!(s.next_seq().is_ok());
    }
    let mut c = SequenceCounter::resume_after(u64::MAX);
    assert!(c.advance().is_err());
}

#[test]
fn hostile_envelopes_do_not_panic() {
    let (_, _, mut v) = setup();
    let mut x: u64 = 1;
    for len in 0..300 {
        let mut buf = Vec::with_capacity(len);
        for _ in 0..len {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            buf.push(x as u8);
        }
        assert!(v.admit(&buf, NOW).is_err());
    }
    // Valid envelope framing around a garbage body.
    let env = aeris_wire::pb::Envelope {
        body: vec![0x0A, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F],
        signature: vec![0; 64],
    }
    .encode_to_vec();
    assert!(v.admit(&env, NOW).is_err());
}
