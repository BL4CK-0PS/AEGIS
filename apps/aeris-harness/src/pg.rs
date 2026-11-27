//! PostgreSQL persistence of the audit chain, using the schema owned by
//! aeris-server (`migrations/0001_init.sql`, Kesivarshini) **verbatim**.
//!
//! This is the harness standing in for aeris-server's audit writer until that
//! crate exists. It writes exactly what §6.15 / the migration describe:
//! `audit_log(n, decision_id, record, hash, sig)` with the parent `incidents`
//! and `decisions` rows the foreign keys require, and reads the chain back for
//! verification. The database is never trusted: `load_audit` returns raw rows
//! and [`aeris_crypto::audit::verify_chain`] decides.

use aeris_crypto::audit::AuditEntry;
use postgres::{Client, NoTls};

/// The schema, compiled in from the repository (statements are idempotent:
/// `CREATE … IF NOT EXISTS`).
pub const MIGRATION_0001: &str = include_str!("../../../migrations/0001_init.sql");

/// Connects. `url` is a libpq URL (`postgres://user:pass@host:port/db`); it
/// should come from the environment, never a flag (visible in `ps`).
pub fn connect(url: &str) -> Result<Client, String> {
    Client::connect(url, NoTls).map_err(|e| format!("postgres connect: {}", describe(&e)))
}

/// `postgres::Error`'s Display is only "db error"; include the server's
/// message (never contains the password).
fn describe(e: &postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => format!("{e}: {} ({})", db.message(), db.code().code()),
        None => match std::error::Error::source(e) {
            Some(src) => format!("{e}: {src}"),
            None => e.to_string(),
        },
    }
}

/// Applies `migrations/0001_init.sql`.
pub fn migrate(c: &mut Client) -> Result<(), String> {
    c.batch_execute(MIGRATION_0001)
        .map_err(|e| format!("migration 0001: {}", describe(&e)))
}

/// Decision status in the migration's vocabulary for an audit record.
/// Failed executions stay `EXECUTED`: the record itself carries the real
/// execution state, and a failure is never reported as `VERIFIED`.
fn decision_status(record: &serde_json::Value) -> &'static str {
    match record["execution"]["state"].as_str() {
        Some("VERIFIED") => "VERIFIED",
        Some("REFUSED") => match record["authorization"].get("human") {
            Some(h) if h["verdict"] == "reject" => "REJECTED",
            _ => "HUMAN_REVIEW",
        },
        Some(_) => "EXECUTED",
        None => "COMPUTED",
    }
}

/// Gate variant name (`auto` | `human_review` | `reject`).
fn gate_name(gate: &serde_json::Value) -> String {
    match gate {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(m) => m.keys().next().cloned().unwrap_or_default(),
        _ => String::new(),
    }
}

