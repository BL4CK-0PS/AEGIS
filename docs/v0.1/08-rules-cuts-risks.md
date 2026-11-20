# 08 · Working rules, scope cuts and risks

[← Index](README.md)

## Working rules: how we work for ten days

| Area | Rule | Detail |
|---|---|---|
| Rhythm | **Standup and merge window** | 10:00 standup, 15 minutes: done, next, blocked. 18:00 integration: pull main, run `cargo test --workspace` and `aeris-eval --scenario s4`. Main stays green. |
| Pull requests | **Small, reviewed, green** | Short-lived branches, one reviewer, CI green before merge. Any change to `aeris-types` or the proto files needs Kishanth's review. |
| Per-crate done | **Tested and deterministic** | Every function with logic has a unit test. Clippy is clean and the public API has doc comments. Same seed, same output. No `unwrap()` on server paths. |
| Security | **Secrets and auth** | Node keys, the server signing key and DB credentials come from environment variables or mounted secret files and are never committed. Authorization is checked on every request, scoped to the specific object. The console never computes trust; the server owns the truth. |
| Honest numbers | **No demo fiction** | Tests assert that the chosen action is the argmax of the computed outcomes, across seeds. No expected value from the source spec appears in the code. The demo's figures come from `results.md`. |
| Hosting | **Quantrix data centre** | The server, LLM and database run on Quantrix-owned machines, not local-only and with no third-party AI API. Only the simulated edge nodes run in `aeris-simd`. |

## Scope cuts and fallbacks

**Not built by 30 Nov (v0.2 backlog):** UKF (v0.1 uses a linear Kalman filter), MILP/CP-SAT, MPC, Dempster–Shafer, k-shortest attack paths, the full recovery state machine, Redis, TimescaleDB, Kafka, Kubernetes, mTLS between services, a real edge agent on hardware, PQC, RAG, learning from human feedback, and Canopus OS / Aquila integration (that's the Q3 2027 roadmap).

### If a gate slips, cut in this order

| # | Cut | What replaces it |
|---|---|---|
| 1 | S7 partition replay | Show S1–S6 and S8 only. |
| 2 | "Ask why" Q&A | A static explanation panel only. |
| 3 | mistralrs LLM | The template explainer, which is already built first. |
| 4 | Isolation Forest | MAD + EWMA + CUSUM only. |
| 5 | NATS | In-process tokio channels behind the same `EventBus` trait. Decide at G1. |
| 6 | Leptos console | Run the demo headless with `aeris-eval --scenario s4 --step`, printing each stage and taking the approval on stdin. |

*Never cut: the simulator, trust, graph and mission, risk, rollouts, optimizer, policy and safety, human approval, execution and verification. Those are the product.*

## Top risks: what's most likely to break the plan

| Risk | Mitigation | Owner |
|---|---|---|
| Integration crunch: crates work alone but not together | Walking skeleton at G1 with stubs, a daily 18:00 merge window, and types frozen at G0. | KISHANTH |
| Optimizer picks the "wrong" action on S4 | Fix the world model, not the answer. Check that the rollouts reflect L1's dependence on B. Tune weights only with Kishanth and Kesivarshini both present. | KISHANTH · DHARSHAN |
| Too many false positives on S1 | Tune on D5 against S1 × 100 seeds; the discount for correlated evidence prevents false certainty. | AMIRTHA |
| mistralrs or GPU build problems in the DC | Smoke test on D0, go/no-go at G1, template fallback always available. | AMIRTHA |
| Leptos learning curve | Plain SVG and CSS with no component library, built against fixtures from D4, and the headless fallback. | KISHANTH |
| Overengineering (named the #1 risk in the source spec) | Anything not in [03](03-maths-to-code.md)/[04](04-mathematical-model.md) waits for v0.2. "Just one more module" is a no until v0.1.0 is tagged. | ALL |
