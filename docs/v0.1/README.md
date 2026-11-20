# AERIS v0.1.0 — Build Specification

> Ten-day, four-person, all-Rust build plan for Quantrix AERIS v0.1.0.
> Fri 20 Nov → Sun 29 Nov 2026 · buffer Mon 30 Nov.

| Working days | Engineers | Crates + apps | Hard gates | Person-days used |
|---|---|---|---|---|
| 10 (D0–D9) | 4 | 15 + 4 | G0–G5 | ~34 / 40 |

> **Assumption.** The team works the weekend (21–22 and 28–29 Nov). If it doesn't, apply the cuts in [08 — Cuts](08-rules-cuts-risks.md#scope-cuts-and-fallbacks) from the bottom up. Mon 30 Nov is for fixes only: no new features.

Source: `startup/chatgpt/aegis/aegis.md`, read in full (final spec §1–52, module architecture, MVP, golden scenario, Definition of Done). In these docs the product is named **AERIS**.

> **Note.** `docs/00–25` and the root `README.md` describe an earlier AERIS concept (multimodal evidence reasoning). The v0.1 build follows the documents in this folder.

## Contents

| # | Document | Covers |
|---|---|---|
| 00 | This page | What AERIS is, how it works, design rules, glossary |
| 01 | [Definition of done](01-definition-of-done.md) | The end-to-end loop, simulated world, scenario suite S1–S8 |
| 02 | [Team, stack, layout](02-team-stack-layout.md) | Ownership, pinned versions, repository layout |
| 03 | [Maths → code](03-maths-to-code.md) | What each crate implements in v0.1 |
| 04 | [Mathematical model](04-mathematical-model.md) | Every equation, with parameters and worked numbers |
| 05 | [Worked example: S4](05-worked-example-s4.md) | One attack through every stage |
| 06 | [Contracts](06-contracts.md) | Wire format, core types, HTTP API, database, config files |
| 07 | [Schedule](07-schedule.md) | Gates G0–G5, day-by-day plan, today's checklist |
| 08 | [Rules, cuts, risks](08-rules-cuts-risks.md) | Working rules, scope cuts, top risks |
| 09 | [Demo and run](09-demo-and-run.md) | Four-minute demo script, local run commands |
| 10 | [FAQ](10-faq.md) | Questions newcomers ask on day one |

