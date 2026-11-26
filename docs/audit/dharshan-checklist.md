# Dharshan — engineering checklist (live)

Status legend: [x] done and verified · [~] in progress · [ ] not started · [!] blocked (reason given)

## Workspace / infra
- [x] Toolchain pin 1.100.0 (was a Windows-GNU triple)
- [x] Edition 2024, workspace builds; fmt + clippy green on existing crates (mechanical fixes only)
- [x] Workspace dependency versions pinned; deviation rand 0.8 documented (M-2)
- [x] deny.toml + cargo-deny run (see update below)
- [x] CI workflow
- [x] deploy/compose.yaml, .env.example, NATS/Postgres config, health verified
- [!] Verify build/tests on rust:1.100.0 container (2026-11-26: host disk full; verified on rustc 1.99.1 instead, CI runs 1.100.0)
- [x] 2026-11-26 re-verify: fmt fixed (harness, graph), clippy `too_many_arguments` in aeris-graph allowed; `cargo test --workspace` all green

## aeris-wire
- [x] proto/aeris/v1/event.proto, command.proto (protox codegen, no protoc)
- [x] Validated WireEvent / Command / CommandAck, lossless round trips
- [x] Unknown enums, missing fields, malformed bytes, size limits, canonical UUIDv7
- [x] HLC send/receive with overflow + forward-drift guard
- [x] Γ(a) commands_for_action (shared by rollouts and execution)
- 21 unit tests + proptests

## aeris-crypto
- [x] Identities, key files (0600 enforced), registry (weak keys rejected), revocation
- [x] Domain-separated Ed25519 (event/command/ack/audit), verify_strict
- [x] Sequence counter (no wrap), replay window (2 s; S7 replayed rule + max age)
- [x] EventVerifier: exact-byte verification; integrity ledger → evidence (aggregated)
- [x] Command signing/verification with subject binding; signed acks
- [x] BLAKE3 audit chain; canonical JSON (NaN refused); tamper/missing/reorder/sig/truncation detection
- 33 tests

## aeris-sim
- [x] docs/spec/world.toml, scenarios/s1..s8.toml
- [x] World F(S,a,W) 10 Hz, ChaCha8 streams via BLAKE3 KDF, ground truth separated from observations
- [x] Attacker / fault injection for all eight scenarios (+ actuator faults for exec tests)
- [x] Commands: isolate/restrict/recover/activate/failover/degrade, idempotent, validated
- [x] fork(), rollouts with CRN, isolation, reproducibility, belief sampling, verify snapshot
- 33 tests

## aeris-exec
- [x] Adapter trait, LocalSimAdapter, NATS SimAdapter
- [x] Executor: structure, authorization, policy recheck, dedupe, ordered submit, timeouts
- [x] Verification ‖e‖ ≤ ε, replan signal, escalation after 3

## aeris-simd
- [x] keygen, run, JetStream publish with acks, signed command handling, S7 buffer/replay, shutdown
- [x] Forged command gets a correlatable signed REJECTED ack (fixed 2026-11-26)

## Integration
- [x] aeris-core stage 8 uses aeris-sim rollouts (replaced hardcoded p_success table)
- [x] Closed-loop harness (in-process) S1–S8; G2 20-seed S4; G4 100-seed sweep (metrics fixed 2026-11-26, regenerated)
- [x] NATS end-to-end S4 against compose
- [x] verify-audit CLI (JSONL and PostgreSQL)
- [x] PostgreSQL: audit chain stored with migrations/0001_init.sql, tamper detected (2026-11-26)
- [x] `aeris-harness live`: closed loop over the running stack with typed approval (2026-11-26)
- [x] aeris-graph + migrations imported from origin/main; builds and tests in workspace
- [!] aeris-server / aeris-policy / aeris-mission: do not exist (Kesivarshini)
- [!] Amirtha's ai-crates branch: unmerged, conflicts in aeris-core stages 4/10

## Workspace / infra (update)
- [x] deny.toml + CI deny job
- [x] CI workflow (fmt, check, clippy, test, deny, NATS + PostgreSQL e2e)
- [x] deploy/compose.yaml, .env.example, NATS/Postgres config, health verified live

## Docs
- [x] docs/audit/dharshan-initial-audit.md
- [x] implementation report, security audit, deployment checklist
- [x] completion plan, Techgium readiness (2026-11-26)
