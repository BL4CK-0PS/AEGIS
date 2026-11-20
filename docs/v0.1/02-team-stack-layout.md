# 02 · Team, stack and repository layout

[← Index](README.md)

## Team and ownership: each person owns whole crates

| Person | Role | Owns | ≈ days |
|---|---|---|---|
| **Kishanth** — cyber · AI · webdev · backend | Tech lead, architect, integration owner | `aeris-types` (contracts), `aeris-trust`, `aeris-decision` (actions and optimizer), `aeris-core` (pipeline), `aeris-console` (Leptos UI), threat model, demo script | 8 |
| **Dharshan** — DevOps · cyber | Infra and security engineer | Workspace, CI, compose, Quantrix DC deploy, `aeris-wire`, `aeris-crypto`, `aeris-sim` + `aeris-simd` (world, attacker, scenarios, Monte Carlo rollouts), `aeris-exec` (adapter and verification) | 8.5 |
| **Amirtha** — AI dev | AI/ML engineer | `aeris-estimator`, `aeris-anomaly` (including a hand-written Isolation Forest), `aeris-fusion`, `aeris-explain` (local LLM and grounding), `aeris-eval` metrics | 8.75 |
| **Kesivarshini** — backend dev | Backend engineer | `aeris-server` (ingest, REST, WebSocket, auth/RBAC, decision lifecycle, audit store), `migrations/`, `aeris-graph`, `aeris-mission` (criticality, risk, CVaR), `aeris-policy` (safety monitor and autonomy gate) | 8.5 |

That's about 34 of 40 person-days, which leaves roughly 15% buffer.

- **Different authors for the safety monitor and the optimizer.** The spec requires the safety monitor to be independent of the optimizer, so Kesivarshini writes `aeris-policy` and Kishanth writes `aeris-decision`.
- **`aeris-trust` is built as the future Quantrix Core trust crate**, which Canopus OS and Aquila will reuse. It has no I/O and its public API is documented.

## All-Rust stack (versions checked on 20 Nov 2026)

All application code is Rust. PostgreSQL and the NATS server are off-the-shelf infrastructure; we write no code for them. Pin the toolchain with `rust-toolchain.toml`.

| Layer | Choice | Version |
|---|---|---|
| Toolchain | Rust stable, edition 2024 | **1.100.0** (the local machine has 1.95, so run `rustup update`) |
| Async runtime | tokio | 1.54.1 |
| HTTP / WebSocket API | axum + tower-http | 0.8.9 / 0.7.1 |
| Sessions and auth | tower-sessions + tower-sessions-sqlx-store + argon2 | 0.15.0 / 0.16.0 / 0.6.0 |
| Database access | sqlx (Postgres, compile-time checked queries) | 0.9.1 (needs Rust ≥ 1.94) |
| Event bus client | async-nats (JetStream for replay) | 0.53.0 |
| Wire format | prost (Protobuf in `proto/aeris/v1/`) | 0.14.6 |
| Graph | petgraph | 0.8.3 |
| Linear algebra (Kalman filter) | nalgebra | 0.35.0 |
| Deterministic randomness | rand + rand_chacha (seeded) | 0.10.4 / 0.10.0 |
| Distributions (Beta etc.) | statrs | 0.19.1 |
| Signatures | ed25519-dalek | 3.0.0 |
| Audit hash chain | blake3 | 1.8.7 |
| TLS | rustls | 0.23.46 |
| Frontend | **Leptos** (client-side rendering) built with **Trunk** | 0.8.23 / 0.22.2 |
| Local LLM | **mistralrs**, running in a Quantrix data centre with no third-party AI API | 0.8.1 (built on candle-core 0.11.1) |
| Logging and tracing | tracing | 0.1.44 |
| Property tests | proptest | 1.12.0 |
| Dependency and licence audit | cargo-deny | 0.22.0 |
| Database server (infra) | PostgreSQL 18 (19.0 is a brand-new .0 release, so not for this sprint) | 18.7 |
| Event bus server (infra) | NATS server with JetStream | 2.15.3 |

- **Flagged, not used — stale crates.** `extended-isolation-forest` hasn't had a release since Nov 2022, so Amirtha writes the Isolation Forest herself (about 200 lines). `axum-login` 0.18 is from Jul 2025, so we use tower-sessions directly with a small RBAC layer.
- **Swapped from the source spec — Rust replacements.** Next.js becomes Leptos. Ollama becomes mistralrs. Redis, TimescaleDB, Kafka and Kubernetes are out of v0.1. `good_lp` comes in only if the MILP stretch goal happens.
- **All-Rust payoff — one types crate.** Console and server compile against the same `aeris-types` crate, so the API contract can't drift between them.

## Repository layout: algorithm crates do no I/O; apps do all of it

The dependency direction follows the source spec: the frontend talks only to the backend, AI is never on the execution path, and the backend contains no maths.

```text
aeris/
├── rust-toolchain.toml          1.100.0
├── Cargo.toml                   workspace
├── deny.toml                    cargo-deny policy
├── proto/aeris/v1/*.proto       event schema
├── migrations/                  sqlx migrations
├── deploy/compose.yaml          postgres:18 · nats:2.15 (JetStream) · apps
├── docs/spec/                   threat model · scenario spec · results.md
├── crates/                      pure logic: no network, no DB, deterministic with a seed
│   ├── aeris-types      K   domain + API types shared by server and console
│   ├── aeris-wire       D   prost wire types + conversions
│   ├── aeris-crypto     D   Ed25519 signing · blake3 hash chain · replay window
│   ├── aeris-sim        D   world model S' = F(S, a, W) · attacker · scenarios · rollouts
│   ├── aeris-estimator  A   Kalman filter + discrete-state HMM filter
│   ├── aeris-anomaly    A   MAD · EWMA · CUSUM · Isolation Forest
│   ├── aeris-fusion     A   Bayesian hypothesis fusion · correlation discount · robust aggregation
│   ├── aeris-trust      K   Beta trust vector · decay · uncertainty
│   ├── aeris-graph      KV  operational graph · reachability · blast radius
│   ├── aeris-mission    KV  capabilities · P(mission success) · criticality · risk · CVaR
│   ├── aeris-decision   K   action generator · utility J(a) · constrained argmax
│   ├── aeris-policy     KV  safety invariants · policy Π · autonomy gate H_t
│   ├── aeris-exec       D   Adapter trait · SimAdapter · verification · replan signal
│   ├── aeris-explain    A   template explainer · mistralrs explainer · grounding validator
│   └── aeris-core       K   Pipeline::step(events) -> CycleOutput
└── apps/
    ├── aeris-simd       D   simulator daemon: signed events out, commands in (NATS)
    ├── aeris-server     KV  axum control plane: ingest · pipeline · REST · WS · auth · audit
    ├── aeris-console    K   Leptos operator console
    └── aeris-eval       A+D headless harness: scenarios × seeds → metrics CSV + results.md

K = Kishanth · D = Dharshan · A = Amirtha · KV = Kesivarshini
```
