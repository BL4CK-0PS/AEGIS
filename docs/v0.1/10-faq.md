# 10 · FAQ for newcomers

[← Index](README.md)

**Why not just ask an LLM "is node B hacked?"**
Because it can't be tested, it can hallucinate, and it can be manipulated through the very data it reads. A decision about isolating a live system needs maths we can check, replay and bound. The LLM's job is to explain those results in words.

**Why Rust for everything?**
Memory safety, predictable performance in the inner loop (500 rollouts × 5 actions every incident), one language from simulator to UI, and one types crate shared by backend and frontend. AERIS is also meant to end up inside Canopus OS and on the Spica flight computer, which are Rust targets.

**Why a simulator instead of real drones?**
We need ground truth: we know exactly when and where the fault was injected, so we can measure precision, recall and mission success honestly. Real hardware comes later (Q3 2027 roadmap). The `Adapter` trait is the seam where it plugs in.

**What does "trust 0.57" actually mean?**
Given all the evidence so far, there's about a 57% chance the node is healthy and honest, with a stated uncertainty (±0.13 in the example). It's the mean of a Beta distribution ([§6.6](04-mathematical-model.md#66-trust)), not a vibe.

**What if AERIS is wrong and isolates a healthy node?**
Several layers catch it: correlated evidence is discounted ([§6.5](04-mathematical-model.md#65-evidence-fusion)), high-impact actions need a human ([§6.12](04-mathematical-model.md#612-autonomy-gate-policy-and-safety)), every action is reversible, verification checks the result ([§6.13](04-mathematical-model.md#613-execution-verification-and-recovery)), and the recovery state machine brings a node back safely. We measure the false-positive rate on S1 for exactly this reason.

**What if the AI model is down?**
Nothing changes except the explanation wording: the template explainer takes over. CI runs the S4 test with AI off.

**What if no human answers?**
High- and medium-impact actions never run without one. After 60 s AERIS applies the safest low-impact action and keeps escalating ([§6.12](04-mathematical-model.md#612-autonomy-gate-policy-and-safety)).

**What is CVaR in one line?**
The average of the worst 10% of outcomes. It makes AERIS prefer "reliably good" over "great on average, occasionally disastrous".

**Is the 94% in the demo real?**
Only if `aeris-eval` produced it. Every figure in these docs is a worked example of the formulas, and the demo shows only measured numbers.

**Where do I start?**
Find your name in [02](02-team-stack-layout.md), read the matching blocks in [04](04-mathematical-model.md), read [06](06-contracts.md), then do your D0 row in [07](07-schedule.md#day-by-day-who-does-what-every-day).
