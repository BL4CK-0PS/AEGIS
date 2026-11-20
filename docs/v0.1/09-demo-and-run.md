# 09 · Demo script and how to run

[← Index](README.md)

## The four-minute demo

| Time | On screen | What the presenter says |
|---|---|---|
| 0:00 | Mission overview, all green, graph A–B–C on L1 | "Three autonomous nodes on a surveillance mission. AERIS is watching trust, not just alerts." |
| 0:30 | Start S4: B is compromised | "We've just taken over node B." |
| 1:00 | Evidence list fills in. Posterior moves from "sensor failure?" to "compromise 0.95" | "One signal is ambiguous. Four independent signals aren't." |
| 1:20 | B's trust bar drops to 0.57, and the uncertainty band is visible | "Trust is a probability with error bars, not a red light." |
| 1:40 | Path B → L1 → Communication → Mission is highlighted; comm 0.55, below its 0.90 minimum | "B is the only relay, so this is a mission problem, not just a security one." |
| 2:00 | Risk 0.99 · mode CONTAINMENT | — |
| 2:20 | Options table: success, CVaR and score per action. "Isolate only" is struck out under rule I1 | "We simulated 500 futures per option. Isolating alone would cut our last link." |
| 2:40 | Recommendation: ISOLATE B + REROUTE L2 (0.94), with the explanation and evidence IDs | "The AI explains. It didn't decide." |
| 3:00 | Commander presses APPROVE | "High-impact actions always need a human." |
| 3:20 | Graph changes: B dashed, L2 solid | — |
| 3:40 | Verification ticks: isolated ✓, L2 ✓, capabilities ✓ | "AERIS checks that its action actually worked." |
| 4:00 | Before/after: risk and mission score, plus the results table from `aeris-eval` | "Across 100 seeds: [real MSR] for AERIS vs [real] for alert-only." Numbers come from `results.md`. |

## Run it locally

```bash
# once
rustup update                       # Rust 1.100.0 (rust-toolchain.toml pins it)
cargo install trunk --locked        # Leptos build tool
rustup target add wasm32-unknown-unknown

# every time
docker compose -f deploy/compose.yaml up -d        # Postgres 18 + NATS 2.15 (JetStream)
cargo run -p aeris-server                          # control plane on :8080
cargo run -p aeris-simd -- --scenario s4 --seed 7  # simulated world
(cd apps/aeris-console && trunk serve)             # console on :8081

# headless and evaluation
cargo run -p aeris-eval -- --scenario s4 --seed 7 --step       # narrated loop, approve on stdin
cargo run -p aeris-eval -- --suite all --seeds 100             # writes docs/spec/results.md + CSV
cargo run -p aeris-eval -- verify-audit                        # checks the hash chain

# before every PR
cargo fmt --all && cargo clippy --workspace -- -D warnings && cargo test --workspace && cargo deny check
```

*Secrets (node keys, server signing key, DB password) come from environment variables or files mounted at runtime, listed in `deploy/.env.example` with dummy values. Never commit the real ones.*
