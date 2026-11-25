# Decision record: `aeris-graph` contract

Status: **approved on 2026-11-25**. Kishanth approved all Section 9 proposals and adding `petgraph = "0.8.3"` to the workspace. Section 9 is the canonical implementation decision; where it conflicts with Sections 3–7, Section 9 wins. Workspace changes are limited to adding `petgraph` and registering `aeris-graph`.

## 1. Conflicting definitions found

| Topic | Exact source | Text / conflict |
|---|---|---|
| Edge kinds | `docs/v0.1/03-maths-to-code.md` line 17, `aeris-graph` table row | `depends_on, communicates_with, routes_through and supports` |
| Edge kinds | `docs/v0.1/04-mathematical-model.md` §6.7 line 338 | `depends_on, routes_through, observes, supports` |
| Edge attributes | `docs/v0.1/04-mathematical-model.md` §6.7 line 341 | Each link edge carries `(availability, latency, active)`; standby L2 has `active=0` |
| Reachability prose | `docs/v0.1/04-mathematical-model.md` §6.7 line 343 | `BFS over "who depends on me"` |
| Reachability formula | `docs/v0.1/04-mathematical-model.md` §6.7 line 346 | `Reach(v) = {u : u leadsto v}` |
| Ownership | `docs/v0.1/02-team-stack-layout.md` lines 9, 12, 73 | `aeris-types`, `aeris-core`, `aeris-decision` are Kishanth-owned; `aeris-graph` is Kesivarshini-owned |
| Frozen contract rule | `docs/v0.1/06-contracts.md` lines 1–5 | Contracts are frozen at G0; changes require a PR reviewed by Kishanth and announcement at standup |
| Toolchain / edition | `docs/v0.1/02-team-stack-layout.md` line 25 | Rust `1.100.0`, edition `2024` |
| Toolchain / edition | `Cargo.toml` line 13; `rust-toolchain.toml` lines 1–2 | workspace edition `2021`; channel `stable-x86_64-pc-windows-gnu` |
| Graph dependency | `docs/v0.1/02-team-stack-layout.md` line 32 | `petgraph` `0.8.3` is the chosen graph library |
| Core graph logic | `crates/aeris-core/src/lib.rs` lines 25–32, 69–126, 132–225 | `LinkState` and capability/criticality math are hardcoded and live in Kishanth's crate |
| Rust/DB field names | `crates/aeris-types/src/lib.rs` lines 55–66, 401–407, 444–453, 487–497; `migrations/0001_init.sql` lines 54–61, 80–117, 125–180; `docs/v0.1/06-contracts.md` lines 105–120 | Shared Rust shapes and DB columns differ |

## 2. Conflicts, options, and recommendations

### 2.1 Edge-kind set

- Option A: treat §6.7's four kinds as canonical and map `communicates_with` onto `routes_through`. Minimal contract delta, but loses the distinction between link adjacency and mission path contribution.
- Option B: treat §03's four kinds as canonical and map `observes` onto `supports`. Matches the summary table, but loses the explicit sensor-coverage relation needed by `c_obs` in §6.7 line 358.
- **Recommended (Option C):** use the union of both lists — five kinds: `DependsOn`, `CommunicatesWith`, `RoutesThrough`, `Observes`, `Supports`. Give each a non-overlapping meaning (Section 3). Reason: both documents contain semantics that the other lacks; the union preserves link adjacency (`communicates_with` from §03) and sensor coverage (`observes` from §6.7) without overloading either. This changes/clarifies frozen text and needs Kishanth's approval.

### 2.2 Reachability direction

- Option A: store edges as dependency -> dependent, then forward BFS from `v` returns dependents. This reads naturally for “who depends on me”, but makes the written formula `u leadsto v` describe the opposite traversal.
- **Recommended (Option B):** store `DependsOn` as dependent -> dependency. Then `u leadsto v` literally means `u` depends transitively on `v`; dependents of `v` are found by reverse BFS from `v`. This reconciles §6.7 lines 343 and 346 without changing the formula. Explicit direction must be exposed in the API so dependencies are still available.
- Option C: treat `depends_on` as undirected. Rejected: it destroys the dependency/dependent distinction needed for blast radius and policy.

### 2.3 `reach()` return value

