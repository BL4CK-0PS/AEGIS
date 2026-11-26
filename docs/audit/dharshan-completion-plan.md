# Dharshan — Techgium completion plan (2026-11-26)

Branch `dharshan/v0.1-infra-crypto-sim-exec` (base `main @ 0801510`, all work uncommitted: the machine has no git
identity and none was invented). This plan was written after re-auditing the working tree, `origin/main`
(`b0d41bc`, Kesivarshini's `aeris-graph` + `migrations/0001_init.sql`) and `origin/amirtha/ai-crates` (`7c6c27a`).
Prior detail: [initial audit](dharshan-initial-audit.md), [implementation report](dharshan-implementation-report.md).

## 1. Repository state found

| Area | Owner | State on this branch | Evidence |
|---|---|---|---|
| Workspace, toolchain, deps | Dharshan | Complete. Edition 2024, resolver 3, toolchain file 1.100.0, `rust-version` 1.99 | `Cargo.toml`, `rust-toolchain.toml` |
| fmt / clippy | Dharshan | **Was red** (rustfmt drift; 3 clippy errors: `manual_is_multiple_of` ×2, `large_enum_variant` in simd CLI). Fixed today | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| CI | Dharshan | Complete: fmt, check, clippy, test (G2 ×20), cargo-deny, NATS e2e against compose | `.github/workflows/ci.yml` |
| Compose (Postgres 18, NATS 2.15 JetStream) | Dharshan | Complete, health checks, ACLs, no default credentials | `deploy/compose.yaml`; running locally (healthy) |
| PostgreSQL integration | Dharshan / Kesivarshini | **Missing**: nothing wrote to Postgres. Migration existed only on `origin/main`. Added today (§3) | `apps/aeris-harness/src/pg.rs` |
| `aeris-wire` | Dharshan | Complete (proto, validation, HLC, Γ(a)) | 21 tests + proptests |
| `aeris-crypto` | Dharshan | Complete (Ed25519 exact bytes, replay window incl. S7, audit chain) | 33 tests |
| `aeris-sim` + rollouts | Dharshan | Complete (10 Hz, ChaCha8, S1–S8 faults, `fork()`, CRN rollouts N=500) | 35 tests |
| `aeris-simd` | Dharshan | Complete (signed JetStream publish, acks, S7 buffer/replay, signed commands, shutdown) | NATS e2e |
| `aeris-exec` | Dharshan | Complete (Adapter, Local/NATS adapters, authz gate, verification ‖e‖≤ε, replan/escalate) | 20 tests |
| Harness (closed loop, sweep, verify-audit) | Dharshan | Complete; **metric bug** found today (S6 scored B's detection as FP and B's injected compromise as lateral spread). Fixed | `closed_loop.rs`, `sweep.rs` |
| `aeris-types/trust/decision/core/console` | Kishanth | Present. Core fusion posterior and capability scores are **literals** (`0.947`, `0.920/0.815/0.570`, nav `0.930`). Console is a CLI with fixture events, no web UI / APPROVE button | `crates/aeris-core/src/lib.rs:167-175, 461-472` |
| `aeris-graph`, `migrations/0001_init.sql` | Kesivarshini | On `origin/main` only. Imported verbatim into this branch today and added to the workspace | `git diff origin/main -- crates/aeris-graph migrations` is empty |
| `aeris-server`, `aeris-policy`, `aeris-mission`, auth | Kesivarshini | **Do not exist** anywhere | — |
| `aeris-estimator/anomaly/fusion/explain`, `aeris-eval` | Amirtha | On unmerged branch `origin/amirtha/ai-crates`; it rewrites `aeris-core` stage 4/10, conflicting with this branch's stage 8/10 changes. Not integrated | `git diff --stat main origin/amirtha/ai-crates` |

## 2. Task status (Dharshan)

| # | Task | Status | Dependency | Gap / action today |
|---|---|---|---|---|
| 1 | Workspace + toolchain | Done | — | Local host has Arch rustc 1.99.1 (no rustup), the toolchain file asks for 1.100.0; builds on both because `rust-version = 1.99` |
| 2 | fmt / clippy green | Done today | — | 3 lint fixes, `cargo fmt` |
| 3 | CI | Done | GitHub runner | Not run on GitHub from here (no push) |
| 4 | Compose + health checks | Done | Docker | Re-verified live |
| 5 | Postgres persistence | Done today (harness scope) | Kesivarshini's migration | `run --store-pg`, `verify-audit --pg`; stand-in for `aeris-server` audit writer |
| 6 | aeris-wire | Done | — | — |
| 7 | aeris-crypto (sign, replay, S7, audit) | Done | — | — |
| 8 | aeris-sim S1–S8 | Done | — | S3/S5/S7/S8 produce no proposal because core's detector stub only reacts to net/auth/process anomalies (Kishanth/Amirtha) |
| 9 | Monte Carlo N=500, H=100, CRN, fork isolation | Done | — | — |
| 10 | aeris-simd over real NATS | Done | NATS | Re-run e2e today |
| 11 | aeris-exec safety path | Done | — | Re-verified by G2 + no-approval test |
| 12 | Cross-team integration | Partial | other owners | graph imported (compiles + tests in workspace; not yet called by core — its consumers are Kishanth's/Kesivarshini's); Amirtha's branch not merged |
| 13 | Threat model / security validation | Done (docs/security) | — | Re-run forged/replay e2e |
| 14 | Readiness doc | Today | all above | `docs/audit/dharshan-techgium-readiness.md` |

## 3. Presentation-critical requirements (from `docs/spec/demo-script.md`, `docs/v0.1/09-demo-and-run.md`)

| Script claim | Implemented? | How to show it |
|---|---|---|
| Three nodes A,B,C, L1 primary, L2 standby | Yes (`aeris-sim`, `world.toml`) | `aeris-harness run --narrate` |
| S4 at t=30 s: beaconing, SSH brute force, rogue process, camera disagreement | Yes (attacker profile in `scenarios/s4.toml`) | narrated run |
| Signed events over NATS, forged/replayed rejected | Yes | NATS e2e tests |
| 500 Monte Carlo rollouts per option, "isolate only" vetoed by I1, isolate+reroute #1 | Yes (real rollouts, real filter) | narrated run; G2 |
| HUMAN_REVIEW for HIGH impact; nothing runs unapproved | Yes | G2 + `without_approval_high_impact_never_runs` |
| Execute: activate L2 then isolate B; verify observed state | Yes | narrated run / golden NATS e2e |
| Audit chain verifies | Yes (JSONL and now Postgres) | `verify-audit` |
| Posterior "94.7 %" | **Literal** in core fusion stub | do not present as computed |
| Trust "0.90 → 0.565, Beta(8.0,3.4)" | Trust is computed by `aeris-trust`; the specific numbers in the script are not reproduced — use printed values | narrated run |
| "P(success) = 0.94, J = 0.415" | Not literal any more: real rollout values are printed; they differ from the script | read numbers off the run |
| Capabilities Nav 0.930 / Obs 0.920/0.815, M(G)=1.0 | **Literal table** in core | do not present as measured |
| Evidence IDs `ev-0412…` | Not produced (ids are UUIDv7) | do not quote |
| `aeris-server` on :8080, console on :8081 (Trunk/Leptos), APPROVE button | **Not implemented** | use harness CLI |
| `aeris-eval` commands | Not on this branch (Amirtha's branch); equivalents: `aeris-harness sweep/verify-audit` | — |
| "Across 100 seeds" results table | Measured by `aeris-harness sweep` (`docs/testing/results/`) | regenerate S6 after metric fix |
