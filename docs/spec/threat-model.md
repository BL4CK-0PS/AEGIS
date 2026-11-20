# AERIS v0.1.0 — One-Page Threat Model
**Owner:** KISHANTH  
**Milestone:** G0 Baseline  
**Scope:** Autonomous Node Cluster (Nodes A, B, C) over Multi-Hop Tactical Relay (Links L1, L2)

---

## 1. System Overview & Asset Inventory
AERIS governs an autonomous 3-node reconnaissance swarm (A, B, C). Node A and Node C communicate via Node B over primary link L1; a backup direct link L2 exists on standby.
- **Critical Assets:**
  - Flight safety and navigation capability ($c_{\text{nav}} \ge 0.85$)
  - Tactical communication capability ($c_{\text{comm}} \ge 0.90$)
  - Mission observation / reconnaissance capability ($c_{\text{obs}} \ge 0.80$)
  - Decision integrity and cryptographic audit chain.

---

## 2. Threat Actor Profile & Attacker Assumptions
- **Adversary Capability:**
  - **Node Compromise (Insider / Remote Code Execution):** Remote adversary compromises at most one node (Node B in scenario S4) via zero-day or credential brute-force.
  - **Sensor Spoofing:** Adversary can spoof or manipulate localized sensor readings (e.g., GPS spoofing, camera discrepancies, synthetic telemetry).
  - **Network Manipulation:** Adversary can drop, delay, replay, or inject arbitrary network packets on wireless links.
- **Assumptions & Bounds:**
  - Ed25519 signing keys of uncompromised nodes and the central control plane are secure.
  - No physical takeover of all three nodes simultaneously in v0.1.
  - Cryptographic primitives (Ed25519, Blake3) are secure.

---

## 3. Core Threats & Attack Vectors

| Threat ID | Attack Vector | Target & Mechanism | System Impact |
|---|---|---|---|
| **T-01** | **Single Node Compromise** | Node B: Rogue process spawned, unauthorized credential access, outbound beaconing to rogue C2. | Node B becomes untrusted relay. Eavesdropping or data manipulation risk. |
| **T-02** | **Sensor Disagreement / Spoofing** | Sensor readings (GPS / IMU / Camera) on Node A or B spoofed with false coordinates. | Distorts Kalman navigation state; risk of flight collision or mission failure. |
| **T-03** | **Event Replay Attack** | Intercepting valid signed telemetry from tick $t_0$ and replaying it at tick $t_k$ ($k > 0$). | Masks malicious activity by presenting outdated "healthy" telemetry. |
| **T-04** | **Event Signature Forgery** | Injecting synthesized telemetry attributing it to Node A or C without proper Ed25519 private key. | Attempts to poison belief state and force illegitimate node isolations (Denial of Service). |
| **T-05** | **Last Communication Path Severance** | An over-aggressive autonomous isolate action disconnecting Node B while L2 is inactive. | Mission communication is severed ($c_{\text{comm}} = 0$), failing mission immediately. |
| **T-06** | **Adversarial AI Manipulation** | Prompt injection or misleading evidence presented to explanation generator. | Tricking operator into approving unauthorized actions. |

---

## 4. Security Mitigations & Architectural Invariants

### 4.1 Cryptographic Identity & Anti-Replay
- **Per-Node Ed25519 Signatures:** Every event body is signed by the source node. Invalid signatures are rejected immediately ($V(e) = 0$).
- **Strict Monotonic Sequence & Replay Window:** Replayed events with duplicate or backwards sequence numbers $(source, seq)$ are dropped. Partition buffers (S7) carry an explicit `replayed = true` flag and strict timestamp ordering.

### 4.2 Multi-Sensor Anomaly & Hypothesis Fusion
- **Multi-Modal Detection:** Anomaly detectors (MAD z-score, EWMA, CUSUM, Isolation Forest) run independently on telemetry, network statistics, and sensor innovation.
- **Bayesian Hypothesis Fusion with Correlation Discount:** Correlated signals from the same process or interface are discounted ($w = 1/2$), preventing a single anomaly from spuriously generating 99% compromise confidence.

### 4.3 Multidimensional Beta Trust Vector
- Trust is partitioned into 6 distinct dimensions: $[\text{identity}, \text{integrity}, \text{behaviour}, \text{telemetry}, \text{network}, \text{provenance}]$.
- Compromised behaviour on Node B collapses behaviour and network trust to 0.44 and 0.22, collapsing geometric mean node trust to 0.565, even if its cryptographic identity key remains technically valid.
- Sensor trust $T_j^{\text{obs}}$ is separated from node trust, so a noisy or spoofed sensor does not cause unnecessary full-node isolation.

### 4.4 Invariant-Enforced Decision & Tail-Risk Optimization
- **Rule 1: Maths Decides, AI Explains.** Candidate actions are evaluated exclusively by Monte Carlo rollouts (500 futures) measuring expected utility $\mathbb E[U|a]$ and tail risk $\text{CVaR}_{0.9}(L|a)$.
- **Hard Invariant I1 (Safety Veto):** AERIS is mathematically forbidden from selecting an action that severs the last active communication path. Isolating Node B alone is vetoed unconditionally.
- **Combined Remediation:** AERIS generates composite actions: $\text{Isolate}(B) + \text{Reroute}(L2)$, restoring capability while neutralizing the threat.
- **Autonomy Gate:** High-impact actions ($C_i \ge 0.50$) or high uncertainty ($u_t \ge 0.25$) mandatorily require human operator approval.

### 4.5 Blake3 Tamper-Evident Audit Chain
- All decision records $D_t$ are cryptographically chained using Blake3 and signed by the server, ensuring post-incident forensic immutability.
