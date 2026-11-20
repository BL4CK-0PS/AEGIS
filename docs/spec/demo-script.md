# AERIS v0.1.0 — Demo Playbook & Script
**Owner:** KISHANTH  
**Format:** 4-minute live run of Scenario S4 (Golden Path: Node B Compromise & Recovery)  
**Deliverable:** G3 & G5 rehearsal script

---

## 1. Setup & Pre-Flight Checklist
- Local deployment running or standalone console:
  - Backend control plane on `http://localhost:8080` (or `cargo run -p aeris-console`)
  - Simulated edge world running on seed 7 (`s4.toml`)
  - Operator console running on `http://localhost:8081`

---

## 2. Four-Minute Rehearsal Timeline

| Timestamp | Visual Display | Presenter Voiceover Script | Action / Cue |
|---|---|---|---|
| **00:00 - 00:30** | Mission overview: All green. Operational graph shows Node A, Node B, Node C active over primary link L1. Link L2 is standby (dashed). Capabilities: Nav = 0.930, Comm = 0.902, Obs = 0.920 ($M(G) = 1.000$). | *"Welcome. This is AERIS, an autonomous decision engine for mission-critical multi-node swarms. Right now, three reconnaissance drones—A, B, and C—are executing a joint surveillance mission. AERIS continuously tracks multidimensional trust and mission capability, not just raw alerts."* | Point out L1 active relay and capability score gauges. |
| **00:30 - 01:00** | Inject Scenario S4 fault schedule. Attacker gains RCE on Node B. Network outbound packet spike, rogue SSH brute-force, high CPU process, and camera sensor disagreement. | *"At t = 30 seconds, an adversary compromises Node B. The node begins beaconing to a rogue C2, brute-forcing local credentials, and reporting discordant camera readings."* | Trigger S4 injection. Evidence feed begins populating with `ev-0412`, `ev-0415`, `ev-0419`, `ev-0423`. |
| **01:00 - 01:20** | Incident #1 opens. Posterior probability updates from initial ambiguity to Compromise = 94.7% (with correlation discount applied). | *"A single anomaly is ambiguous—maybe a noisy sensor or bad weather. But four independent signals spanning network, authentication, and execution pile up. Bayesian hypothesis fusion discounts correlated signals and isolates the true cause: compromise with 95% certainty."* | Highlight posterior breakdown bar. |
| **01:20 - 01:40** | Node B's trust vector collapses from 0.90 to 0.565 ($\text{Beta}(8.0, 3.4)$ with $\pm 0.13$ uncertainty band). Status badge shifts from NORMAL to SUSPICIOUS. | *"Notice that trust is not a binary red light. It is a Beta probability distribution with explicit uncertainty bounds. B's behaviour and network trust collapse, pulling down its geometric mean node trust to 0.57."* | Show trust vector breakdown and variance. |
| **01:40 - 02:00** | Graph highlights critical path: B $\to$ L1 $\to$ Communication capability. Communication capability drops to 0.554 (below 0.90 minimum). Overall Mission Score drops to 0.615. Counterfactual Criticality $C_B = 1.00$, Risk $R_B = 0.987$. Operating mode shifts to CONTAINMENT. | *"Because B is the sole relay on Link L1, communication across the swarm collapses to 0.55. Counterfactual criticality registers 1.0: removing B without a substitute kills the mission. Operating mode immediately escalates to CONTAINMENT."* | Highlight red communication capability bar. |
| **02:00 - 02:40** | Decision Proposal appears: Candidate table shows 5 options. 500 Monte Carlo rollouts simulated per option. Option 'Isolate B alone' is struck out (vetoed by Hard Invariant I1). 'Isolate B + Reroute via L2' ranks #1 with $P(\text{success}) = 0.94$, $J(a) = 0.415$. Grounded explanation displayed citing evidence IDs. Autonomy Gate flags HUMAN_REVIEW. | *"AERIS doesn't guess commands using an LLM. It generates candidate actions and runs 500 counterfactual rollouts for each. Isolating B alone is vetoed by hard safety invariant I1—it would sever our last link. The optimizer chooses Isolate B plus Reroute via L2, yielding a 94% success rate and minimal tail risk. Because this is a high-impact action on a critical node, the autonomy gate requires human confirmation."* | Emphasize struck-out invariant violation and rollouts CVaR. |
| **02:40 - 03:20** | Presenter moves mouse to console and clicks green **APPROVE** button. | *"The AI explains the evidence; the maths proves the outcome; but the commander retains authority. We click APPROVE."* | Click **APPROVE**. |
| **03:20 - 03:40** | Execution & Safety Re-check: Policy validates current state. Commands dispatched: Activate Link L2, Isolate Node B. Graph updates dynamically: Node B circle becomes dashed red, Link L2 becomes solid blue. | *"Commands execute: backup link L2 activates, and Node B is isolated from routing. The operational graph reflects the new topology in real time."* | Watch graph topology redraw. |
| **03:40 - 04:00** | Verification & Recovery: Observed state checked against expected rollout. Capabilities verified: Nav = 0.930, Comm = 0.930, Obs = 0.815 ($M(G) = 1.000$). Incident #1 marked CLOSED. Mode transitions to RECOVERY. | *"AERIS verifies that the real system matches the predicted state: B is isolated, L2 is active, all capabilities are restored above minimums, and mission score returns to 1.0. The incident closes, and the mission continues safely."* | Show 100% mission recovery and before/after metrics. |

---

## 3. Fallback Procedures During Live Demo
1. **Network or Display Delay:**
   - Fall back to headless console narration:
     ```bash
     cargo run -p aeris-console -- --demo
     ```
2. **AI Service Timeout:**
   - Grounded template explainer runs automatically without external LLM latency (< 5ms).
