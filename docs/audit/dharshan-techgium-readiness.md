# Dharshan — Techgium readiness (AERIS v0.1, 2026-11-26)

Everything marked ✅ below was **run on this machine today** and the output quoted is what it printed. Nothing is
committed (no git identity on this machine); all work is in the working tree of
`dharshan/v0.1-infra-crypto-sim-exec`. Plan and gap analysis: [completion plan](dharshan-completion-plan.md).

## 1. What is actually implemented

| Capability | Where | Verified by |
|---|---|---|
| Simulated world: nodes A/B/C, L1 primary, L2 standby, 10 Hz, seeded ChaCha8 streams | `crates/aeris-sim`, `docs/spec/world.toml` | 35 sim unit tests; `identical_seed_gives_identical_run` |
| S1–S8 fault injection (attacker, GPS, link loss/latency, false telemetry, partition, model mismatch) | `scenarios/s*.toml`, `aeris-sim::scenario` | sim tests; 100-seed sweep (§4.4) |
| Signed events (Ed25519 over exact body bytes, domain-separated), per-node sequence, replay window incl. S7 buffered replay | `aeris-wire`, `aeris-crypto` | 33 crypto tests; NATS e2e (§4.2) |
| JetStream publishing with acks, S7 buffer and replay, signed command channel, graceful shutdown | `apps/aeris-simd` | NATS e2e (§4.2) |
| Monte Carlo rollouts: `fork()` isolation, common random numbers, N = 500, H = 100 ticks per candidate, called by `aeris-core` stage 8 | `aeris-sim::rollout`, `aeris-core` | sim tests; live run (§3) prints the table |
| Approval gate, execution, verification ‖e‖ ≤ ε, replan/escalation | `crates/aeris-exec` | 20 exec tests; G2; no-approval test |
| BLAKE3 audit chain signed by the server key; verification from JSONL **and from PostgreSQL** (`migrations/0001_init.sql`) | `aeris-crypto::audit`, `apps/aeris-harness/src/pg.rs` | crypto tests; `pg_audit` test; tamper demo (§3.4) |
| Live closed loop over the running stack with a **typed human approval** | `apps/aeris-harness/src/live.rs` (`aeris-harness live`) | §3.2 |
| Compose: Postgres 18 + NATS 2.15 JetStream, health checks, ACLs, no default secrets | `deploy/` | running `healthy` |

## 2. Runtime requirements

