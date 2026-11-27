//! AERIS closed-loop integration harness (owner: DHARSHAN; shared with
//! `aeris-eval`, AMIRTHA + DHARSHAN).
//!
//! * [`closed_loop`] — one scenario × seed through every real component.
//! * [`sweep`] — S1–S8 × N seeds × {AERIS, alert-only, naive} metrics.
//! * [`nats_ingest`] — JetStream consumer + `aeris-crypto` verification for
//!   the NATS end-to-end tests.
//! * [`verify_audit_file`] — `verify-audit` over a JSONL audit export.
//! * [`live`] — the loop against a running stack (simd → JetStream → core →
//!   typed human approval → exec over NATS → verification → audit).
//! * [`pg`] — audit chain to and from PostgreSQL (migrations/0001_init.sql).
//!
//! The harness never decides anything: decisions come from `aeris-core`,
//! authorization from the gate or an *explicitly enabled* test approver,
//! execution from `aeris-exec`.

#![forbid(unsafe_code)]

pub mod closed_loop;
pub mod live;
pub mod nats_ingest;
pub mod pg;
pub mod sweep;

use aeris_crypto::audit::{Checkpoint, VerifyReport, parse_jsonl, verify_chain};
use aeris_crypto::keys::{KeyRegistry, ids};
use std::path::Path;

/// Verifies a JSONL audit export against the server key in `registry`.
pub fn verify_audit_file(
    file: &Path,
    registry: &Path,
    checkpoint: Option<Checkpoint>,
) -> Result<VerifyReport, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let entries = parse_jsonl(&text).map_err(|e| e.to_string())?;
    verify_audit_entries(&entries, registry, checkpoint)
}

/// Verifies audit entries (from any store) against the server key in `registry`.
pub fn verify_audit_entries(
    entries: &[aeris_crypto::audit::AuditEntry],
    registry: &Path,
    checkpoint: Option<Checkpoint>,
) -> Result<VerifyReport, String> {
    let reg_text =
        std::fs::read_to_string(registry).map_err(|e| format!("{}: {e}", registry.display()))?;
    let reg = KeyRegistry::from_toml(&reg_text).map_err(|e| e.to_string())?;
    let key = reg.get(ids::SERVER).map_err(|e| e.to_string())?;
    Ok(verify_chain(entries, key, checkpoint))
}
