# Dharshan — Implementation report (AERIS v0.1.0)

Branch `dharshan/v0.1-infra-crypto-sim-exec`, created from `main @ 0801510`. **Nothing is committed**: this machine
has no git author identity configured and none was invented. All changes are in the working tree (see §8).

**Update 2026-11-26:** see [completion plan](dharshan-completion-plan.md) and
[Techgium readiness](dharshan-techgium-readiness.md) for today's fixes, live verification and demo runbook.

Companion documents: [initial audit](dharshan-initial-audit.md) · [checklist](dharshan-checklist.md) ·
[security audit](../security/dharshan-security-audit.md) · [readiness + test results](dharshan-techgium-readiness.md) ·
[deployment checklist](../deployment/dharshan-deployment-checklist.md).

## 1. What existed and what was wrong (summary)

`main` had four K-owned crates and a console. None of Dharshan's scope existed: no proto, wire, crypto, simulator,
daemon, executor, CI, compose or deny policy. The toolchain was pinned to a Windows-GNU triple; fmt and clippy were
red; `aeris-core` chose actions from a hardcoded success table. Details in the initial audit.

## 2. Components delivered

| Component | Path | What it does | Tests |
|---|---|---|---|
| Workspace | `Cargo.toml`, `rust-toolchain.toml`, `deny.toml` | Rust 1.100.0 pin, edition 2024, resolver 3, pinned deps, `publish = false`, dev-profile optimisation of hot dependencies | — |
| CI | `.github/workflows/ci.yml` | fmt · check · clippy `-D warnings` · test (incl. G2) · cargo-deny · NATS end-to-end against compose | — |
| Protobuf | `proto/aeris/v1/event.proto`, `command.proto` | Event envelope/body (id, source, seq, HLC, time, replayed, confidence, provenance, payload); signed command channel; signed acks; observable world state | — |
| `aeris-wire` | `crates/aeris-wire` | protox+prost codegen (no `protoc`), validated lossless `WireEvent`/`Command`/`CommandAck`, size limits, HLC, Γ(a) `commands_for_action`, projection to `aeris_types::VerifiedEvent` | 21 + proptests |
| `aeris-crypto` | `crates/aeris-crypto` | identities and key files, registry, domain-separated Ed25519, sequence counter, replay window (S7 rule), `EventSigner`/`EventVerifier`, integrity evidence, command/ack authentication, BLAKE3 audit chain + verification | 33 |
| `aeris-sim` | `crates/aeris-sim`, `docs/spec/world.toml`, `scenarios/s1..s8.toml` | World `F(S,a,W)` at 10 Hz, ChaCha8 streams via BLAKE3-KDF, attacker/fault injection for S1–S8 (+ actuator faults), ground truth separated from observations, command effects, `fork()`, Monte Carlo rollouts with common random numbers, capability estimation from observations | 35 |
| `aeris-exec` | `crates/aeris-exec` | `Adapter` trait, `LocalSimAdapter`, `NatsSimAdapter` (feature `nats`), authorization gate, policy re-check seam + interim `TopologyGuard`, verification `‖e‖ ≤ ε`, executor lifecycle, replan + escalation | 20 |
| `aeris-simd` | `apps/aeris-simd` | keygen; daemon publishing signed events to JetStream with acks, ordered retries, S7 buffering/replay, authenticated idempotent command handling, graceful shutdown, summary | NATS e2e |
| `aeris-harness` | `apps/aeris-harness` | in-process closed loop over all real components, sweeps S1–S8 × seeds × {AERIS, alert-only, naive}, JetStream ingest, `verify-audit` CLI | G2 + NATS e2e |
| Compose | `deploy/compose.yaml`, `deploy/.env.example`, `deploy/nats/*`, `deploy/docker/aeris-simd.Dockerfile` | Postgres 18, NATS 2.15 + JetStream, health checks, ordering, volumes, non-root, ACLs | verified live |

### Key design decisions