- Option A: `reach(v)` returns dependents only, following the prose. Simple, but cannot answer “what does `v` depend on?”.
- Option B: two separate functions, `dependencies(v)` and `dependents(v)`. Explicit, but duplicates traversal code and leaves the meaning of `reach` undefined.
- **Recommended (Option C):** `reach(v, kind, direction)` with `ReachDirection::{Dependencies, Dependents}`, plus thin convenience wrappers. This makes ambiguity impossible at call sites and supports both §6.7 formula and §6.7 prose. The default blast-radius call must explicitly use `Dependents`.

### 2.4 Hardcoded `aeris-core` graph logic

- `CoreState::new` hardcodes `node-a/b/c`, `link-l1` (availability `0.99`, via `node-b`, active) and `link-l2` (`0.93`, standby) at `crates/aeris-core/src/lib.rs:70–126`.
- `compute_capabilities` hardcodes those link IDs and literal scores at `crates/aeris-core/src/lib.rs:132–187`.
- `compute_mission_score` hardcodes thresholds at `crates/aeris-core/src/lib.rs:191–207`, while `docs/spec/params.toml:42–50` is the declared source of truth.
- `compute_criticality` clones state and isolates a node at `crates/aeris-core/src/lib.rs:211–225`.

Option A: leave core untouched and let `aeris-mission`/`aeris-policy` call the new graph crate later. Risks two divergent capability implementations.
Option B: move the current hardcoded behavior into `aeris-graph` unchanged. Rejected: it would embed magic numbers that contradict `params.toml`.
**Recommended:** define and test the graph API first; in a separate, explicitly approved PR, Kishanth replaces the hardcoded core functions with calls into `aeris-graph`/`aeris-mission`. Until then, the graph crate is the new canonical structural model but is not wired into the pipeline.

### 2.5 Database schema vs Rust types

Examples of actual mismatches:

| Rust | Database / frozen DB table list |
|---|---|
| `Evidence { event, group, timestamp_ms }` (`aeris-types:55–66`) | `evidence(event_id, group_id)`, no timestamp (`migrations:108–117`; `06-contracts:116`) |
| `Gate::HumanReview { reason }` (`aeris-types:401–407`) | `decisions.gate TEXT` in `'auto' \| 'human_review' \| 'reject'` (`migrations:156,177`) |
| `LinkState { source, target, via }` (`aeris-core:25–32`) | `links(a, b)` with no `via` or latency (`migrations:54–61`; `06-contracts:113`) |
| `VerifiedEvent.seq: u64`, `Hlc.c: u32` (`aeris-types:470–497`) | `events.seq BIGINT`, `hlc_c INTEGER` (`migrations:83–85`) |
| `Incident.opened_at_tick: u64` (`aeris-types:444–453`) | `incidents.opened_at TIMESTAMPTZ`, plus nullable `closed_at` (`migrations:134–135`) |
| No `DecisionStatus` in the frozen shared types list (`06-contracts:43–69`) | `decisions.status` enum `COMPUTED … VERIFIED` (`migrations:162–172`) |

Option A: keep an explicit conversion/adapter in `aeris-server` (which is Kesivarshini-owned) and keep `aeris-graph` free of DB row types. No shared-type change required.
Option B: rename DB columns to match Rust and add missing columns. Requires a migration change and an update to the frozen DB table list in `06-contracts.md` → Kishanth approval.
Option C: rename shared Rust fields to match DB columns. Requires Kishanth approval and would break existing core/console code.
**Recommended:** Option A for v0.1. `aeris-graph` should depend only on `EntityId`, `LinkId`, and `Capability` from `aeris-types`, and accept dynamic trust/coverage/sigma values through closures. DB-to-graph conversion belongs in `aeris-server`.

### 2.6 Workspace edition and toolchain

- Spec: `docs/v0.1/02-team-stack-layout.md:25` says edition `2024`, Rust `1.100.0`.
- Repo: `Cargo.toml:13` says edition `2021`; `rust-toolchain.toml:1–2` is an unpinned channel.

Option A: leave the workspace as-is; the new crate inherits edition `2021`. Safest for this decision document.
Option B: update workspace to edition `2024` and pin `1.100.0`. Requires local toolchain installation and approval from Dharshan (workspace owner, `02-team-stack-layout.md:10`) and Kishanth (tech lead).
**Recommended:** Option A now; raise Option B separately. No `Cargo.toml` or `rust-toolchain.toml` change is made here.

