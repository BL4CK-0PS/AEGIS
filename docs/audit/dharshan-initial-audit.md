# Dharshan — Initial Repository Audit (AERIS v0.1.0)

- **Date:** 2026-11-25 (schedule day D5, G2 gate day)
- **Commit audited:** `0801510` on `main` (clean tree); unmerged remote branch `origin/amirtha/ai-crates` (`7c6c27a`) also inspected.
- **Scope owned (per `docs/v0.1/02-team-stack-layout.md`):** workspace, CI, compose, Quantrix DC deploy, `aeris-wire`, `aeris-crypto`, `aeris-sim` + `aeris-simd`, `aeris-exec`, co-owner of `aeris-eval`.

> **Naming.** The request refers to "AEGIS", `aegis.md` and `proto/aegis/v1/`. The repository names the product **AERIS**
> (`aeris-*` crates, `proto/aeris/v1/` in the contracts doc) and `aegis.md` is not in the repo (the v0.1 docs cite it at
> `startup/chatgpt/aegis/aegis.md`, outside this repository). All work follows the repository's names and the
> `docs/v0.1/` specification, which the docs declare to be the source of truth for v0.1.

## 1. Repository architecture (as found)

```text
Cargo.toml            workspace: aeris-types, aeris-trust, aeris-decision, aeris-core, apps/aeris-console
rust-toolchain.toml   channel = "stable-x86_64-pc-windows-gnu"
crates/aeris-types    K  domain types (Action, Candidate, TrustVector, VerifiedEvent, CycleOutput, ...)
crates/aeris-trust    K  Beta trust registry
crates/aeris-decision K  CVaR, evaluate_rollout_samples, check_hard_invariants, optimize_decision
crates/aeris-core     K  step(): stages 1–10 (stub-level detection/fusion; see findings)
apps/aeris-console    K  terminal/HTML renderer, fixture playback
docs/v0.1/            build specification (source of truth)
docs/spec/            params.toml, threat-model.md, demo-script.md
```

Missing from `main` entirely (all owned by Dharshan unless noted): `proto/`, `deploy/`, `deny.toml`, CI workflow,
`docs/spec/world.toml`, `scenarios/s1..s8.toml`, `aeris-wire`, `aeris-crypto`, `aeris-sim`, `apps/aeris-simd`,
`aeris-exec`. Also missing (other owners): `aeris-graph`, `aeris-mission`, `aeris-policy`, `apps/aeris-server`,
`migrations/` (Kesivarshini); `aeris-estimator`, `aeris-anomaly`, `aeris-fusion`, `aeris-explain`, `apps/aeris-eval`
(Amirtha — present only on the unmerged `amirtha/ai-crates` branch).

### Dependency map (target, with Dharshan's components)

```text
                     ┌──────────── aeris-types (K) ────────────┐
                     │                                          │
 aeris-wire (D) ─────┤  prost types  ⇄  WireEvent  →  VerifiedEvent
 aeris-crypto (D) ───┤  Ed25519 sign/verify, replay window, BLAKE3 audit chain
 aeris-sim (D) ──────┤  World S'=F(S,a,W), attacker, scenarios, fork + Monte Carlo rollouts
 aeris-exec (D) ─────┤  Adapter trait, Γ(a) → commands, verification ‖e‖≤ε, replan signal
                     │
 aeris-core (K) ─────┴─ uses aeris-trust, aeris-decision, (aeris-sim rollouts)
 apps/aeris-simd (D): aeris-sim + aeris-wire + aeris-crypto + async-nats  ──JetStream──▶ aeris.events.<node>
                                                                          ◀──request── aeris.cmd.<node>
 apps/aeris-server (KV, missing): ingest → crypto verify → core → policy → approval → aeris-exec
```

## 2. Baseline results (commands run on `main` @ `0801510`, rustc 1.99.1 system toolchain)

| Check | Result |
|---|---|
| `cargo check --workspace --all-targets` | PASS |
| `cargo test --workspace` | PASS — 9 tests (types 4, trust 2, decision 1, core 1, console 1) |
| `cargo fmt --all -- --check` | **FAIL** — 39 diff hunks in K-owned crates |
| `cargo clippy --workspace --all-targets -- -D warnings` | **FAIL** — 3 errors in `aeris-types` (`derivable_impls`, `needless_range_loop`, `match_like_matches_macro`), 1 more in `aeris-core` (`collapsible_match`) once types compiles |
| `cargo deny check` | NOT RUNNABLE — no `deny.toml`, `cargo-deny` not installed |
| CI | NONE — no workflow exists, so G0 ("CI green on main") was never met |
| Docker Compose | NONE — no `deploy/` directory |
| Environment | No `rustup` (pin file ignored locally), no `protoc`, Docker 29.9 + Compose 5.6 present; `rust:1.100.0`, `postgres:18`, `nats:2.15-alpine` images pullable |

