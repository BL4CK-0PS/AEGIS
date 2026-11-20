# 06 · Contracts: the interfaces you must not break

[← Index](README.md)

These are frozen at G0. A change to anything in this document needs a pull request reviewed by Kishanth and announced at standup. Below are sketches; the source of truth is the code in `aeris-types` and `proto/`.

## Wire format: `proto/aeris/v1/event.proto`

**Owner:** DHARSHAN

```proto
syntax = "proto3";
package aeris.v1;

// What travels over NATS. The signature covers the exact body bytes.
message Envelope {
  bytes body      = 1;   // serialized Body
  bytes signature = 2;   // Ed25519(sk_source, body)
}

message Body {
  string id        = 1;  // UUIDv7
  string source    = 2;  // "node-b" or "node-b/gps"
  uint64 seq       = 3;  // strictly increasing per source
  Hlc    hlc       = 4;
  int64  t_unix_ms = 5;
  bool   replayed  = 6;  // true when sent from the partition buffer (S7)
  oneof payload {
    Telemetry     telemetry = 10;  // cpu, mem, process count
    NetStats      net       = 11;  // pkts/s, bytes/s, peers, latency, loss
    AuthEvent     auth      = 12;  // service, success, principal
    ProcessEvent  proc      = 13;  // started/stopped, name, hash
    SensorReading sensor    = 14;  // kind (gps/imu/camera), value, sigma
    LinkStatus    link      = 15;  // link id, up, availability
  }
}
message Hlc { int64 l = 1; uint32 c = 2; }

// NATS subjects:  aeris.events.<node>   (simulator → server)
//                 aeris.cmd.<node>      (server → simulator, commands)
```

## Core Rust types: `aeris-types`

**Owner:** KISHANTH

```rust
pub struct Evidence { pub id: EvidenceId, pub entity: EntityId, pub kind: EvidenceKind,
                      pub score: f64, pub event: EventId, pub group: Option<GroupId> }
pub struct Posterior { pub normal: f64, pub compromise: f64, pub sensor_failure: f64,
                       pub misconfig: f64, pub environment: f64 }
pub struct Beta { pub alpha: f64, pub beta: f64 }          // mean(), var()
pub struct TrustVector { pub dims: [Beta; 6] }               // identity … provenance
pub enum Action { Monitor, Restrict(EntityId), Isolate(EntityId), Reroute(LinkId),
                  Failover { from: EntityId, to: EntityId }, Degrade(Capability),
                  Recover(EntityId), Combo(Vec<Action>) }
pub struct Candidate { pub action: Action, pub p_success: f64, pub se: f64,
                       pub exp_utility: f64, pub cvar: f64, pub score: f64,
                       pub violations: Vec<Invariant> }
pub enum Gate { Auto, HumanReview { reason: String }, Reject { invariant: Invariant } }
pub enum Mode { Normal, Degraded, Containment, HumanReview, Recovery, Failsafe }
pub struct CycleOutput { pub tick: u64, pub mode: Mode, pub trust: Vec<(EntityId, TrustVector)>,
                         pub capabilities: Vec<(Capability, f64)>, pub mission_score: f64,
                         pub risks: Vec<(EntityId, f64)>, pub incidents: Vec<Incident>,
                         pub proposal: Option<Proposal> }

// aeris-core: the whole computational loop, with no I/O
pub fn step(state: &mut CoreState, events: &[VerifiedEvent], seed: u64) -> CycleOutput;
```

*Each algorithm crate exposes plain functions and structs over these types. No traits until a second implementation actually exists. The one exception is `aeris-exec::Adapter`, since v0.2 needs Linux and network adapters alongside the simulator.*

## HTTP API: `aeris-server`

**Owner:** KESIVARSHINI

| Method and path | Purpose | Who may call it |
|---|---|---|
| `POST /api/auth/login` | Username and password. Sets an httpOnly, Secure, SameSite=Strict session cookie. | anyone |
| `POST /api/auth/logout` | Ends the server-side session. | signed in |
| `GET /api/state` | Latest `CycleOutput`. | operator, commander |
| `GET /api/incidents?status=open` | List incidents. | operator, commander |
| `GET /api/incidents/{id}` | One incident with its evidence and posterior. | operator, commander |
| `GET /api/decisions/{id}` | Proposal, candidates table, explanation and evidence IDs. | operator, commander |
| `POST /api/decisions/{id}/approve` | Approve the recommended action. | role ≥ the action's impact class |
| `POST /api/decisions/{id}/modify` | Approve a different candidate; it is re-checked by policy. | role ≥ the new action's class |
| `POST /api/decisions/{id}/reject` | Reject, with a mandatory reason. | operator, commander |
| `GET /api/audit?after={n}` | Audit chain records. | commander |
| `GET /api/audit/verify` | Recompute the hash chain. Returns `{ok, broken_at}`. | commander |
| `GET /api/ws` | WebSocket stream (below). | operator, commander |
| `POST /api/sim/scenario` | Start a scenario with a seed. Staging builds only. | commander |

> **Object-level authorization.** An approval is a single SQL statement that checks everything at once:
>
> ```sql
> UPDATE decisions SET status='APPROVED' …
> WHERE id=$1 AND status='HUMAN_REVIEW' AND required_role <= $role
>   AND incident_id IN (SELECT id FROM incidents WHERE status='open')
> ```
>
> If zero rows change, the response is 404 or 409. The code never loads a row by ID and checks permissions afterwards.

**WebSocket messages** (JSON, using the `aeris-types` serde shapes): `cycle` (at most 5 per second, as diffs), `incident_opened`, `proposal`, `decision_updated`, `verification`, `mode_changed`.

## Database: `migrations/0001_init.sql`

**Owner:** KESIVARSHINI

| Table | Key columns |
|---|---|
| `operators` | id, username (unique), pw_hash (argon2id), role (operator \| commander) |
| `tower_sessions` | created by tower-sessions-sqlx-store |
| `assets`, `links` | id, kind, name · id, a, b, availability, active |
| `missions` | id, name, thresholds (jsonb) |
| `events` | id, source, seq, hlc_l, hlc_c, t, type, payload (jsonb), sig_ok, replayed · **unique (source, seq)** |
| `evidence` | id, event_id, entity, kind, score, group_id |
| `incidents` | id, entity, opened_at, closed_at, status, posterior (jsonb) |
| `decisions` | id, incident_id, status, required_role, proposal (jsonb), chosen_action (jsonb), gate, created_at |
| `approvals` | id, decision_id, operator_id, verdict, reason, at |
| `audit_log` | n (serial), decision_id, record (bytea), hash (bytea), sig (bytea) |

## Frozen config files (no magic numbers in code)

| File | Contents | Owner |
|---|---|---|
| `docs/spec/world.toml` | Nodes, sensors, links (L2 standby), coverage, capability minimums. | DHARSHAN |
| `docs/spec/params.toml` | Every parameter in these docs ($z_0, \kappa, \lambda_{\text{ewma}}, L, k, h, \gamma, T_{\min}, \alpha, \lambda, \varepsilon, \theta_U, \tau_h, N, K$), the $d(a)$ table and the thresholds. | KISHANTH |
| `docs/spec/likelihoods.toml` | Priors and $P(E_k \mid H)$ for fusion. | AMIRTHA |
| `scenarios/s1.toml … s8.toml` | Fault schedules (what is injected, where and when). | DHARSHAN |