/// Appends `entries` to an **empty** `audit_log` in one transaction, keeping
/// each entry's chain position `n`. Refuses to append to a non-empty log
/// (the chain would not continue from its head).
pub fn store_audit(c: &mut Client, entries: &[AuditEntry]) -> Result<usize, String> {
    let mut tx = c.transaction().map_err(|e| e.to_string())?;
    let existing: i64 = tx
        .query_one("SELECT count(*) FROM audit_log", &[])
        .map_err(|e| e.to_string())?
        .get(0);
    if existing != 0 {
        return Err(format!(
            "audit_log already holds {existing} rows; refusing to start a second chain in it"
        ));
    }
    for e in entries {
        let record: serde_json::Value = serde_json::from_slice(&e.record)
            .map_err(|err| format!("audit record {} is not JSON: {err}", e.n))?;
        let incident_id = record["incident_id"]
            .as_str()
            .ok_or_else(|| format!("audit record {} has no incident_id", e.n))?;
        let entity = record["target"].as_str().unwrap_or_default();
        // Unknown or missing impact is treated as HIGH (fail safe).
        let required_role = match record["execution"]["impact"].as_str() {
            Some("LOW") | Some("MEDIUM") => "operator",
            _ => "commander",
        };
        let chosen = if record["authorization"].get("human").is_some()
            || record["execution"]["state"] != "REFUSED"
        {
            Some(record["recommended_action"].clone())
        } else {
            None
        };
        tx.execute(
            "INSERT INTO incidents (id, entity, posterior) VALUES ($1, $2, $3)
             ON CONFLICT (id) DO NOTHING",
            &[
                &incident_id,
                &entity,
                &serde_json::json!({ "source": "aeris-harness audit record" }),
            ],
        )
        .map_err(|err| format!("insert incident: {}", describe(&err)))?;
        tx.execute(
            "INSERT INTO decisions (id, incident_id, status, required_role, proposal, chosen_action, gate)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id) DO UPDATE SET status = EXCLUDED.status, updated_at = now()",
            &[
                &e.decision_id,
                &incident_id,
                &decision_status(&record),
                &required_role,
                &record["candidates"],
                &chosen,
                &gate_name(&record["gate"]),
            ],
        )
        .map_err(|err| format!("insert decision: {}", describe(&err)))?;
        let n = i32::try_from(e.n).map_err(|_| format!("chain position {} overflows", e.n))?;
        tx.execute(
            "INSERT INTO audit_log (n, decision_id, record, hash, sig) VALUES ($1, $2, $3, $4, $5)",
            &[&n, &e.decision_id, &e.record, &&e.hash[..], &&e.sig[..]],
        )
        .map_err(|err| format!("insert audit_log n={}: {}", e.n, describe(&err)))?;
    }
    // Keep the SERIAL sequence past the explicit positions.
    tx.execute(
        "SELECT setval(pg_get_serial_sequence('audit_log', 'n'), GREATEST((SELECT max(n) FROM audit_log), 1))",
        &[],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(entries.len())
}

/// Reads `audit_log` in chain order. Malformed rows (wrong hash/sig length)
/// are an error, not silently skipped.
pub fn load_audit(c: &mut Client) -> Result<Vec<AuditEntry>, String> {
    let rows = c
        .query(
            "SELECT n, decision_id, record, hash, sig FROM audit_log ORDER BY n ASC",
            &[],
        )
        .map_err(|e| e.to_string())?;
    rows.iter()
        .map(|r| {
            let n: i32 = r.get(0);
            let hash: Vec<u8> = r.get(3);
            let sig: Vec<u8> = r.get(4);
            Ok(AuditEntry {
                n: u64::try_from(n).map_err(|_| format!("negative n {n}"))?,
                decision_id: r.get(1),
                record: r.get(2),
                hash: hash
                    .try_into()
                    .map_err(|_| format!("audit_log n={n}: hash is not 32 bytes"))?,
                sig: sig
                    .try_into()
                    .map_err(|_| format!("audit_log n={n}: sig is not 64 bytes"))?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_never_reports_failure_as_verified() {
        let rec =
            |state: &str| serde_json::json!({"execution": {"state": state}, "authorization": {}});
        assert_eq!(decision_status(&rec("VERIFIED")), "VERIFIED");
        for s in [
            "VERIFICATION_FAILED",
            "COMMAND_FAILED",
            "PARTIALLY_EXECUTED",
            "TIMED_OUT",
        ] {
            assert_eq!(decision_status(&rec(s)), "EXECUTED", "{s}");
        }
        assert_eq!(decision_status(&rec("REFUSED")), "HUMAN_REVIEW");
    }

    #[test]
    fn gate_names_match_migration_vocabulary() {
        assert_eq!(gate_name(&serde_json::json!("auto")), "auto");
        assert_eq!(
            gate_name(&serde_json::json!({"human_review": {"reason": "x"}})),
            "human_review"
        );
        assert_eq!(
            gate_name(&serde_json::json!({"reject": {"reason": "x"}})),
            "reject"
        );
    }

    #[test]
    fn migration_is_the_repository_file() {
        assert!(MIGRATION_0001.contains("CREATE TABLE IF NOT EXISTS audit_log"));
    }
}
