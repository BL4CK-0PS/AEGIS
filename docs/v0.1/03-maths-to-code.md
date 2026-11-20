# 03 · Maths → code

[← Index](README.md)

## What each crate implements in v0.1

This is the one-table summary of the source spec, cut down to what can be built and tested in ten days. Full equations, parameters and worked numbers are in [04 — Mathematical model](04-mathematical-model.md). Anything marked "later" waits for v0.2 (see [Scope cuts](08-rules-cuts-risks.md#scope-cuts-and-fallbacks)).

| Crate | v0.1 implements | Later | Owner |
|---|---|---|---|
| `aeris-sim` | S<sub>t+1</sub> = F(S<sub>t</sub>, a, W) at 10 Hz with a seeded ChaCha RNG. Attacker injects faults for S2–S8. `fork()` + Monte Carlo rollouts (N = 500, horizon H) per candidate action return outcome samples. | Discrete-event engine | DHARSHAN |
| `aeris-crypto` | Ed25519 signature per event; V(e) ∈ {0, 1}. Per-node monotonic sequence + replay window. blake3 hash chain over decision records. Server-signed decisions. | mTLS, PQC behind a trait | DHARSHAN |
| `aeris-estimator` | Linear Kalman filter on position and velocity per node. Each sensor's measurement noise R is inflated by 1/T<sub>j</sub><sup>obs</sup>, so a sensor AERIS distrusts counts for less. HMM forward filter over q ∈ {NORMAL, DEGRADED, SUSPICIOUS, COMPROMISED, ISOLATED, RECOVERING}. | UKF | AMIRTHA |
| `aeris-anomaly` | Robust z = (x − median) / (1.4826 · MAD). EWMA s<sub>t</sub> = λx<sub>t</sub> + (1−λ)s<sub>t−1</sub>. CUSUM g<sub>t</sub> = max(0, g<sub>t−1</sub> + x<sub>t</sub> − k), alarm when g > h. Isolation Forest trained on S1 normal runs. Every score lands in [0, 1] with an evidence ID. | Temporal autoencoder | AMIRTHA |
| `aeris-fusion` | P(H \| E) ∝ P(E \| H) · P(H) over {compromise, sensor failure, misconfiguration, environment, normal}. Correlation discount for evidence from the same source or feature. Median or trimmed mean for numeric readings. | Dempster–Shafer, factor graph | AMIRTHA |
| `aeris-trust` | Trust vector [identity, integrity, behaviour, telemetry, network, provenance]. Each dimension is a Beta(α, β) distribution: E = α/(α+β), plus its variance. Evidence is weighted by w = e<sup>−λΔt</sup> so trust can recover over time. Per-sensor trust is tracked separately from node trust. | Learned likelihoods | KISHANTH |
| `aeris-graph` | petgraph graph G = (V, E) with depends_on, communicates_with, routes_through and supports edges. Reach(v) by BFS. Blast radius B(v) = Σ C<sub>j</sub> over reachable nodes. | k-shortest attack paths | KESIVARSHINI |
| `aeris-mission` | Mission succeeds when every capability c<sub>i</sub> ≥ c<sub>i</sub><sup>min</sup>, also expressed as P(success \| S). Counterfactual criticality C<sub>i</sub> = (M(G) − M(G∖v<sub>i</sub>)) / M(G). Risk R = P(failure \| E) · f(C, B, M). CVaR<sub>α</sub> of loss computed from rollout samples. | Fault trees, Bayesian networks | KESIVARSHINI |
| `aeris-decision` | Feasible set A<sub>t</sub> ⊆ {monitor, restrict, isolate, reroute, failover, degrade, recover}, including combinations. J(a) = w<sub>M</sub>M − w<sub>R</sub>R − w<sub>C</sub>C − w<sub>D</sub>D − λ·CVaR<sub>α</sub>. Exhaustive argmax over actions that pass the hard constraints and P(fail \| a) ≤ ε. | MILP / CP-SAT, MPC | KISHANTH |
| `aeris-policy` | Safety monitor with hard invariants: never remove the last communication path; never isolate the only provider of a required capability without a commander. Policy Π = Auth ∧ Safety ∧ Mission ∧ Context. Autonomy gate H<sub>t</sub>(impact, uncertainty, reversibility, model health) returns AUTO, HUMAN_REVIEW or REJECT. | Policy DSL | KESIVARSHINI |
| `aeris-exec` | Γ(a) turns an action into commands through the `Adapter` trait; `SimAdapter` sends them over NATS. Verification compares observed state with the expected state: if ‖e‖ ≤ ε it's a success, otherwise replan. | Linux and network adapters | DHARSHAN |
| `aeris-explain` | The deterministic template explainer is built first and always works. The mistralrs explainer receives structured JSON only. A validator rejects any claim that cites an evidence ID or number not in the input, then falls back to the template. | RAG | AMIRTHA |

> **Non-negotiable.** AERIS must decide correctly with AI switched off (`--no-ai`), and CI tests that. AI explains. It never computes trust, picks an action or sits between policy and execution.