## 3. Proposed graph edge kinds and meanings

| Edge kind | Direction | Meaning |
|---|---|---|
| `DependsOn` | dependent -> dependency | Structural/mission dependency. Used for dependency/dependent traversal and blast radius. |
| `CommunicatesWith` | symmetric adjacency; store both directions | Direct link adjacency between two assets, carrying availability/latency/active from §6.7 line 341. Used by policy last-path checks; **not** used in `c_comm` products. |
| `RoutesThrough` | path direction, source -> next hop / link -> relay | Directed path leg used in the `c_comm` max-product calculation in §6.7 line 352. Only active edges contribute. |
| `Observes` | sensor -> observed entity | Sensor coverage relation supplying `cov_v` and `T_v^obs` to `c_obs` in §6.7 line 358. |
| `Supports` | evidence/claim -> supported claim/capability | Provenance/support relation retained from both source lists. Not used by v0.1 capability scoring. |

Non-overlap rule: `CommunicatesWith` describes adjacency; `RoutesThrough` describes a scored path. The same active link may produce both edge kinds, but path scoring must read only `RoutesThrough`.

## 4. Explicit direction semantics

For `A depends on B`:

1. Store one `DependsOn` edge `A -> B`.
2. `reach(A, DependsOn, Dependencies) = {B}` (forward BFS).
3. `reach(B, DependsOn, Dependents) = {A}` (reverse BFS).
4. `blast_radius(B, DependsOn, Dependents)` includes `C_A`; if `C depends on A`, it also includes `C_C` transitively.
5. `reach(B, DependsOn, Dependencies)` returns whatever `B` itself depends on, and must not include `A`.

For `CommunicatesWith`, treat the edge as undirected; insert both `A -> B` and `B -> A` or normalize in traversal. Direction arguments have the same result for that kind.

## 5. `reach()` decision

`reach()` must support **both** through an explicit `ReachDirection` argument. The blast-radius path uses `Dependents` because §6.7 line 343 says “who depends on me”; the formula in §6.7 line 346 is then satisfied by the edge orientation in Section 4. Convenience wrappers `dependencies()` and `dependents()` should delegate to `reach()`.

## 6. Existing Rust types to reuse

Reuse, without modification:

- `aeris_types::EntityId` (`aeris-types:13`)
- `aeris_types::LinkId` (`aeris-types:17`)
- `aeris_types::Capability` (`aeris-types:22–28`)
- The `Vec<(Capability, f64)>` shape of `CycleOutput.capabilities` (`aeris-types:503–514`) as the output contract consumed later by mission/core.

Do not reuse or depend on:

- `aeris_core::LinkState` or `CoreState` — Kishanth-owned and hardcoded; convert from them in `aeris-server` later.
- `aeris_types::Evidence`, `Gate`, `Incident`, or `VerifiedEvent` inside `aeris-graph` — DB mapping belongs in `aeris-server`.
- Any new shared type in `aeris-types` until Kishanth approves it. `GraphError`, `EdgeKind`, `NodeAttrs`, `EdgeAttrs`, and `ReachDirection` should be local to `aeris-graph`.

## 7. Proposed public API for `aeris-graph`