* Linux/macOS, Docker + Compose v2, Rust ≥ 1.99 (`rust-toolchain.toml` pins 1.100.0 for rustup users; this machine
  used Arch's rustc 1.99.1 and builds cleanly).
* `deploy/.env` created from `deploy/.env.example` with every `CHANGE_ME` replaced; keys in `deploy/keys/`
  (`aeris-simd keygen`). Both are git-ignored.
* **This machine:** another project's Postgres owns port 5432, so `deploy/.env` here sets
  `POSTGRES_PORT=127.0.0.1:55432`. Always derive the port from `.env` (commands below do).
* Build once in release before the demo (≈ 2 min): `cargo build --release -p aeris-simd -p aeris-harness`.

## 3. Demo runbook

### 3.1 Start the infrastructure

```bash
cd AEGIS
docker compose -f deploy/compose.yaml up -d --wait          # prints "Healthy" for nats and postgres
docker ps --format '{{.Names}}\t{{.Status}}' | grep aeris   # aeris-nats-1 / aeris-postgres-1 ... (healthy)
set -a; . deploy/.env; set +a                               # in every terminal used below
```

### 3.2 Golden path S4, live over NATS (main demo)

Terminal 1 — control-plane loop (connects as the NATS *server* user):

```bash
AERIS_NATS_USER="$NATS_SERVER_USER" AERIS_NATS_PASSWORD="$NATS_SERVER_PASSWORD" \
  ./target/release/aeris-harness live --key-dir deploy/keys --duration-s 75 --audit-out /tmp/live-audit.jsonl
```

Terminal 2 — simulated edge (connects as the NATS *simd* user):

```bash
AERIS_NATS_USER="$NATS_SIMD_USER" AERIS_NATS_PASSWORD="$NATS_SIMD_PASSWORD" RUST_LOG=warn \
  ./target/release/aeris-simd run --scenario s4 --seed 7 --key-dir deploy/keys --hold
```

What terminal 1 printed in today's run (abridged, verbatim values):

```text
[ 30.3s] events admitted 4323 rejected 0 | mode Normal | trust node-a=0.980 node-b=0.980 node-c=0.980 | mission 1.000
[ 32.2s] INCIDENT inc-01 on node-b | risk 0.987 | P(compromise) 0.947 | evidence 2
[ 32.2s] PROPOSAL dec-314 for inc-01 — candidates (500 Monte Carlo rollouts each):
  action                                         P(succ)     ±se     CVaR     J(a)  invariants
  Monitor                                          0.044   0.009    1.000   -0.456  ok
  Restrict node-b                                  0.044   0.009    1.050   -0.531  ok
  Reroute via link-l2                              0.450   0.022    1.050   -0.125  ok
  Isolate node-b                                   0.000   0.000    1.100   -0.650  VETO I1: Never remove last active communication path; VETO I2: ...
  Isolate node-b + Reroute via link-l2             0.960   0.009    0.550    0.535  ok
  Degrade communication                            0.420   0.022    1.300   -0.530  ok
  RECOMMENDED: Isolate node-b + Reroute via link-l2 | gate HumanReview { reason: "Action has High impact on mission (criticality 1.00)" }
  HUMAN REVIEW REQUIRED: Action has High impact on mission (criticality 1.00)
  Type 'approve' to execute "Isolate node-b + Reroute via link-l2" as commander commander-1, anything else rejects:
  COMMAND ActivateLink { link: "link-l2" } -> Some(Applied)
  COMMAND Isolate { node: "node-b" } -> Some(Applied)
  VERIFY ‖e‖ = 0.0000 -> OK | B mode Some(Isolated) | L2 active Some(true)
  EXECUTION STATE Verified
  AUDIT n=1 hash 1ddd79058829d202
```

Nothing executes until the presenter types `approve`; any other answer (or EOF) records a REJECT and sends no
command. **Values vary between live runs** (rollout seed is `seed ^ tick`, and the tick at which the incident
opens depends on wall-clock delivery): e.g. J/P were 0.367/0.932 and 0.535/0.960 in two runs today. The
recommendation was `Isolate node-b + Reroute via link-l2` in every run. For fixed numbers use §3.3.

Verify the audit chain written by the live run:

```bash
./target/release/aeris-harness verify-audit --file /tmp/live-audit.jsonl --registry deploy/keys/public-keys.toml
# {"ok": true, "verified": 1, "broken_at": null, ...}   exit 0
```

### 3.3 Reproducible in-process run (fallback; identical every time)

```bash
AERIS_SERVER_KEY=deploy/keys/server.key ./target/release/aeris-harness run --scenario s4 --seed 7 --approve --narrate --audit-out /tmp/s4.jsonl
```

Printed today (≈ 1.3 s):

```text
[ 30.0s] INCIDENT inc-01 on node-b (risk 0.987, P(compromise) 0.947)
[ 30.0s] PROPOSAL dec-300 gate=HUMAN_REVIEW
[ 30.0s] EXECUTE Isolate node-b + Reroute via link-l2 by test commander -> Verified
```

followed by the JSON run result. `--approve` enables a **test** commander and is labelled `human-test` in the audit
record; say so if you show it.

### 3.4 Audit chain in PostgreSQL, and tamper detection

```bash
PGP=${POSTGRES_PORT##*:}
docker compose -f deploy/compose.yaml exec -T postgres psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" \
  -qc "DROP DATABASE IF EXISTS aeris_demo" -c "CREATE DATABASE aeris_demo"
export AERIS_DATABASE_URL="postgres://$POSTGRES_USER:$POSTGRES_PASSWORD@127.0.0.1:$PGP/aeris_demo"
AERIS_SERVER_KEY=deploy/keys/server.key ./target/release/aeris-harness run --scenario s4 --seed 7 --approve --store-pg >/dev/null
#   stored 1 audit records in PostgreSQL audit_log
./target/release/aeris-harness verify-audit --pg --registry deploy/keys/public-keys.toml
#   {"ok": true, "verified": 1, ...}   exit 0
docker compose -f deploy/compose.yaml exec -T postgres psql -U "$POSTGRES_USER" -d aeris_demo -c \
  "UPDATE audit_log SET record = convert_to(replace(convert_from(record,'UTF8'), '\"VERIFIED\"', '\"VERIFIED \"'),'UTF8') WHERE n=1"
./target/release/aeris-harness verify-audit --pg --registry deploy/keys/public-keys.toml
#   {"ok": false, "verified": 0, "broken_at": 1, "failure": {"kind": "hash_mismatch", "n": 1}}   exit 2
```

The schema is Kesivarshini's `migrations/0001_init.sql`, applied unchanged; rows go to `incidents`, `decisions`
(status `VERIFIED`, gate `human_review`, role `commander`) and `audit_log`.

### 3.5 Security demos (each prints `ok`)

```bash
export AERIS_TEST_NATS_URL=nats://127.0.0.1:${NATS_PORT##*:} \
  AERIS_TEST_SIMD_USER="$NATS_SIMD_USER" AERIS_TEST_SIMD_PASSWORD="$NATS_SIMD_PASSWORD" \
  AERIS_TEST_SERVER_USER="$NATS_SERVER_USER" AERIS_TEST_SERVER_PASSWORD="$NATS_SERVER_PASSWORD"
cargo test --release -p aeris-harness --test nats_e2e -- --ignored --test-threads=1 --nocapture
```

Covers forged and replayed envelopes over JetStream, forged commands (wrong key → signed REJECTED ack, no
effect), idempotent commands, S7 partition buffer replayed once in order, and the golden S4 over NATS.

## 4. Verified test results (today)

### 4.1 Static checks

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | ✅ clean (after `cargo fmt`; tree had drift) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ clean (after 3 fixes) |
| `cargo deny check` (cargo-deny 0.22.1) | ✅ exit 0: advisories ok, bans ok, licenses ok, sources ok; 18 duplicate-version warnings (policy `warn`) |

### 4.2 Tests

| Suite | Result |
|---|---|
| `cargo test --workspace --all-features` (unit + integration, includes G2 ×20) | RESULT_WORKSPACE |
| G2 `g2_s4_twenty_seeds_ai_off` (20 seeds) | RESULT_G2 |
| `without_approval_high_impact_never_runs` | ✅ ok — no execution on either seed |
| `identical_seed_gives_identical_run` | ✅ ok |
| aeris-graph (Kesivarshini, imported from `origin/main`) | ✅ 6 passed inside this workspace |
| NATS e2e, live JetStream (`nats_e2e --ignored`) | ✅ 6 passed (after fixing the forged-command ack, §6) |
| PostgreSQL audit (`pg_audit --ignored`) | ✅ 1 passed (round trip, signature verify, refuse second chain, tamper → `HashMismatch{n:1}`) |
| Live loop (§3.2) | ✅ incident on node-b, isolate+reroute recommended, executed only after approval, verified, audited |

### 4.3 G2 (S4 × 20 seeds, AI off)

RESULT_G2_DETAIL

### 4.4 S1–S8 × 100 seeds (`docs/testing/results/g4-s1-s8-100seeds/`)

RESULT_G4

## 5. Script claims (`docs/spec/demo-script.md`) — keep, correct, or drop

| Claim in the script | Status | Say instead |
|---|---|---|
| Three drones A, B, C; L1 primary, L2 standby | ✅ true | — |
| B compromised at t = 30 s: beaconing, SSH brute force, rogue process, camera disagreement | ✅ simulated (`scenarios/s4.toml`) | — |
| "Bayesian fusion… compromise with 95 % certainty", posterior 94.7 % | ❌ **literal**: `aeris-core` stage 4 sets `compromise = 0.947` when any of three evidence kinds appears (`crates/aeris-core/src/lib.rs:461-472`). Real fusion is on Amirtha's unmerged branch | "Evidence from several sources raises an incident" — do not claim Bayesian fusion or the 95 % as computed |
| Trust in B "collapses from 0.90 to 0.565, Beta(8.0, 3.4) ± 0.13" | ❌ not reproduced. Live run: B's trust 0.980 → 0.965 (explanation: "0.97 (prior 0.90)") | Show the printed value; do not quote 0.57 |
| Comm capability 0.554, mission score 0.615, C_B = 1.00, risk 0.987 | ⚠ risk 0.987 and criticality 1.00 are printed; capability scores are a **literal table** in core (`lib.rs:167-175`: obs 0.920/0.815/0.570, nav 0.930) | Quote only risk and criticality |
| "500 counterfactual rollouts for each" option | ✅ real (N = 500, H = 100) | — |
| "Isolate B alone" vetoed by I1 | ✅ real (also flags I2) | — |
| Isolate + reroute "P(success) = 0.94, J(a) = 0.415" | ⚠ real computation, different numbers (e.g. 0.960 / 0.535 live; varies per live run) | Read the numbers off the screen |
| Explanation cites `ev-0412 … ev-0423` | ⚠ ids are generated by core and differ (e.g. `ev-0419, ev-0413`) | Don't pre-announce ids |
| Autonomy gate HUMAN_REVIEW; nothing runs without approval | ✅ real and tested | — |
| Click **APPROVE** in the console | ❌ no web console / button exists. Approval is typed in the `live` terminal | "The commander approves" (type `approve`) |
| Commands: activate L2, isolate B; graph redraws | ✅ commands real over signed NATS; ❌ no graph UI | Show the COMMAND/VERIFY lines |
| Verification: B isolated, L2 active, capabilities restored, M(G) = 1.0 | ✅ B isolated + L2 active verified (‖e‖ = 0); ⚠ capability numbers are core's literal table | — |
| `aeris-server` on :8080, console on :8081, `trunk serve` | ❌ do not exist | Use the two-terminal runbook |
| `aeris-eval --suite all --seeds 100`, `aeris-eval verify-audit` | ❌ `aeris-eval` is on Amirtha's unmerged branch; equivalents: `aeris-harness sweep`, `aeris-harness verify-audit` | — |
| "Across 100 seeds: AERIS vs alert-only" | ✅ measured (§4.4) with the definitions in `apps/aeris-harness/src/sweep.rs` | Quote §4.4 numbers only |
| "AI explains; LLM fallback < 5 ms" | ❌ no LLM integration on this branch; explanation is a template in core | "Template explanation from computed values" |
| `cargo run -p aeris-console -- --demo` fallback | ⚠ runs, but prints a fixture (hand-written events), not the simulation | Use §3.3 as the fallback instead |

## 6. Defects fixed today

| Defect | Fix | Evidence |
|---|---|---|
| fmt drift; clippy `manual_is_multiple_of` ×2, `large_enum_variant` (simd CLI) | `cargo fmt`, `is_multiple_of`, boxed `RunArgs` | clippy clean |
| Forged command got an ack with id `unknown`; the sender's adapter rejected it as `BadAck` instead of seeing REJECTED | simd echoes the *claimed* id (if the envelope decodes) in the signed REJECTED ack | `commands_are_authenticated_and_idempotent_over_nats` ok |
| Sweep metrics: S6's primary target was node-a (GPS fault), so detecting B's compromise counted as FP and B's injected compromise as lateral spread | primary target = compromised node; incidents on any faulted node are not FP; lateral spread excludes scenario-compromised nodes | regenerated §4.4 |
| Nothing reached PostgreSQL | `pg.rs`, `run/live --store-pg`, `verify-audit --pg`, `pg_audit` test, CI step | §3.4 |
| No way to run the loop over the real stack outside a test | `aeris-harness live` with typed approval, key revocation after verified isolation | §3.2 |
| `cargo test` took > 30 min (debug crypto and core) | dev-profile `opt-level = 3` for hot crates (same code, same results) | `Cargo.toml` |
| Kesivarshini's `aeris-graph` + migration absent from this branch | imported verbatim from `origin/main` b0d41bc, added to workspace; `publish.workspace = true` added for cargo-deny | 6 graph tests pass here |

## 7. Known limitations (do not claim otherwise)

1. `aeris-server`, `aeris-policy`, `aeris-mission`, auth, web console: not built. The harness stands in; the policy
   re-check is `aeris-exec::TopologyGuard` (interim).
2. Detection/fusion/capabilities in `aeris-core` are stubs with literal values (Kishanth/Amirtha). Consequences in
   §4.4: S2, S3, S5, S7, S8 raise no incident, and S3 fails the mission for every strategy (core never receives link
   measurements). S1 mission success is 89 %, a property of the world parameters and mission definition, not a bug
   that was tuned away.
3. Amirtha's `origin/amirtha/ai-crates` is not merged; it rewrites the same `aeris-core` stages this branch changed
   and will conflict.
4. `aeris-graph` compiles and its tests pass here, but nothing calls it yet (its consumers are core/server).
5. Live-run numbers vary between runs (wall-clock timing); in-process runs are bit-reproducible.
6. TLS for NATS is not exercised locally; DC deployment not done.
7. `run --approve` uses a labelled test commander; only `live` takes a real typed approval.

## 8. Recovery if something fails during the demo

| Symptom | Action |
|---|---|
| `compose up` fails: `set POSTGRES_PASSWORD in deploy/.env` | `cp deploy/.env.example deploy/.env`, fill every `CHANGE_ME` |
| container unhealthy | `docker compose -f deploy/compose.yaml logs nats postgres`; `docker compose -f deploy/compose.yaml restart nats` |
| `postgres connect: password authentication failed` | wrong port: use `${POSTGRES_PORT##*:}` from `.env` (55432 here; 5432 is another project) |
| `audit_log already holds N rows` | use a fresh database (§3.4 `DROP/CREATE DATABASE aeris_demo`) |
| `live` shows no events | simd not running or wrong NATS user: simd uses `NATS_SIMD_*`, live uses `NATS_SERVER_*` |
| simd: `could not connect … 127.0.0.1:4222` | NATS down: `docker compose -f deploy/compose.yaml up -d --wait nats` |
| anything else | fall back to §3.3 (no infrastructure needed) |