1. **Signing exact bytes + domain separation.** The spec's `σ = Sign(bytes(e∖σ))` is implemented over the transmitted body bytes, prefixed with a per-purpose tag, because the server key signs both commands and audit hashes. Verifiers never re-encode (`signature_covers_exact_received_bytes`).
2. **`confidence` and `provenance` on the wire.** §6.2 lists them in `e`; the proto sketch did not. They were added as fields 7/8 (additive). `aeris_types::VerifiedEvent` lacks them, so `WireEvent` keeps full fidelity and `to_verified_event` is the documented projection.
3. **Strict replay window per spec** (`seq > last`), with a bounded history only to tell duplicates from other out-of-order events. JetStream preserves per-subject order and simd never reorders, so no reorder window is needed.
4. **Ground truth vs observation.** `World` exposes truth only through `ground_truth()` (evaluator) and topology through `observe()`. Rollout bases are rebuilt from observations; hidden state is sampled from belief using `[threat_model]` (AERIS's model), not from the scenario's attacker (ground truth).
5. **Common random numbers** in rollouts: rollout *n* uses the same stream for every candidate and each tick draws a fixed number of values, so candidates differ only by their action.
6. **Γ(a) in one place** (`aeris_wire::commands_for_action`) and used by both rollouts and execution, so simulated and executed effects cannot drift. Paths are activated before isolation.
7. **Verification uses AERIS's own estimate.** Observed topology + measured link availability + AERIS's node trust (§6.7), never simulator truth. This was found necessary by a failing test (Monitor under compromise looked "verified" from topology alone).
8. **Approval is explicit.** The harness test commander is opt-in (`--approve`) and labelled in every audit record. Without it, non-LOW actions never execute (`without_approval_high_impact_never_runs`).
9. **Common sense over literal version pins:** rand 0.8/rand_chacha 0.3 kept (all existing code uses 0.8 APIs); everything else at the stack table's versions.

## 3. Public interfaces (for other owners)

```rust
// Ingest (aeris-server, Kesivarshini): admit every envelope from JetStream.
let mut v = aeris_crypto::ingest::EventVerifier::new(registry, ReplayConfig::default());
match v.admit(&msg.payload, now_ms) {
    Ok(a)  => pipeline_events.push(a.to_verified_event()),       // persist a.body + a.signature
    Err(r) => integrity_ledger.record(&r, v.keys()),              // → drain_evidence() each cycle
}
v.keys_mut().revoke("node-b");                                     // after a verified isolation

// Rollouts (aeris-core / aeris-decision, Kishanth): already wired into core stage 8.
let batch = aeris_sim::run_rollouts(&base, &belief, &aeris_wire::commands_for_action(&a)?, &RolloutSpec::v01(seed))?;
aeris_decision::evaluate_rollout_samples(a, &batch.successes(), &params);

// Execution (aeris-server): after approval + policy.
let mut ex = aeris_exec::Executor::new(NatsSimAdapter::new(client, server_key, sim_pub, t), policy, world_cfg, ExecConfig::default());
let report = ex.execute(ExecutionRequest { decision_id, incident_id, action, authorization, target_criticality, expected_capabilities, node_trust }).await;
// report.state: VERIFIED | VERIFICATION_FAILED | COMMAND_FAILED | PARTIALLY_EXECUTED | TIMED_OUT | REFUSED; report.replan

// Audit (aeris-server): append + verify; GET /api/audit/verify → {ok, broken_at}.
let entry = chain.append_record(&server_key, &decision_id, &record)?;   // store n, record, hash, sig in audit_log
let report = aeris_crypto::audit::verify_chain(&rows, &server_pub, Some(checkpoint));
```

NATS subjects: `aeris.events.<node>` (JetStream stream `AERIS_EVENTS`, `Nats-Msg-Id` = event id) and
`aeris.cmd.<entity>` (request/reply; `<entity>` = node, link, `mission` for degrade, `world` for observe).

## 4. Changes to other owners' code (all reviewable, behaviour-preserving unless stated)

| File | Change | Why |
|---|---|---|
| `crates/aeris-types/src/lib.rs`, `aeris-trust`, `aeris-decision`, `aeris-console` | rustfmt; clippy's own suggestions (derive `Default`, `matches!`, zip loop, let-chains) | CI was red; no behaviour change (9/9 original tests still pass) |
| `crates/*/Cargo.toml`, `apps/aeris-console/Cargo.toml` | `publish.workspace = true` | cargo-deny private-crate licence handling |
| `crates/aeris-core/src/lib.rs` stage 8 | **Behaviour change.** Replaced the hardcoded `p_success` table (`Monitor 0.31, …, Combo 0.94`) and `rng.gen_bool` with `aeris_sim::run_rollouts` of Γ(a) from core's own topology view and its posterior | Spec §6.10; audit C-3. Core's own S4 test still passes with real rollouts |
| `crates/aeris-core/src/lib.rs` stage 10 | **Behaviour change.** Explanation template now prints computed values (current trust, posterior, chosen candidate's simulated success/score, actual rejected candidates) instead of literals `0.90→0.57`, `0.94`, `0.415`, fallback evidence ids | Real numbers sat next to fabricated ones after the stage-8 change (audit H-4). `amirtha/ai-crates` replaces this block with `aeris-explain`; expect a merge conflict there |
| `crates/aeris-core/Cargo.toml` | + `aeris-sim`, `aeris-wire` | the above |

Needs Kishanth's review per the contracts rule (`06-contracts.md`). `aeris-types` itself was **not** changed; the
proposed additive changes are listed in §6.

## 5. Integration status

| Path | Status |
|---|---|
| simd → NATS JetStream (authenticated, acked) | ✅ verified live |
| JetStream → `EventVerifier` (test ingest) | ✅ verified live (S1, S7, forged/replayed) |
| verified events → `aeris-core::step` (main) | ✅ in-process (G2/G4) and over NATS (golden S4) |
| core stage 8 → `aeris-sim` rollouts → `aeris-decision` optimizer | ✅ |
| proposal → approval → `aeris-exec` → `NatsSimAdapter` → simd → ack → verification | ✅ over NATS (golden S4) |
| audit chain + `verify-audit` | ✅ |
| audit chain → PostgreSQL `audit_log` (Kesivarshini's `migrations/0001_init.sql`) → `verify-audit --pg` | ✅ verified live (2026-11-26) |
| `aeris-harness live`: simd → JetStream → core → typed approval → exec over NATS → verify → audit | ✅ verified live (2026-11-26) |
| `aeris-graph`, migrations (Kesivarshini, `origin/main` b0d41bc) | ✅ imported verbatim, builds and tests in this workspace; graph not yet called by core |
| `aeris-server`, `aeris-policy`, `aeris-mission` | ❌ do not exist (Kesivarshini) — harness stands in for ingest/approval; `TopologyGuard` for policy |
| Amirtha's estimator/anomaly/fusion/explain, `aeris-eval` | ⚠ unmerged branch; not integrated. Integration on `main` only |

## 6. Proposed contract changes (not applied; need owner approval)

1. `aeris-types::VerifiedEvent`: add `confidence: f64` and `provenance: Provenance` (spec §6.2), `EventPayload::Net { peers }`.
2. `aeris-types::EvidenceKind`: add `Integrity(String)` instead of relying on `Other("integrity:…")`.
3. `aeris-core::step`: accept integrity evidence (from `IntegrityLedger::drain_evidence`) and link-status measurements, so forged events and S3 degradation reach trust and rollouts.
4. `Action::Recover` impact class: at least MEDIUM until the §6.13 recovery state machine exists.
5. `docs/spec/params.toml`: resolve `t_min` (0.40 vs §6.3 0.05), `cusum_h` (5 vs 4), `sliding_window_l` (100 vs 300).

## 7. Known limitations

* Detection and fusion on `main` are threshold stubs with literal posteriors (owner K/A); the G2 detection delay of 0 s reflects that, not a real detector.
* Core recomputes 3 000 rollouts on every tick an incident stays open (≈80 ms release); alert-only runs therefore take ~25 s each in sweeps.
* Mission success is a harness definition (≥90 % of the final 10 s with all capabilities ≥ minimum); §6.16 does not define it precisely.
* World/threat-model parameters (fade rate, lateral hazard, relay integrity) are v0.1 assumptions in `world.toml`; results are only as good as they are.
* TLS was not exercised; `aeris-server` absent; DC not deployed.

## 8. Files

New: `.github/workflows/ci.yml`, `.dockerignore`, `deny.toml`, `proto/`, `scenarios/`, `deploy/` (except the git-ignored
`.env` and `keys/`), `docs/spec/world.toml`, `docs/audit/*`, `docs/security/*`, `docs/testing/*`, `docs/deployment/*`,
`crates/aeris-{wire,crypto,sim,exec}/`, `apps/aeris-{simd,harness}/`.
Modified: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.gitignore`, `crates/aeris-{types,trust,decision,core}/…`,
`apps/aeris-console/…` (see §4).