```rust
use aeris_types::{Capability, EntityId, LinkId};
use petgraph::graph::{EdgeIndex, NodeIndex};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    DependsOn,
    CommunicatesWith,
    RoutesThrough,
    Observes,
    Supports,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReachDirection {
    Dependencies,
    Dependents,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeAttrs {
    pub id: EntityId,
    pub criticality: f64,
    pub capability: Option<Capability>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeAttrs {
    pub kind: EdgeKind,
    pub link_id: Option<LinkId>,
    pub availability: f64,
    pub latency_ms: f64,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    DuplicateEntity(EntityId),
    UnknownEntity(EntityId),
    MissingEndpoint { from: EntityId, to: EntityId },
}

pub type OperationalGraph = petgraph::graph::DiGraph<NodeAttrs, EdgeAttrs>;

#[derive(Debug, Clone)]
pub struct GraphModel {
    graph: OperationalGraph,
    nodes: HashMap<EntityId, NodeIndex>,
}

impl GraphModel {
    pub fn new() -> Self;

    pub fn add_node(&mut self, node: NodeAttrs) -> Result<NodeIndex, GraphError>;

    pub fn add_edge(
        &mut self,
        from: &EntityId,
        to: &EntityId,
        edge: EdgeAttrs,
    ) -> Result<EdgeIndex, GraphError>;

    pub fn node_index(&self, id: &EntityId) -> Option<NodeIndex>;

    pub fn reach(
        &self,
        from: &EntityId,
        kind: EdgeKind,
        direction: ReachDirection,
    ) -> Result<HashSet<EntityId>, GraphError>;

    pub fn dependencies(
        &self,
        from: &EntityId,
        kind: EdgeKind,
    ) -> Result<HashSet<EntityId>, GraphError>;

    pub fn dependents(
        &self,
        from: &EntityId,
        kind: EdgeKind,
    ) -> Result<HashSet<EntityId>, GraphError>;

    pub fn blast_radius(
        &self,
        from: &EntityId,
        kind: EdgeKind,
        direction: ReachDirection,
    ) -> Result<(f64, f64), GraphError>;

    pub fn communication_score(
        &self,
        source: &EntityId,
        target: &EntityId,
        trust: impl Fn(&EntityId) -> f64,
    ) -> Result<f64, GraphError>;

    pub fn observation_score(
        &self,
        coverage: impl Fn(&EntityId) -> f64,
        sensor_trust: impl Fn(&EntityId) -> f64,
    ) -> f64;

    pub fn navigation_score(
        &self,
        sigma: impl Fn(&EntityId) -> f64,
        sigma_max: f64,
    ) -> f64;
}
```

`blast_radius` returns `(raw_sum, normalized)`; normalized is `0.0` when total criticality is `0.0`. `communication_score` considers only active `RoutesThrough` paths and multiplies interior-node trust. `petgraph` is specified at `docs/v0.1/02-team-stack-layout.md:32`, but adding it to `Cargo.toml` is deferred pending workspace approval.

## 8. Changes requiring Kishanth's approval

| Change | Why approval is needed |
|---|---|
| Canonical edge-kind set (five kinds) and non-overlap rule | Clarifies/changes frozen `docs/v0.1/03-maths-to-code.md:17` and `docs/v0.1/04-mathematical-model.md:338`; `06-contracts.md:5` requires PR review by Kishanth. |
| `DependsOn` edge orientation and `reach()` direction semantics | The formula and prose in §6.7 currently read inconsistently; this decision defines the shared interpretation used by mission and policy. |
| Any edit to `aeris-types` | Kishanth owns `aeris-types` (`02-team-stack-layout.md:9`, `06-contracts.md:45`). This proposal adds no shared types. |
| Replacing hardcoded capability/criticality logic in `aeris-core` | Kishanth owns `aeris-core` (`02-team-stack-layout.md:9`, `06-contracts.md:67–68`). |
| Any change to the frozen DB table list or shared type field names | `06-contracts.md:5` and lines 105–120 freeze the database contract. |
| Any change to `aeris-decision` invariant checks | Kishanth owns `aeris-decision` (`02-team-stack-layout.md:9`). |
| Workspace edition/toolchain change and adding `petgraph` to the workspace | Dharshan owns workspace/CI (`02-team-stack-layout.md:10`); Kishanth is tech lead and the spec conflicts with the repo. |

Kishanth approved all changes listed above on 2026-11-25. The approval was confirmed in the project session; no separate approval file is present.

## 9. Technical-review corrections (approved 2026-11-25)

A technical review found errors and ambiguities in Sections 3–7. Kishanth approved every item in this section on 2026-11-25; these decisions supersede conflicting text in Sections 3–7.

### 9.1 `EdgeKind` needs tighter endpoint domains

Sources: `docs/v0.1/03-maths-to-code.md` line 17 lists `depends_on, communicates_with, routes_through, supports`; `docs/v0.1/04-mathematical-model.md` §6.7 line 338 lists `depends_on, routes_through, observes, supports`; `docs/04-architecture.md` §12 lines 355–380 defines `supports` as an evidence-relationship type.

