# 04 · The full mathematical model

[← Index](README.md)

## Every equation AERIS v0.1 uses, explained

This is the source of truth for every algorithm crate. Each block gives the maths, then a plain-English line, then the default parameters we freeze on D0. If your code disagrees with this section, either the code is wrong or this section needs a PR.

- [6.0 Notation](#60-notation)
- [6.1 The whole system as one decision problem](#61-the-whole-system-as-one-decision-problem)
- [6.2 Events, integrity and ordering](#62-events-integrity-and-ordering)
- [6.3 State estimation](#63-state-estimation)
- [6.4 Anomaly detection](#64-anomaly-detection)
- [6.5 Evidence fusion](#65-evidence-fusion)
- [6.6 Trust](#66-trust)
- [6.7 Operational graph and capabilities](#67-operational-graph-and-capabilities)
- [6.8 Mission and criticality](#68-mission-and-criticality)
- [6.9 Risk and tail risk (CVaR)](#69-risk-and-tail-risk-cvar)
- [6.10 Actions and counterfactual rollouts](#610-actions-and-counterfactual-rollouts)
- [6.11 Decision optimization](#611-decision-optimization)
- [6.12 Autonomy gate, policy and safety](#612-autonomy-gate-policy-and-safety)
- [6.13 Execution, verification and recovery](#613-execution-verification-and-recovery)
- [6.14 AI explanation with grounding](#614-ai-explanation-with-grounding)
- [6.15 Decision record](#615-decision-record-audit-and-provenance)
- [6.16 Evaluation metrics](#616-evaluation-metrics)

---

## 6.0 Notation

| Symbol | Meaning | Symbol | Meaning |
|---|---|---|---|
| $t$ | time step (100 ms tick, 10 Hz) | $i, j$ | an entity (node / sensor / link), or a sensor index |
| $S_t$ | true hidden state of the world | $O_t$ | observations received at $t$ |
| $b_t$ | belief: probability distribution over $S_t$ | $\hat X_t$ | estimated continuous state (position, velocity) |
| $A_t^{k}$ | anomaly score of feature $k$, in $[0,1]$ | $E_t$ | set of evidence items |
| $H$ | a hypothesis (normal, compromise, …) | $T_i$ | trust of entity $i$, in $[0,1]$ |
| $G=(V,E)$ | operational graph: vertices, edges | $c_k$, $c_k^{\min}$ | capability level and its required minimum |
| $M(G)$ | mission score, in $[0,1]$ | $C_i$ | criticality of entity $i$ |
| $R_i$ | risk of entity $i$ | $a$, $\mathcal A_t$ | an action; the feasible action set at $t$ |
| $L(a)$ | loss if we take action $a$ (random) | $J(a)$ | score of an action; we pick the largest |
| $\alpha$ | CVaR tail level (0.9 = worst 10%) | $\lambda$ | tail-risk aversion weight |
| $\varepsilon$ | allowed failure probability or tolerance | $N$, $K$ | Monte Carlo rollouts per action; rollout horizon in ticks |

## 6.1 The whole system as one decision problem

Formally, AERIS solves a **risk-sensitive, constrained, partially observable decision problem**. "Partially observable" means we never see the truth directly, only noisy and possibly manipulated readings.

```math
\mathcal{P} = (\mathcal S,\ \mathcal A,\ \mathcal O,\ F,\ Z,\ U,\ \mathcal C,\ \mathcal H)
```

*States, actions, observations, how the world moves ($F$), how readings are produced ($Z$), how good an outcome is ($U$), the constraints ($\mathcal C$) and the human-governance rules ($\mathcal H$).*

**World state** is a bundle of everything that matters:

```math
S_t = (X_t,\ G_t,\ M_t,\ C^{\text{cyber}}_t,\ T_t,\ \Theta_t,\ \Omega_t)
```

*Physical state, operational graph, mission state, cyber state, trust, model parameters and operator state.*

**How the world evolves**, under action $a_t$ with random disturbance $W_t$ (wind, noise, the attacker):

```math
S_{t+1} = F_\theta(S_t,\ a_t,\ W_t)
```

**How we observe it.** This is the adversarial version: readings are true signal plus noise plus a possible attack term.

```math
O_t = h_\theta(S_t) + V_t + A^{\text{adv}}_t, \qquad V_t \sim \mathcal N(0, R)
```

*A spoofed GPS is exactly a non-zero $A^{\text{adv}}_t$. AERIS must work out when that term is probably present.*

**What we maintain**, the belief state:

```math
b_t(s) = P\big(S_t = s \mid O_{1:t},\ a_{1:t-1}\big)
```

*"Given everything seen and done so far, how likely is each possible true state?" Reality, observation and belief are three different things.*

**What we decide**, the master equation that §6.11 builds up piece by piece:

```math
a_t^{*} = \arg\max_{a \in \mathcal A_t}\ \Big( \mathbb E_{b_t}\big[U(S,a)\big] \;-\; \lambda\,\mathrm{CVaR}_\alpha\big(L(S,a)\big) \Big) \quad \text{s.t.}\ \ g_h(S,a)\le 0,\ \ P\big(g_r(S,a) > 0\big)\le\varepsilon,\ \ \Pi(S,a)=1
```

*Pick the allowed action with the best expected mission outcome, penalising bad worst cases, while never breaking a hard rule, keeping the failure probability small, and passing policy.*

## 6.2 Events, integrity and ordering

**Owner:** DHARSHAN · `aeris-wire`, `aeris-crypto`

Every reading becomes a signed **event**:

```math
e = (\text{id},\ \text{source},\ \text{seq},\ \text{hlc},\ \text{type},\ \text{payload},\ \text{confidence},\ \text{provenance},\ \sigma)
```

**Signature.** Each simulated node $i$ has an Ed25519 key pair $(sk_i, pk_i)$. We sign the exact bytes we send, not a re-encoded struct, because Protobuf encoding isn't guaranteed to be byte-identical across encoders:

```math
\sigma = \mathrm{Sign}_{sk_i}\big(\text{bytes}(e \setminus \sigma)\big), \qquad V(e) = \mathrm{Verify}_{pk_i}\big(\text{bytes}, \sigma\big) \in \{0,1\}
```

**Replay protection.** Accept an event only if it's newer than the last one from that source and fresh:

```math
\text{accept}(e) \iff V(e)=1 \ \wedge\ \text{seq}_e > \text{last}[\text{source}_e] \ \wedge\ |t_e - t_{\text{now}}| \le \Delta_{\max}
```

*A replayed or forged event is rejected **and** recorded as integrity evidence against its claimed source, which lowers that source's identity trust. v0.1: $\Delta_{\max} = 2$ s, except during the S7 partition replay, where buffered events are flagged and judged on sequence number alone.*

**Ordering with a hybrid logical clock (HLC).** An HLC keeps event order consistent even when node clocks drift. Each node keeps $(l, c)$: $l$ is the largest physical time seen, $c$ a counter. With $pt$ the local physical clock:

```math
\text{send/local: } l' = \max(l, pt),\quad c' = \begin{cases} c+1 & l' = l \\ 0 & \text{otherwise}\end{cases}
```

```math
\text{receive } (l_m,c_m)\text{: } l' = \max(l, l_m, pt),\quad c' = \begin{cases} \max(c, c_m)+1 & l'=l=l_m \\ c+1 & l'=l\ne l_m \\ c_m+1 & l'=l_m\ne l \\ 0 & \text{otherwise}\end{cases}
```

*The server orders events by $(l, c, \text{source})$ and drops duplicates by $(\text{source}, \text{seq})$. The HLC is D7 work; until it lands, the server orders by $(t_e, \text{seq})$.*

**Tamper-evident audit chain.** Every decision record $D_n$ is chained:

```math
h_0 = 0^{256}, \qquad h_n = \mathrm{BLAKE3}\big(h_{n-1} \,\|\, \text{bytes}(D_n)\big), \qquad \sigma_n = \mathrm{Sign}_{sk_{\text{srv}}}(h_n)
```

*Changing any old record changes its hash and breaks every later link, so `aeris-eval verify-audit` catches it.*

## 6.3 State estimation

**Owner:** AMIRTHA · `aeris-estimator`

**Continuous state: a Kalman filter per node.** Per axis, the state is position and velocity, $x = [p,\ v]^\top$, with a constant-velocity model:

```math
F = \begin{bmatrix} 1 & \Delta t \\ 0 & 1 \end{bmatrix},\qquad H = \begin{bmatrix} 1 & 0 \end{bmatrix},\qquad Q = q\begin{bmatrix} \Delta t^3/3 & \Delta t^2/2 \\ \Delta t^2/2 & \Delta t \end{bmatrix}
```

**Predict** (once per tick, using the IMU-driven motion model):

```math
\hat x^- = F\hat x, \qquad P^- = F P F^\top + Q
```

**Update**, once per sensor $j$ (GPS, camera fix) with measurement $z_j$:

```math
\nu_j = z_j - H\hat x^-,\quad S_j = HP^-H^\top + R_j,\quad K_j = P^-H^\top S_j^{-1},\quad \hat x = \hat x^- + K_j\nu_j,\quad P = (I - K_jH)P^-
```

*$\nu_j$ (the innovation) is "how surprised we are by this reading". $K_j$ (the Kalman gain) decides how much to believe it.*

**The AERIS twist: trust-weighted noise.** A sensor we distrust is treated as noisier, so it counts for less:

```math
R_j = \frac{\sigma_j^2}{\max\big(T^{\text{obs}}_j,\ T_{\min}\big)}, \qquad T_{\min} = 0.05
```

**Outlier gate.** A reading whose normalised innovation squared is absurd is not fused at all; it becomes evidence against the sensor:

```math
\epsilon_j = \nu_j^\top S_j^{-1} \nu_j \sim \chi^2_m, \qquad \text{reject if } \epsilon_j > \chi^2_{m,\,0.99} \ \ (= 9.21 \text{ for } m=2)
```

> **Worked example: a spoofed GPS loses its vote.**
> The IMU prediction is $\hat x^- = 100.0$ m with variance $P^- = 9$. The GPS says $120.0$ m ($\sigma^2 = 9$); the camera says $101.5$ m ($\sigma^2=16$). Fusing all three in information form, $\hat x = \dfrac{\hat x^-/P^- + \sum_j z_j/R_j}{1/P^- + \sum_j 1/R_j}$:
>
> - With equal trust ($T=1$): $\hat x = 108.13$ m. The GPS carries 39% of the weight and drags the estimate 8 m off.
> - With $T_{\text{gps}} = 0.21$ and $T_{\text{cam}} = 0.89$: $\hat x = 102.89$ m. The GPS weight drops to 12.3%, and the estimate stays near the honest sensors.
>
> A bad sensor no longer gets to define reality.

**Model health** comes from the same innovations. If the filter keeps being surprised, our model of the world is wrong:

```math
\text{health}_t = \frac{1}{W}\sum_{\tau=t-W+1}^{t} \mathbb 1\big[\epsilon_\tau \le \chi^2_{m,\,0.95}\big], \qquad \text{DEGRADED if } \text{health}_t < 0.90 \ \ (W = 50)
```

*For a consistent filter about 95% of innovations fall inside the bound. Far fewer means model mismatch (scenario S8), and §6.12 then lowers autonomy.*

**Discrete state: an HMM forward filter per node.** Each node's operating state is one of six:

```math
q_t \in \{\textsf{NORMAL},\ \textsf{DEGRADED},\ \textsf{SUSPICIOUS},\ \textsf{COMPROMISED},\ \textsf{ISOLATED},\ \textsf{RECOVERING}\}
```

```math
b_t(q) = \frac{P(o_t \mid q)\ \sum_{q'} P(q \mid q')\, b_{t-1}(q')}{\sum_{r} P(o_t \mid r)\ \sum_{q'} P(r \mid q')\, b_{t-1}(q')}
```

*Predict with the transition matrix $P(q \mid q')$ (sticky: 0.97 on the diagonal), then reweight by how well each state explains today's fused evidence $o_t$, then normalise. ISOLATED and RECOVERING are entered only by executed actions, never inferred.*

## 6.4 Anomaly detection

**Owner:** AMIRTHA · `aeris-anomaly`

No single detector catches every kind of anomaly, so we run four in parallel on each feature (CPU, packet rate, auth failures, sensor residuals and so on) and keep the strongest signal. Every detector outputs a score in $[0,1]$ plus an evidence ID.

**1. Robust z-score (median + MAD)** catches sudden spikes. It uses the median and the median absolute deviation instead of the mean and standard deviation, so the spike itself can't distort the baseline:

```math
z_t = \frac{x_t - \operatorname{median}(X)}{1.4826 \cdot \operatorname{MAD}(X)}, \qquad \operatorname{MAD}(X) = \operatorname{median}\big(|X - \operatorname{median}(X)|\big)
```

*1.4826 makes MAD match the standard deviation for normal data. $X$ is a rolling window of the last 300 samples (30 s).*

**Mapping any detector to a score in [0,1]:**

```math
A_t = \frac{1}{1 + e^{-\kappa(|z_t| - z_0)}}, \qquad z_0 = 3,\ \kappa = 1.5
```

> **Worked example: CPU spike on B.** The last eight CPU readings were 41, 43, 40, 44, 42, 45, 39, 43, so the median is 42.5 and MAD is 1.5. A new reading of 88% gives $z = (88-42.5)/(1.4826\times1.5) = 20.46$, and the score is $A \approx 1.00$. That's an unmistakable spike.

**2. EWMA** (exponentially weighted moving average) catches slow drift that never spikes:

```math
s_t = \lambda x_t + (1-\lambda)s_{t-1}, \qquad \text{alarm if } |s_t - \mu_0| > L\,\sigma\sqrt{\tfrac{\lambda}{2-\lambda}} \quad (\lambda=0.2,\ L=3)
```

> **Worked example.** With baseline $\mu_0 = 42.5$ and $\sigma = 2.22$, the alarm limit is 44.72. CPU creeps 44 → 47 → 52 → 58 → 63, and the EWMA goes 42.80, 43.64, **45.31** (alarm), 47.85, 50.88. Drift is flagged on the third sample.

**3. CUSUM** (cumulative sum) catches a small but persistent shift, such as a steady trickle of failed logins:

```math
g_t = \max\big(0,\ g_{t-1} + (x_t - \mu_0) - k\big), \qquad \text{alarm if } g_t > h \quad (k = 0.5,\ h = 4)
```

> **Worked example: auth failures per second on B.** With baseline $\mu_0=0.2$, the readings 0, 1, 0, 3, 4, 5, 4 give $g$ = 0, 0.3, 0, 2.3, **5.6**, 9.9, 13.2. The alarm fires on the fifth reading, and $g$ keeps growing while the attack continues.

**4. Isolation Forest** catches combinations that are each normal alone but abnormal together (say, medium CPU *and* medium packet rate *and* a new process). It builds 100 random trees, each from a 256-sample subset, and isolates points with random splits. Anomalies get isolated in fewer splits:

```math
s(x) = 2^{-\,\mathbb E[h(x)]\,/\,c(n)}, \qquad c(n) = 2\big(\ln(n-1) + 0.5772\big) - \frac{2(n-1)}{n}
```

*$h(x)$ is the path length to isolate $x$; $c(n)$ is the average path length for $n$ points, used to normalise. Scores near 1 are anomalous; scores near 0.5 are normal.*

> **Worked example.** For $n = 256$, $c(256) = 10.245$. A point isolated in 3.2 splits on average scores $2^{-3.2/10.245} = 0.805$ (anomalous). One that needs 10 splits scores 0.508 (normal). Trained on S1 normal runs only, so no labelled attack data is needed.

**Context matters.** Baselines are kept per node and per mission phase, because normal behaviour at launch isn't normal behaviour in cruise.

## 6.5 Evidence fusion

**Owner:** AMIRTHA · `aeris-fusion`

Fusion turns many separate scores into one question: *what is most likely going on?* For each node there are five hypotheses: $H \in$ {normal, compromise, sensor failure, misconfiguration, environment}. Bayes' rule:

```math
P(H \mid E) = \frac{P(H)\ \prod_k P(E_k \mid H)^{\,w_k}}{\sum_{H'} P(H')\ \prod_k P(E_k \mid H')^{\,w_k}}
```

*Start from a prior belief $P(H)$ and multiply by how well each hypothesis explains each piece of evidence. Implement it in log space, $\log P(H\mid E) = \log P(H) + \sum_k w_k \log P(E_k\mid H) - \log Z$, so small numbers don't underflow.*

**The correlation trap.** Four detectors firing on the same underlying event aren't four independent witnesses, and treating them as independent produces false certainty. v0.1 uses **tempered likelihoods**: evidence items judged to be views of the same event (same source and same time window) form a group $g$, and each gets weight

```math
w_k = \frac{1}{|g|} \quad \text{(independent evidence: } w_k = 1\text{)}
```

**Robust aggregation of numbers.** When several sources report the same quantity and some may be lying, use a statistic one liar can't move far:

```math
\hat x = \operatorname{median}(x_1,\dots,x_n) \quad\text{or}\quad \hat x = \tfrac{1}{n-2m}\textstyle\sum_{i=m+1}^{n-m} x_{(i)} \ \ \text{(trimmed mean, drop } m \text{ from each end)}
```

> **Worked example: why fusion beats single alerts (Node B).**
> Priors: normal 0.95, compromise 0.01, sensor failure 0.02, misconfiguration 0.015, environment 0.005. Likelihoods $P(E_k=1\mid H)$ are frozen in `docs/spec/likelihoods.toml`; for compromise they are network 0.85, auth 0.70, process 0.75, sensor 0.60.
>
> | Evidence so far | normal | compromise | sensor fail | misconfig | environment |
> |---|---|---|---|---|---|
> | E4 sensor disagreement only | **0.629** | 0.079 | 0.238 | 0.020 | 0.033 |
> | + E1 network anomaly | 0.251 | **0.538** | 0.095 | 0.063 | 0.053 |
> | + E2 auth failures | 0.012 | **0.935** | 0.002 | 0.047 | 0.003 |
> | + E3 process anomaly (naive, $w=1$) | 0.001 | **0.986** | 0.000 | 0.013 | 0.000 |
> | + E3, with E1 and E3 grouped ($w=\tfrac12$) | 0.013 | **0.947** | 0.003 | 0.036 | 0.001 |
>
> One signal alone is ambiguous: "normal, or maybe a broken sensor". Independent signals together point clearly to compromise. The correlation discount keeps us from claiming 98.6% certainty from evidence that partly repeats itself.

## 6.6 Trust

**Owner:** KISHANTH · `aeris-trust`

Trust is the probability that an entity is healthy. We store it as a **Beta distribution** rather than a single number, so we always know how *sure* we are:

```math
T \sim \mathrm{Beta}(\alpha, \beta), \qquad \mathbb E[T] = \frac{\alpha}{\alpha+\beta}, \qquad \mathrm{Var}[T] = \frac{\alpha\beta}{(\alpha+\beta)^2(\alpha+\beta+1)}
```

*$\alpha$ counts "good behaviour" evidence and $\beta$ counts "bad". "0.82 with low variance" and "0.82 with high variance" mean very different things to the decision engine.*

**Update with forgetting**, every tick with anomaly-derived badness $s_t \in [0,1]$ and evidence weight $w$:

```math
\alpha_t = \gamma\,\alpha_{t-1} + w(1 - s_t), \qquad \beta_t = \gamma\,\beta_{t-1} + w\,s_t, \qquad \gamma = e^{-\lambda_d\,\Delta t}
```

*$\gamma$ slowly forgets old evidence, so trust can recover after sustained good behaviour and never stays stuck forever. v0.1: $\gamma = 0.95$ per tick, $w = 1$, prior $\mathrm{Beta}(9,1)$ (trust 0.90).*

**Six trust dimensions per node**, each its own Beta:

```math
\mathbf T_i = \big[\,T_{\text{identity}},\ T_{\text{integrity}},\ T_{\text{behaviour}},\ T_{\text{telemetry}},\ T_{\text{network}},\ T_{\text{provenance}}\,\big]
```

**Aggregation** uses a weighted geometric mean, so one very bad dimension pulls the total down harder than an average would:

```math
T_i = \prod_{d} T_{i,d}^{\,\omega_d}, \qquad \textstyle\sum_d \omega_d = 1 \ \ (\text{v0.1: } \omega_d = \tfrac16)
```

> **Worked example: B's behaviour trust collapses, then the node total.**
> Starting from Beta(9, 1), trust 0.900. Behaviour evidence is $s = 0.9$ every tick:
>
> - after 3 ticks (0.3 s): $\alpha = 8.00,\ \beta = 3.43$, trust 0.700 ± 0.130
> - after 6 ticks: trust 0.565 ± 0.134
> - after 10 ticks (1 s): trust **0.442** ± 0.128
>
> With identity 0.97, integrity 0.94, behaviour 0.44, telemetry 0.44, network 0.22 and provenance 0.83, the node trust is a geometric mean of **0.565**, compared with an arithmetic mean of 0.640. If B then behaves well ($s=0.05$), behaviour trust recovers to 0.690 after 10 ticks, 0.807 after 20 and 0.902 after 40.

**Sensor trust is separate from node trust:** $T^{\text{obs}}_j = P(\text{reading from sensor } j \text{ is reliable} \mid E)$. Its evidence is the sensor's innovation $\epsilon_j$ (§6.3) and how much it disagrees with the robust aggregate (§6.5). This is what §6.3 feeds back into the Kalman filter.

**Labels shown in the UI**, from $\mathbb E[T_i]$: NORMAL ≥ 0.75 · SUSPICIOUS 0.40–0.75 · UNTRUSTED < 0.40. They're for display only; decisions always use the numbers.

## 6.7 Operational graph and capabilities

**Owner:** KESIVARSHINI · `aeris-graph`

```math
G_t = (V_t, E_t), \quad V = \{\text{nodes, sensors, links, capabilities, mission}\}, \quad E \subseteq V\times V\times\{\textsf{depends\_on}, \textsf{routes\_through}, \textsf{observes}, \textsf{supports}\}
```

Each vertex carries its live estimate $(\hat x_i, T_i, C_i, R_i)$. Each link edge carries $(\text{availability}, \text{latency}, \text{active})$. A standby link such as L2 exists in the graph but has $\text{active}=0$ until a reroute turns it on.

**Reach and blast radius** (BFS over "who depends on me"):

```math
\mathrm{Reach}(v) = \{u : u \leadsto v\}, \qquad B(v) = \sum_{u\in \mathrm{Reach}(v)} C_u, \qquad \hat B(v) = \frac{B(v)}{\sum_{u\in V} C_u}
```

**Capability levels**, computed from the graph every tick. These definitions are frozen on D0:

```math
c_{\text{comm}} = \max_{p\ \in\ \text{active paths } A\to C}\ \prod_{e\in p}\text{avail}_e \prod_{v\in \text{interior}(p)} T_v
```

*Communication is only as good as the best active path, and a path through a distrusted relay is a bad path.*

```math
c_{\text{obs}} = 1 - \prod_{v \in \text{active}} \big(1 - \text{cov}_v\,T^{\text{obs}}_v\big)
```

*Observation succeeds if at least one trusted sensor sees the target. Each node covers $\text{cov}_v = 0.6$ of the area.*

```math
c_{\text{nav}} = \min_{v \in \text{active}} \Big(1 - \min\big(1, \sqrt{P_v}\,/\,\sigma_{\max}\big)\Big), \qquad \sigma_{\max} = 30\text{ m}
```

*Navigation is only as good as the worst-localised active node, using the Kalman covariance $P_v$ from §6.3.*

## 6.8 Mission and criticality

**Owner:** KESIVARSHINI · `aeris-mission`

The mission $M = (\text{objectives}, \text{capabilities}, \text{dependencies}, \text{constraints})$ succeeds when every capability meets its minimum:

```math
\text{MissionSuccess} = \bigwedge_k \big(c_k \ge c_k^{\min}\big), \qquad c^{\min}_{\text{nav}} = 0.85,\ \ c^{\min}_{\text{comm}} = 0.90,\ \ c^{\min}_{\text{obs}} = 0.80
```

For ranking and display we also use a continuous mission score, where 1 means fully met:

```math
M(G) = \prod_k \min\!\Big(1,\ \frac{c_k}{c_k^{\min}}\Big)
```

**Counterfactual criticality.** Don't hardcode "B is important". Remove B and measure the damage:

```math
C_i = \frac{M(G) - M(G\setminus v_i)}{M(G)} \qquad (C_i = 0 \text{ if } M(G) = 0)
```

> **Worked example: the mission around B.**
>
> | Situation | $c_{\text{nav}}$ | $c_{\text{comm}}$ | $c_{\text{obs}}$ | $M(G)$ |
> |---|---|---|---|---|
> | All healthy, $T_B = 0.92$, L1 active | 0.930 | 0.99 × 0.99 × 0.92 = 0.902 | 0.920 | **1.000** ✓ |
> | B suspicious, $T_B = 0.565$ | 0.930 | 0.554 ✗ | 0.920 | 0.615 |
> | B removed, L2 still off | 0.930 | 0 ✗ | 0.815 | 0.000 |
> | B isolated *and* L2 activated | 0.930 | 0.930 | 0.815 | **1.000** ✓ |
>
> So $C_B = (1 - 0)/1 = 1.0$: B is fully critical *while it's the only active relay*. The last row is why "isolate + reroute" can save the mission. Nobody scripted that; it falls out of the formulas.

## 6.9 Risk and tail risk (CVaR)

**Owner:** KESIVARSHINI · `aeris-mission`

**Trust ≠ risk.** A slightly suspicious critical node can be more dangerous than a very suspicious irrelevant one.

```math
R_i = P(\text{fail}_i \mid E)\cdot \text{Impact}_i, \qquad P(\text{fail}_i\mid E) = 1 - P(\text{normal}_i \mid E), \qquad \text{Impact}_i = 1 - (1 - C_i)(1 - \hat B_i)
```

*Impact combines "how much mission we lose" (criticality) with "how much else breaks" (blast radius) in a noisy-OR, so either one can make impact high. For B: $P(\text{fail}) = 1 - 0.013 = 0.987$ and $\text{Impact} = 1.0$, so $R_B = 0.987$.*

**Tail risk.** Expected loss alone hides rare disasters. Value at risk and conditional value at risk at level $\alpha$:

```math
\mathrm{VaR}_\alpha(L) = \inf\{\ell : P(L \le \ell) \ge \alpha\}, \qquad \mathrm{CVaR}_\alpha(L) = \min_{\nu}\Big\{\nu + \tfrac{1}{1-\alpha}\,\mathbb E\big[(L - \nu)^+\big]\Big\}
```

*CVaR at 0.9 is "the average loss across the worst 10% of outcomes". From $N$ rollout samples, sort the losses and average the worst $\lceil (1-\alpha)N \rceil$ (50 of 500). That empirical estimate is what `aeris-mission` implements.*

## 6.10 Actions and counterfactual rollouts

**Owners:** DHARSHAN · `aeris-sim` · KISHANTH · `aeris-decision`

AERIS never invents commands. It picks from a whitelist:

```math
\mathcal A_{\text{approved}} = \{\textsf{monitor},\ \textsf{restrict}(i),\ \textsf{isolate}(i),\ \textsf{reroute}(\ell),\ \textsf{failover}(i\to j),\ \textsf{degrade}(k),\ \textsf{recover}(i)\}
```

Candidates are single actions plus pairs (like isolate + reroute), filtered to those that make sense now: $\mathcal A_t = \{a : \text{Feasible}(S_t, a)\}$.

**A rollout** is one simulated future. For each candidate $a$, run $n = 1\dots N$:

```text
sample a hidden state  s⁽ⁿ⁾ ~ b_t        (e.g. "B compromised" with prob 0.947)
copy the world, apply action a
step K ticks:  S ← F(S, a, W⁽ⁿ⁾)       W from rng(seed, a, n), so results are reproducible
success⁽ⁿ⁾ = 1 if every capability ≥ its minimum on every tick, else 0
loss    L⁽ⁿ⁾ = (1 − success⁽ⁿ⁾) + d(a)
```

```math
\hat P(\text{success}\mid a) = \frac1N \sum_{n=1}^{N} \text{success}^{(n)}, \qquad \mathrm{SE} = \sqrt{\frac{\hat p(1-\hat p)}{N}}, \qquad \hat{\mathbb E}[U\mid a] = 1 - \frac1N\sum_n L^{(n)}
```

*v0.1 uses $N = 500$ rollouts and a horizon of $K = 100$ ticks (10 s). With $N = 500$, a 94% success estimate is accurate to about ±1.1%. The disruption cost $d(a)$ is a frozen table: monitor 0, restrict 0.05, reroute 0.05, isolate 0.10, isolate + reroute 0.15, failover 0.10, degrade 0.30.*

## 6.11 Decision optimization

**Owner:** KISHANTH · `aeris-decision`

The score of an action is its expected utility minus a penalty for its bad tail:

```math
J(a) = \hat{\mathbb E}[U \mid a] - \lambda\,\widehat{\mathrm{CVaR}}_\alpha(L \mid a), \qquad \lambda = 0.5,\ \alpha = 0.9
```

*The source spec wrote this as $J = w_M M - w_R R - w_C C - w_D D$. In v0.1, mission performance and residual risk come in through success and loss, and operational cost and disruption through $d(a)$. It's the same idea with fewer knobs to tune.*

**Three kinds of constraint**, applied in this order:

1. **Hard invariants** $g_h(S,a) \le 0$, checked by `aeris-policy` before scoring. An action that fails one is never scored. See §6.12 for I1–I4.
2. **Chance constraint** on the failure rate, using an upper confidence bound so we don't trust a lucky estimate:

   ```math
   \big(1 - \hat P(\text{success}\mid a)\big) + 1.645\cdot \mathrm{SE} \ \le\ \varepsilon, \qquad \varepsilon = 0.10
   ```

3. **Soft objective:** maximise $J(a)$ over whatever survives.

```math
a^* = \arg\max_{a\,\in\,\mathcal A_t}\ J(a) \quad \text{s.t. } g_h(S,a)\le 0,\ \ \text{UCB}_{\text{fail}}(a) \le \varepsilon
```

*If **no** action passes the chance constraint, AERIS doesn't give up. It picks the highest $J$ among the actions that pass the invariants and forces human review, with the reason "no option meets the failure bound".*

With at most about 20 candidates, exhaustive evaluation is exact and fast. MILP and CP-SAT are only needed for large action spaces (v0.2).

> **Worked example: choosing B's response** (illustrative rollout success rates; the real ones come from `aeris-sim`).
>
> | Action | $\hat P(\text{success})$ | $d(a)$ | $\mathbb E[U]$ | $\mathrm{CVaR}_{0.9}(L)$ | $J(a)$ | Constraints |
> |---|---|---|---|---|---|---|
> | Monitor | 0.31 | 0.00 | 0.310 | 1.000 | −0.190 | fails chance |
> | Reroute only | 0.55 | 0.05 | 0.500 | 1.050 | −0.025 | fails chance |
> | Isolate B only | — | — | — | — | never scored: removes the last active comm path | ✗ violates I1 |
> | **Isolate B + reroute via L2** | 0.94 | 0.15 | 0.790 | 0.750 | **0.415** | ✓ (UCB 0.078 ≤ 0.10) |
> | Degrade mission | 0.68 | 0.30 | 0.380 | 1.300 | −0.270 | fails chance |
>
> How the CVaR for isolate + reroute is computed: 6% of rollouts fail (loss 1.15) and 94% succeed (loss 0.15). The worst 10% is all the failures plus 4% of the successes, so $\mathrm{CVaR} = (0.06\times1.15 + 0.04\times0.15)/0.10 = 0.75$. Every other option has failures filling its whole worst 10%, so its CVaR is $1 + d(a)$.

## 6.12 Autonomy gate, policy and safety

**Owner:** KESIVARSHINI · `aeris-policy`

**Impact classes**, taken from the source spec:

| Class | Examples | Who decides |
|---|---|---|
| LOW | monitor, restrict, recompute sensor confidence, switch to a redundant sensor | Can run automatically |
| MEDIUM | reroute, isolate a non-critical node ($C_i < 0.5$), degrade a capability | Operator approval |
| HIGH | isolate a critical node ($C_i \ge 0.5$), disable critical comms, change mission parameters | Commander approval |

**The autonomy gate**:

```math
\mathcal H(a) = \begin{cases} \textsf{REJECT} & \Pi(S,a) = 0 \\ \textsf{HUMAN\_REVIEW} & \text{class}(a)\ne\textsf{LOW} \ \vee\ u_t \ge \theta_U \ \vee\ \neg\text{reversible}(a) \ \vee\ \text{health}_t < 0.9 \\ \textsf{AUTO} & \text{otherwise} \end{cases}
```

*$u_t$ is the decision's uncertainty: the larger of the target's trust standard deviation and the normalised entropy of its hypothesis posterior. v0.1 sets $\theta_U = 0.25$. In short: more uncertainty means less autonomy, and more impact means more human authority.*

**Policy** is a deterministic yes/no. The optimizer proposes; policy disposes:

```math
\Pi(S,a) = \text{Auth}(u, a) \wedge \text{Safety}(S,a) \wedge \text{Mission}(S,a) \wedge \text{Context}(S,a) \in \{0,1\}, \qquad \text{Execute}(a) \iff \Pi(S,a) = 1
```

**Hard invariants** (the $g_h$ of §6.11). No optimizer, AI or human can override these:

- **I1** Never remove the last active communication path between required mission endpoints.
- **I2** Never isolate the only provider of a required capability, unless the same action restores that capability (as reroute does) or a commander explicitly approves degrading the mission.
- **I3** The action must be in $\mathcal A_{\text{approved}}$ and target an entity that exists and isn't already in the target state. Commands are idempotent.
- **I4** While model health is DEGRADED, only monitor and restrict may run without a human.

**Re-check before executing.** The safety monitor re-evaluates $\Pi$ against the *current* state at execution time, not the state when the recommendation was made, because the world may have changed while the human was reading.

**Human unavailable.** If a HIGH or MEDIUM action gets no decision within $\tau_h = 60$ s, AERIS does **not** execute it. It applies the safest LOW action that passes the invariants (usually restrict) and keeps escalating.

## 6.13 Execution, verification and recovery

**Owner:** DHARSHAN · `aeris-exec`

```math
u_t = \Gamma(a_t) \ \ \text{(action to concrete commands, each with a unique command id)}, \qquad S_{t+1} = F(S_t, u_t, W_t)
```

**Verification** never assumes a command worked. It compares the expected state (the median rollout of the chosen action) with what is observed, within $T_v = 3$ s:

```math
e_{\text{verify}} = S^{\text{obs}}_{t+T_v} - \hat S_{t+T_v}, \qquad \text{success} \iff \text{isolated}(B) \wedge \text{active}(L2) \wedge \textstyle\bigwedge_k \big(c_k \ge c_k^{\min}\big)
```

*On failure, re-run the decision from the new state. After 3 failed replans, escalate to a human with everything attached.*

**AERIS's own operating mode** (shown in the UI header):

```math
\textsf{NORMAL} \xrightarrow{R_{\max}> 0.3} \textsf{DEGRADED} \xrightarrow{R_{\max}> 0.6} \textsf{CONTAINMENT} \xrightarrow{\text{class}\ne\textsf{LOW}} \textsf{HUMAN\_REVIEW} \xrightarrow{\text{verified}} \textsf{RECOVERY} \xrightarrow{R_{\max}< 0.3\ \text{for 10 s}} \textsf{NORMAL}
```

*From any mode, model health below 0.9 moves AERIS to FAILSAFE, where only LOW actions run without a human.*

**Recovery of an isolated node** (designed now, built after v0.1):

```math
\textsf{ISOLATED} \to \textsf{VALIDATING} \xrightarrow{\text{identity}=1,\ \text{integrity}=1,\ T>0.6} \textsf{RESTRICTED} \to \textsf{SHADOW} \xrightarrow{\text{agreement}\ge0.95\ \text{for 30 s}} \textsf{RECOVERED}
```

*In SHADOW the node runs, but its outputs are only compared against the others, never used.*

## 6.14 AI explanation with grounding

**Owner:** AMIRTHA · `aeris-explain`

The LLM receives only computed facts. It returns structured claims, and every claim must cite evidence:

```math
Z^{\text{AI}}_t = \big[E_t,\ \Delta T_t,\ \text{path}_G,\ \{(a, \hat P, J)\},\ a^*,\ \Pi\big] \ \longrightarrow\ \text{LLM} \ \longrightarrow\ \{(\text{claim}_m,\ \text{evidence\_ids}_m,\ \text{numbers}_m)\}
```

```math
\text{accept} \iff \forall m:\ \text{evidence\_ids}_m \subseteq \text{ids}(E_t)\ \wedge\ \text{numbers}_m \subseteq_{\pm0.005} \text{numbers}(Z^{\text{AI}}_t)\ \wedge\ \text{action} = a^*
```

*If any claim cites an unknown evidence ID, a number that isn't in the input, or a different action, the whole answer is discarded and the template explainer is used instead. AI output is never authority.*

> **Template output (always available).**
> "Trust in Node B fell from 0.90 to 0.57 [ev-0412, ev-0415, ev-0419, ev-0423]. Network, authentication and process anomalies point to compromise (P = 0.95). B is the only active relay for Communication (criticality 1.00). Isolating B and activating L2 gives the best predicted outcome (success 0.94, score 0.415). Isolating B alone was rejected (rule I1)."

## 6.15 Decision record (audit and provenance)

```math
D_t = \big(b_t,\ E_t,\ G_t,\ M_t,\ R_t,\ \mathcal A_t,\ \text{Sim}_t,\ \text{Opt}_t,\ \text{AI}_t,\ \mathcal H_t,\ \Pi_t,\ \text{Exec}_t,\ \text{Verify}_t,\ \text{model\_ver},\ \text{policy\_ver},\ \text{seed}\big)
```

*With the seed and versions recorded, any decision can be replayed exactly. Each record is hash-chained and signed (§6.2), so we can reconstruct evidence → state → trust → risk → simulation → decision → human → policy → execution → outcome.*

## 6.16 Evaluation metrics

**Owners:** AMIRTHA · DHARSHAN · `aeris-eval`

```math
\text{Precision} = \frac{TP}{TP+FP}, \qquad \text{Recall} = \frac{TP}{TP+FN}, \qquad F_1 = \frac{2\cdot\text{Precision}\cdot\text{Recall}}{\text{Precision}+\text{Recall}}
```

*A TP is an incident opened on the right node within 5 s of the injected fault. An incident with no fault behind it is an FP. A fault with no incident within 5 s is an FN.*

```math
\text{MSR} = \frac{1}{N_{\text{runs}}}\sum_{r} \mathbb 1\big[\text{mission succeeded in run } r\big], \qquad \text{delay} = t_{\text{detect}} - t_{\text{inject}}, \qquad \text{recovery} = t_{\forall k:\,c_k \ge c_k^{\min}} - t_{\text{inject}}
```

We also report decision latency (incident opened → recommendation, p50/p95), expected loss and CVaR across runs. Every metric is reported for **AERIS**, the **alert-only** baseline (monitor forever) and the **naive** baseline (isolate the first alerted node, never reroute).

> **Targets:** S4 MSR = 100% over 100 seeds · S1 false positives ≤ 1 per 10 simulated minutes · detection delay p95 ≤ 2 s · decision latency p95 ≤ 500 ms with AI off.