## 3. Findings

### CRITICAL

| ID | Finding | Files | Reproduction | Required fix |
|---|---|---|---|---|
| C-1 | Toolchain pinned to a **Windows-GNU host triple** (`stable-x86_64-pc-windows-gnu`). On any Linux/macOS machine with rustup this tries to run a Windows toolchain; it is also not the agreed version (1.100.0). Builds are not reproducible. | `rust-toolchain.toml` | `rustup show` in repo on Linux | Pin `channel = "1.100.0"` with `rustfmt`, `clippy` components. |
| C-2 | **No event integrity layer exists.** `VerifiedEvent.signature_valid` is a caller-supplied bool; nothing signs or verifies anything; no proto, no replay window. The threat model's T-03/T-04 mitigations are unimplemented. | `crates/aeris-types/src/lib.rs`, `crates/aeris-core/src/lib.rs` | `grep -r ed25519 crates` → nothing | Implement `aeris-wire` + `aeris-crypto`. |
| C-3 | **No simulator exists.** Every number in the S4 flow is literal. `aeris-core` stage 8 uses a hardcoded `p_success` table (`Monitor 0.31, Reroute 0.55, Combo 0.94, ...`) fed through `rng.gen_bool` — the "winning" action is effectively scripted, contradicting `01-definition-of-done.md` ("no hardcoded numbers") and `08` ("No demo fiction"). | `crates/aeris-core/src/lib.rs` (stage 8) | read `p_success_nominal` match | Implement `aeris-sim` rollouts; replace the table with real rollouts (cross-team change, documented). |
| C-4 | **No execution path or verification exists.** `CoreState::apply_action` mutates the model directly; there is no adapter, no command, no observed-vs-expected check, no approval enforcement at the execution boundary. | `crates/aeris-core/src/lib.rs` | — | Implement `aeris-exec`. |
| C-5 | (Amirtha branch, unmerged) `aeris-eval verify-audit` **prints a hardcoded success** ("All 12 decision records valid") without reading anything; baselines are hardcoded `false`; every run counts a true positive unconditionally; the harness auto-approves every proposal regardless of the gate. If merged, `results.md` would report fabricated numbers. | `apps/aeris-eval/src/main.rs` on `origin/amirtha/ai-crates` | `git show origin/amirtha/ai-crates:apps/aeris-eval/src/main.rs` | Back `verify-audit` with `aeris_crypto::audit::verify_chain`; measure baselines; never auto-approve without an explicit flag. Owner coordination needed (shared A+D app). |

### HIGH

| ID | Finding | Files | Required fix |
|---|---|---|---|
| H-1 | No CI at all; fmt and clippy are already red on `main`. | — | Add `.github/workflows/ci.yml` with fmt, check, clippy `-D warnings`, test, cargo-deny; fix the 4 clippy + fmt issues (mechanical, no behaviour change). |
| H-2 | No `deploy/compose.yaml`, `.env.example`, NATS/Postgres config. | — | Create, verify health. |
| H-3 | `aeris-core` dedupe set `processed_event_seqs` grows without bound (one entry per event forever) — memory exhaustion over a long run (~150 ev/s → ~13M entries/day). | `crates/aeris-core/src/lib.rs` | Replay/dedupe belongs in `aeris-crypto::ReplayWindow` (O(sources) state). Owner: K — report. |
| H-4 | `aeris-core` fabricates values when data is missing: explanation embeds literal `0.94`, `0.415`, `0.90→0.57` and fallback evidence IDs `ev-0412…`; incident `risk: 0.987` literal; posterior literal `0.947`; blast radius literal `1.0` for `node-b`; observation and navigation capabilities are literal tables. | `crates/aeris-core/src/lib.rs` | Owner K (+A for fusion/explain, KV for graph/mission). Reported; not modified except stage 8 rollouts (C-3). |
| H-5 | `Action::disruption_cost` and `impact_class` hardcode `params.toml` values in code ("no magic numbers in code" rule). | `crates/aeris-types/src/lib.rs` | Owner K. Reported. |