Problem: the five kinds are not fully non-overlapping as written. `Supports` is close to the inverse of `DependsOn` (`A depends on B` vs `B supports A`) and is listed as an evidence relation in `04-architecture.md`. `CommunicatesWith` and `RoutesThrough` both describe link adjacency. Section 3 also mixes two endpoint domains for `RoutesThrough` (“source -> next hop” and “link -> relay”).

Proposal: define endpoint domains explicitly and add invariants:

- `DependsOn`: entity -> entity; dependent -> dependency. This is the only kind used by blast radius.
- `CommunicatesWith`: asset -> asset; symmetric adjacency only; `link_id` must be `Some`. Not used by `communication_score`.
- `RoutesThrough`: asset -> asset path leg; `link_id` must be `Some`; only active edges are scored. This is the only kind used by `communication_score`.
- `Observes`: sensor entity -> observed entity; used by `observation_score`.
- `Supports`: evidence/claim -> supported claim/capability; inert in v0.1 capability equations.

Decision: approved these domains; `Supports` remains a variant in `aeris-graph` and is inert in v0.1 capability equations.

### 9.2 `DependsOn` orientation is logically correct

Sources: `docs/v0.1/04-mathematical-model.md` §6.7 lines 343 and 346.

Finding: storing `A depends on B` as edge `A -> B` is consistent with `Reach(v) = {u : u leadsto v}` if `leadsto` follows stored edge direction. Forward BFS from `A` returns dependencies of `A`; reverse BFS from `B` (incoming neighbors) returns dependents of `B`. No correction needed.

Implementation requirement: reverse BFS must use a visited set seeded with the start node so cycles terminate.

### 9.3 `ReachDirection` does not work consistently as named

Sources: `docs/v0.1/03-maths-to-code.md` line 17; `docs/v0.1/04-mathematical-model.md` §6.7 lines 338, 343, 346.

Problem: `Dependencies` / `Dependents` are meaningful for `DependsOn`, but not for `RoutesThrough` or `CommunicatesWith`. For a route leg, the useful directions are upstream/downstream; for adjacency, both directions may be equivalent.

Proposal: rename the traversal enum and keep semantic wrappers:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Traversal {
    Forward,
    Reverse,
}

pub fn reach(
    &self,
    from: &EntityId,
    kind: EdgeKind,
    traversal: Traversal,
) -> Result<HashSet<EntityId>, GraphError>;