**How to read these docs.** Read this page, [01](01-definition-of-done.md) (what we're building) and [02](02-team-stack-layout.md) (your role) first. Then read [04](04-mathematical-model.md) for the maths behind your own crates, and [05](05-worked-example-s4.md), which follows one attack through every equation with real numbers. [06](06-contracts.md) lists the interfaces you must not break. [07](07-schedule.md) and [08](08-rules-cuts-risks.md) cover the schedule, rules, cuts and risks. [09](09-demo-and-run.md) is the demo script and how to run everything, and [10](10-faq.md) answers common first-day questions. Every symbol is explained where it first appears, and each block of maths has a plain-English line below it.

---

## 00 · Start here

### What AERIS is, in one sentence

> AERIS is a decision system for autonomous and mission-critical machines (drones, satellites, robot fleets). When a part starts behaving suspiciously, it works out *what can still be trusted*, *what that means for the mission* and *which response keeps the mission alive*. It then asks a human before doing anything high-impact.

### The problem it solves

Picture three drones, A, B and C, flying a surveillance mission. Drone B suddenly shows unusual network traffic, failed logins, a busy CPU, and a camera that disagrees with its GPS. Today's monitoring tools would say "multiple anomalies detected" and stop. The operator still doesn't know:

- Is B hacked, broken, or just in bad weather?
- Can I still trust B's GPS? Its telemetry?
- What depends on B? What happens to the mission if I cut B off?
- Should I isolate it, reroute around it, or do nothing? How sure are we?

AERIS answers those questions with maths, not guesses, and shows the evidence behind every answer.

### How it works: one loop, ten times a second

```text
  REAL SYSTEM (in v0.1: our simulator, aeris-sim)
        │  telemetry, network, login, sensor readings — each one signed
        ▼
  1 EVENTS        check signature · order · drop duplicates
  2 STATE         "where is each drone really, and is it NORMAL / SUSPICIOUS / COMPROMISED?"
  3 ANOMALY       "does this reading look abnormal?"  → score 0..1
  4 FUSION        "given all the anomalies together, what is most likely going on?"
  5 TRUST         "how much do we still trust each drone, sensor and link?"  → 0..1 ± uncertainty
  6 GRAPH         "what depends on what?"
  7 MISSION/RISK  "which mission capabilities are at risk, and how badly?"
  8 SIMULATE      "if we take action X, what probably happens next?" (500 simulated futures each)
  9 DECIDE        "which allowed action gives the best mission outcome with the least tail risk?"
 10 EXPLAIN       AI writes a plain-English explanation from the computed evidence
 11 HUMAN         operator presses APPROVE / MODIFY / REJECT for high-impact actions
 12 SAFETY        deterministic rules can still veto it
 13 EXECUTE       send the command (isolate, reroute, …)
 14 VERIFY        did the world end up where we predicted? If not, plan again
        └──────► back to 1
```

### Three rules that shape the whole design

| Rule | Name | Meaning |
|---|---|---|
| 1 | **Maths decides, AI explains** | Trust, risk and the chosen action are computed by deterministic, testable Rust code. The LLM only turns those results into sentences. If the AI is switched off, AERIS still decides correctly. |
| 2 | **Three separate authorities** | **Intelligence** asks "what is probably happening?" (stages 2–5). **Decision** asks "what is the best response?" (6–9). **Authority** asks "is it allowed?" (11–12). Different crates, different authors, so one bug can't do everything. |
| 3 | **More doubt, less autonomy** | The more uncertain AERIS is, or the bigger the impact of an action, the more it hands control to a human. It never becomes *more* autonomous when it's *less* sure. |

### What AERIS is not

- **Not a SIEM or alert dashboard.** A SIEM collects security logs and raises alerts. AERIS goes further: it turns those alerts into trust, mission impact and a decision.
- **Not an LLM agent.** The language model never controls anything.
- **Not a weapon system.** v0.1 is a generic simulator of three autonomous nodes.

### Glossary

| Term | Meaning |
|---|---|
| **Observation** | A raw reading from the system: a GPS position, a CPU percentage, a failed login. It can be noisy, or a lie. |
| **Belief state** | AERIS's probability-weighted guess about the true hidden state, for example "B is compromised with probability 0.95". |
| **Anomaly score** | A number from 0 to 1 saying how unusual one signal looks compared with normal behaviour. |
| **Evidence** | An anomaly score plus where it came from (its provenance) and whether its signature checked out. Every piece has an ID, and the AI must cite these IDs. |
| **Hypothesis** | A candidate explanation: normal, compromise, sensor failure, misconfiguration or environment (weather, terrain). |
| **Trust** | The probability that an entity (node, sensor, link) is healthy and honest, kept with its uncertainty. Trust is not the same as risk. |
| **Operational graph** | A network of entities and "depends on / talks to / routes through" relationships. |
| **Capability** | Something the mission needs, such as navigation, communication or observation, each with a minimum level. |
| **Criticality** | How much mission capability we'd lose if a component vanished, computed by actually removing it. |
| **Blast radius** | Everything downstream of a component that would be affected if it failed. |
| **Risk** | Probability of failure × impact on the mission. |
| **Counterfactual / rollout** | A simulated "what if": copy the current state, apply an action, run the simulator forward and see what happens. We do this hundreds of times per action. |
| **CVaR (conditional value at risk)** | The average loss in the worst cases (for example the worst 10%). It stops us picking an action that's great on average but occasionally catastrophic. |
| **Hard invariant** | A rule that can never be broken, such as "never cut the last communication path". It filters actions out; it isn't a penalty that can be traded away. |
| **Chance constraint** | A rule on probability, such as "the chance this action causes mission failure must be ≤ 1%". |
| **Autonomy gate** | The function that decides whether an action runs automatically, needs a human, or is rejected. |
| **Provenance** | Where a fact came from and what processed it. It lets us replay and audit every decision. |
| **Crate** | A Rust library or package. Our code is split into about 15 crates. |
