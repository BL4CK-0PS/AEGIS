# 01 · Definition of done

[← Index](README.md)

## One loop, end to end, with no hardcoded numbers

v0.1.0 is done when both `aeris-eval --scenario s4` (headless) and the browser console run this loop with no manual steps except the human approval:

```text
Normal mission → inject compromise on Node B → detect anomaly → estimate state
→ update trust → mission impact → risk → generate candidate actions
→ simulate them (Monte Carlo) → pick the best feasible action → explain
→ human APPROVE → safety / policy check → execute → verify → mission recovered
```

Every number shown in the demo comes from the simulator. The figures in the source spec (trust 0.91→0.19, risk 82%→11% and so on) show the expected direction only. They are not targets, and no code or test may hardcode them.

## Simulated world (frozen on D0)

| | | |
|---|---|---|
| **Nodes** | A · B · C | Each node has GPS, IMU and camera sensors, plus an Ed25519 identity key, a software state and network stats. |
| **Links** | L1 and L2 | L1 is A–B–C, relayed through B. L2 is a direct A–C backup link. |
| **Mission** | Three capabilities | Navigation ≥ 0.85, Communication ≥ 0.90, Observation ≥ 0.80. |
| **Why it's honest** | The answer has to emerge | Isolating B alone breaks Communication. Rerouting alone keeps the compromised B in the loop. So *isolate B + reroute via L2* has to win on the computed outcomes, not because someone scripted it. |

## Scenario suite (`aeris-eval`)

| ID | Scenario | Expected AERIS behaviour |
|---|---|---|
| S1 | Normal operation | No incident. Measures the false-positive rate. |
| S2 | GPS failure on A | Trust in A's GPS drops and the estimator down-weights it. No isolation. |
| S3 | L1 latency and packet loss | Network degradation is detected and a reroute is recommended. |
| **S4** | **Node B compromised (golden path)** | **Isolate B + reroute via L2. Human approves. Mission recovers.** |
| S5 | False telemetry from B | Telemetry trust falls while identity trust stays high. |
| S6 | S2 and S4 at the same time | Two incidents, each gets the correct action. |
| S7 | A–C partition for 20 s | Events are buffered and replayed. Nothing is processed twice. |
| S8 | Model mismatch | Model health degrades, autonomy drops, every action goes to human review. |

> **Acceptance gate:** on S4 the mission completes in 100% of runs across 100 seeds. `aeris-eval` measures this; nobody asserts it.
