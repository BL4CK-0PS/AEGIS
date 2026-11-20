# 05 · Worked example: scenario S4, start to finish

[← Index](README.md)

This follows one run of the golden scenario through every stage, using the worked numbers from [04 — Mathematical model](04-mathematical-model.md). Timings are design targets, and the exact values will come from `aeris-eval`. Read it alongside the loop diagram in [00 — Start here](README.md#how-it-works-one-loop-ten-times-a-second).

| When | Stage / mode | What happens |
|---|---|---|
| t = 0 → 30 s | NORMAL | **Normal mission.** Nodes A, B and C fly with L1 active and L2 on standby. Every node's trust is about 0.90 and the capabilities are nav 0.93, comm 0.902, obs 0.920, all above their minimums, so $M(G) = 1.0$. Detectors learn their baselines. |
| t = 30.0 s | attack starts | **Attacker takes over B** (scenario file `s4.toml`). B starts beaconing to an unknown host, brute-forcing a local service, running a new process, and reporting camera positions that disagree with its GPS. |
| t = 30.1 s | stages 1–3 | **First evidence is ambiguous.** Events arrive signed and in order. The sensor-disagreement residual fires (E4). Fusion says normal 0.63, sensor failure 0.24, compromise 0.08: not enough to act on, and AERIS doesn't. |
| t = 30.3 → 30.5 s | stages 3–4 | **Independent evidence piles up.** The robust z-score on CPU hits 20.5 (E3). CUSUM alarms on packet rate (E1) and on auth failures (E2). Fusion, with E1 and E3 grouped as correlated, gives **compromise 0.947**. |
| t = 31.0 s | stages 5–7 · CONTAINMENT | **Trust collapses and the mission is at risk.** B's behaviour trust falls to 0.44 and its node trust to **0.565** (SUSPICIOUS). Communication over L1 becomes $0.99\times0.99\times0.565 = 0.554 < 0.90$, so the mission score drops to 0.615. Criticality $C_B = 1.0$ and risk $R_B = 0.987$, so Incident #1 opens and the mode becomes CONTAINMENT. |
| t = 31.1 → 31.3 s | stages 8–9 | **Simulate, then choose.** Five candidates are generated. "Isolate B only" is rejected by I1 without being scored. The other four get 500 rollouts each, and **isolate B + reroute via L2** wins with $J = 0.415$, the only option that passes the chance constraint. Because $C_B \ge 0.5$ its impact class is HIGH, so the gate says HUMAN_REVIEW. Target: under 500 ms from incident to recommendation. |
| t = 31.3 s | stage 10 | **Explanation.** The template explanation appears straight away, with evidence IDs. The LLM version replaces it if it passes the grounding check within 3 s. |
| t ≈ 40 s | stage 11 | **The commander approves.** The console shows the options table, the evidence, the graph path B → L1 → Communication → Mission, and the explanation. The commander presses APPROVE, which is recorded with operator, time and reason. |
| t ≈ 40 s | stages 12–13 | **Safety re-check, then execute.** Policy is re-evaluated on the current state and passes: I1 holds because the same action activates L2, and I2 holds because Communication is restored. Commands go out: activate L2, then remove B from routing and stop accepting B's key. |
| t ≈ 43 s | stage 14 · RECOVERY → NORMAL | **Verify and record.** Isolated(B) ✓, L2 active ✓, capabilities nav 0.930, comm 0.930, obs 0.815, all above minimum ✓, so $M(G) = 1.0$ and verification succeeds. After 10 s with $R_{\max} < 0.3$ the mode returns to NORMAL. Decision record $D_1$ is hash-chained and signed. |

> This is the loop the G2 integration test runs headless across 20 seeds, and the G4 evaluation runs across 100.