### MEDIUM

| ID | Finding | Files | Required fix |
|---|---|---|---|
| M-1 | Workspace edition is 2021; the agreed stack is edition 2024. | `Cargo.toml` | Move to 2024 after verifying all crates compile. |
| M-2 | Stack table pins `rand 0.10.4 / rand_chacha 0.10.0`; workspace uses `rand 0.8 / rand_chacha 0.3`, and all existing code (main and Amirtha's branch) is written against the 0.8 API. Upgrading now breaks other owners' code. | `Cargo.toml` | Keep 0.8/0.3 for v0.1, document the deviation, coordinate upgrade post-v0.1. |
| M-3 | `params.toml` `t_min = 0.40` contradicts §6.3 `T_min = 0.05` (Kalman noise floor) — the key appears to conflate the UI label threshold with the estimator floor; `cusum_h = 5.0` vs §6.4 `h = 4`; `sliding_window_l = 100` vs §6.4 300 samples. | `docs/spec/params.toml` | Owner K/A. Reported. |
| M-4 | `aeris-core` ignores `replayed` and timestamps entirely; no freshness check. | `crates/aeris-core/src/lib.rs` | Provided by `aeris-crypto::ReplayWindow` at ingest. |
| M-5 | `VerifiedEvent` has no `confidence` / `provenance` fields although §6.2 defines them, and `EventPayload::Net` has no `peers` although the proto sketch does. Wire→domain projection is therefore lossy. | `crates/aeris-types/src/lib.rs` | Keep the full data in `aeris_wire::WireEvent`; document the gap; propose an additive change to K. |

### LOW

| ID | Finding | Files |
|---|---|---|
| L-1 | `Cargo.toml` `authors` lists one person for the whole workspace; `license = "Proprietary"` is not an SPDX id, so cargo-deny needs an explicit exception. | `Cargo.toml` |
| L-2 | `docs/00–25` and root `README.md` describe an earlier concept (multimodal evidence reasoning); new readers will be misled. | `README.md` |
| L-3 | `.clawness/` tool state committed in-tree (only `memory.md`, harmless). | `.clawness/memory.md` |

## 4. Dependencies on other developers

| Needed by Dharshan's scope | Owner | Status | Consequence |
|---|---|---|---|
| `aeris-server` (ingest, decision lifecycle, approvals, audit store) | Kesivarshini | missing | Full `simd → NATS → server → …` path cannot run. A clearly-labelled test harness will stand in for ingest in tests only. |
| `aeris-policy` (Π, invariants I1–I4, autonomy gate) | Kesivarshini | missing | Executor re-check will call the existing `aeris_decision::check_hard_invariants` behind a `PolicyRecheck` trait; swap in `aeris-policy` when it lands. |
| `migrations/` | Kesivarshini | missing | Audit chain verification reads a JSONL export; Postgres reader blocked. |
| `aeris-graph`, `aeris-mission` | Kesivarshini | missing | Core keeps its literal capability tables (H-4). |
| anomaly/fusion/estimator/explain, `aeris-eval` | Amirtha | unmerged branch | Integration tested against `main` only; C-5 must be fixed before merge. |
| `aeris-types` additions (M-5) | Kishanth | — | Proposed, not changed unilaterally. |

## 5. Plan (execution order)

1. Workspace: toolchain pin, edition 2024, deny.toml, CI, fmt/clippy fixes (mechanical).
2. `proto/aeris/v1` + `aeris-wire` (pure-Rust codegen via `protox`, so no `protoc` binary is needed).
3. `aeris-crypto`: keys, domain-separated Ed25519, sequence counter, replay window, BLAKE3 audit chain.
4. `docs/spec/world.toml`, `scenarios/s1..s8.toml`, `aeris-sim` (world, attacker, scenarios, fork, rollouts).
5. `apps/aeris-simd` (JetStream publish with acks, signed command channel, S7 buffering).
6. `aeris-exec` (Adapter, LocalSimAdapter, NATS SimAdapter, executor, verification, replan).
7. Wire real rollouts into `aeris-core` stage 8; closed-loop harness; G2 20-seed run; NATS E2E.
8. Compose + deploy artifacts, security audit, test report, deployment checklist.
