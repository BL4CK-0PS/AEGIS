//! Audit chain through PostgreSQL with Kesivarshini's `migrations/0001_init.sql`.
//!
//! Needs a running Postgres: `AERIS_TEST_DATABASE_URL=postgres://user:pw@127.0.0.1:port/db`
//! (compose: `deploy/compose.yaml`). Each test works in its own throwaway
//! schema, so the database's real tables are never touched.

use aeris_harness::closed_loop::{ApprovalPolicy, RunConfig, Strategy, run};
use aeris_harness::pg;
use std::sync::Arc;

struct Schema {
    client: postgres::Client,
    name: String,
}

impl Drop for Schema {
    fn drop(&mut self) {
        let _ = self
            .client
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.name));
    }
}

fn schema() -> Option<Schema> {
    let url = std::env::var("AERIS_TEST_DATABASE_URL").ok()?;
    let mut client = pg::connect(&url).expect("connect");
    let name = format!("t_{}", uuid::Uuid::now_v7().simple());
    client
        .batch_execute(&format!("CREATE SCHEMA {name}; SET search_path TO {name}"))
        .unwrap();
    pg::migrate(&mut client).unwrap();
    Some(Schema { client, name })
}

/// Runs S4 with a server key written to a temp file; returns the entries
/// and the server's public key.
fn s4_entries(
    seed: u64,
) -> (
    Vec<aeris_crypto::audit::AuditEntry>,
    aeris_crypto::VerifyingKey,
) {
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join("server.key");
    let server = aeris_crypto::keys::SigningIdentity::generate("server").unwrap();
    server.write_secret_file(&key_path).unwrap();
    let w = Arc::new(aeris_sim::WorldConfig::frozen().unwrap());
    let cfg = RunConfig {
        scenario: Arc::new(aeris_sim::Scenario::builtin("s4", &w).unwrap()),
        world: w,
        seed,
        strategy: Strategy::Aeris,
        approval: ApprovalPolicy {
            test_commander_approves: true,
            delay_ticks: 0,
        },
        narrate: false,
        server_key_file: Some(key_path),
    };
    let (r, entries) = run(&cfg).unwrap();
    assert!(r.audit_ok && !entries.is_empty());
    (entries, server.verifying_key())
}

#[test]
#[ignore = "needs Postgres: set AERIS_TEST_DATABASE_URL"]
fn audit_chain_round_trips_through_postgres_and_tampering_is_detected() {
    let Some(mut s) = schema() else { return };
    let (entries, server_pub) = s4_entries(3);
    assert_eq!(
        pg::store_audit(&mut s.client, &entries).unwrap(),
        entries.len()
    );

    // Parent rows exist and the decision status matches the execution.
    let row = s
        .client
        .query_one(
            "SELECT d.status, d.gate, d.required_role, i.entity FROM decisions d JOIN incidents i ON i.id = d.incident_id ORDER BY d.created_at LIMIT 1",
            &[],
        )
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "VERIFIED");
    assert_eq!(row.get::<_, String>(1), "human_review");
    assert_eq!(row.get::<_, String>(2), "commander");
    assert_eq!(row.get::<_, String>(3), "node-b");

    let back = pg::load_audit(&mut s.client).unwrap();
    assert_eq!(back, entries, "rows read back are byte-identical");
    let ok = aeris_crypto::audit::verify_chain(&back, &server_pub, None);
    assert!(ok.ok, "chain from Postgres verifies: {ok:?}");

    // A second chain must not be appended to a non-empty log.
    assert!(pg::store_audit(&mut s.client, &entries).is_err());

    // Tamper with one stored record: the chain no longer recomputes.
    s.client
        .execute(
            "UPDATE audit_log SET record = record || '\\x20'::bytea WHERE n = 1",
            &[],
        )
        .unwrap();
    let tampered = pg::load_audit(&mut s.client).unwrap();
    assert_ne!(tampered[0].record, entries[0].record);
    let bad = aeris_crypto::audit::verify_chain(&tampered, &server_pub, None);
    assert!(!bad.ok);
    assert_eq!(bad.broken_at, Some(1));
    assert!(matches!(
        bad.failure,
        Some(aeris_crypto::audit::AuditFailure::HashMismatch { n: 1 })
    ));
}