pub fn dependencies(&self, from: &EntityId) -> Result<HashSet<EntityId>, GraphError>;
pub fn dependents(&self, from: &EntityId) -> Result<HashSet<EntityId>, GraphError>;
pub fn upstream(&self, from: &EntityId) -> Result<HashSet<EntityId>, GraphError>;
pub fn downstream(&self, from: &EntityId) -> Result<HashSet<EntityId>, GraphError>;
pub fn neighbors(
    &self,
    from: &EntityId,
    kind: EdgeKind,
) -> Result<HashSet<EntityId>, GraphError>;
```

`dependencies` and `dependents` use `DependsOn`; `upstream` and `downstream` use `RoutesThrough`; `neighbors` is for `CommunicatesWith`. Decision: approved this rename.

### 9.4 `blast_radius` starting-node inclusion is ambiguous

Sources: `docs/v0.1/04-mathematical-model.md` §6.7 line 346; §6.9 lines 409 and 412.

Problem: the formula sums over `u ∈ Reach(v)`. If `leadsto` is irreflexive, `v` is excluded; if reachability is reflexive or a cycle returns to `v`, `v` is included. The prose “who depends on me” suggests exclusion, but the spec does not say explicitly. §6.9 separately combines `C_i` and `B_hat_i` in a noisy-OR, so including `v` would partially double-count criticality.

Proposal: `blast_radius` excludes the starting node. BFS starts with `visited = {v}`. Return `(raw_sum, normalized)`. If the team wants reflexive reachability, the formula and implementation must be amended together.

### 9.5 Normalized blast-radius denominator is correct, with a freshness requirement

Source: `docs/v0.1/04-mathematical-model.md` §6.7 line 346.

Finding: dividing by `sum(C_u for u in V)` matches the specification. `NodeAttrs.criticality` must be refreshed each tick because §6.8 line 388 defines `C_i` counterfactually from the current mission score. If total criticality is `0.0`, return normalized `0.0`. No formula change is proposed.

### 9.6 `communication_score` needs explicit path semantics

Source: `docs/v0.1/04-mathematical-model.md` §6.7 lines 341, 352, 355; worked example lines 395–398.

Problems and corrections:

- Active/inactive: only `RoutesThrough` edges with `active == true` contribute. A standby link stays in the graph but is excluded until a reroute activates it.
- Active nodes: the path must also exclude isolated nodes. `communication_score` needs `active_nodes`, matching existing behavior in `crates/aeris-core/src/lib.rs:139–149` and §6.7 line 358's use of active nodes.
- Direction: enumerate directed paths from `source` to `target` along `RoutesThrough`; do not treat edges as undirected.
- Trust: multiply trust only for interior nodes, not source or target, per line 352. The trust provider should be clamped to `[0.0, 1.0]`.
- Cycles: enumerate simple paths with a visited set. Without this, cyclic graphs can loop or double-count.
- No path: return `0.0`, not an error. §6.8 line 397 shows `c_comm = 0` when B is removed and L2 is still off.
- Multiple paths: compute `max(prod(availability) * prod(interior trust))`. Do not sum paths.

Proposed signature:

```rust
pub fn communication_score(
    &self,
    source: &EntityId,
    target: &EntityId,
    active_nodes: &HashSet<EntityId>,
    trust: impl Fn(&EntityId) -> f64,
) -> Result<f64, GraphError>;
```

### 9.7 `observation_score` and `navigation_score` are missing active-node inputs

Source: `docs/v0.1/04-mathematical-model.md` §6.7 lines 358, 361, 364, 367; `docs/spec/params.toml` lines 42–50.

Problems:

- Both equations range over `v in active`. The proposed signatures iterate over the whole graph and cannot exclude isolated nodes.
- `c_nav` uses `sqrt(P_v)`, where `P_v` is Kalman covariance (§6.7 line 367). A parameter named `sigma` is ambiguous: it could mean variance or standard deviation.
- `c_obs` needs `cov_v`. `params.toml:50` gives `0.60`, but the API must state whether coverage comes from a provider, a node field, or an `Observes` edge field.

Proposed signatures:

```rust
pub fn observation_score(
    &self,
    active_nodes: &HashSet<EntityId>,
    coverage: impl Fn(&EntityId) -> f64,
    sensor_trust: impl Fn(&EntityId) -> f64,
) -> f64;

pub fn navigation_score(
    &self,
    active_nodes: &HashSet<EntityId>,
    position_variance: impl Fn(&EntityId) -> f64,
    sigma_max: f64,
) -> f64;
```

`navigation_score` computes `sqrt(position_variance(v))` internally. Decision: approved coverage as a provider; no coverage field was added to `NodeAttrs` or `EdgeAttrs`.

### 9.8 `NodeAttrs` does not fully represent live vertex estimates

Source: `docs/v0.1/04-mathematical-model.md` §6.7 lines 338 and 341.

Problem: §6.7 says each vertex carries `(xhat_i, T_i, C_i, R_i)`. `NodeAttrs` currently carries only `id`, `criticality`, and `capability`. Trust is intentionally passed by closure in Section 6, but `R_i` and `xhat_i` are not represented, and `P_v` needed by navigation is also absent.

Decision: approved keeping `aeris-graph` structural. Dynamic values are supplied as follows: `T_i` via trust closure, `C_i` via `NodeAttrs.criticality`, `P_v` via variance closure, `R_i` computed in `aeris-mission` from §6.9, and `xhat_i` is not needed by v0.1 graph equations.

### 9.9 Coupling and workspace practicality

Sources: `docs/v0.1/06-contracts.md` lines 43–69; `crates/aeris-types/src/lib.rs` lines 13–28; `Cargo.toml` line 13; `docs/v0.1/02-team-stack-layout.md` line 32.

Finding: no unnecessary coupling to `aeris-core` was found. Reusing `EntityId`, `LinkId`, and `Capability` from `aeris-types` is justified by the frozen shared contract. The proposed Rust syntax is valid in edition 2021: `impl Fn` arguments, derives, aliases, and `HashMap<EntityId, NodeIndex>` are all supported. `petgraph` is specified but not yet a workspace dependency. No compile check was possible in this environment because no Rust toolchain is on PATH.

Decision: approved adding `petgraph = "0.8.3"` to the workspace when implementation starts.
