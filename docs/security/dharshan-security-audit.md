# Dharshan — Security audit of owned components (AERIS v0.1.0)

- **Date:** 2026-11-25 · **Branch:** `dharshan/v0.1-infra-crypto-sim-exec` (uncommitted, see implementation report)
- **Scope:** `aeris-wire`, `aeris-crypto`, `aeris-sim`, `aeris-exec`, `apps/aeris-simd`, `apps/aeris-harness`, `deploy/`, CI, workspace.
- **Out of scope (other owners):** `aeris-server`, `aeris-policy`, migrations (not on `main`); `aeris-core`/`aeris-types` logic beyond the integration points I changed.
- **Method:** design review against `docs/spec/threat-model.md` (T-01…T-06) and `docs/v0.1/04` §6.2/§6.13; adversarial unit/integration tests; live tests against NATS 2.15.3 with authentication; dependency/licence audit with cargo-deny. Every "retest" below names the test that demonstrates it.

## 1. Trust boundaries

```text
 simulated edge (aeris-simd)         bus (NATS, authenticated users,          control plane (aeris-server — not built;
   node keys sign events      ──▶     subject ACLs, JetStream)          ──▶   test harness stands in)
   sim-controller key signs acks ◀──  aeris.cmd.<entity> request/reply ◀──    server key signs commands + audit hashes
```

* Nothing on the bus is trusted: every event and every command is authenticated by signature, bound to its purpose by a domain tag, and checked against a replay window. NATS authentication and subject permissions are a second, independent layer.
* The detector never sees ground truth: `World::ground_truth()` is called only by the evaluator; observations carry no labels (`s4_compromise_shows_only_through_behaviour` greps the serialized observations for leaks).
* Execution trusts neither the recommendation nor the gate: `aeris-exec` re-derives the impact class and requires a matching human approval for anything non-LOW.

## 2. Cryptographic design

| Property | Implementation | Evidence |
|---|---|---|
| Algorithm | Ed25519 (`ed25519-dalek` 3.0.0, default features `fast`, `zeroize`; `legacy_compatibility` **off**) | `Cargo.toml` |
| Verification | `verify_strict` (rejects non-canonical `S`, small-order `R`/`A`) | `keys::verify` |
| Weak keys | `is_weak()` keys refused at parse and insert | `key_registry_round_trip_and_rejects_bad_keys` |
| Exact bytes | Receiver verifies the body bytes as received; never re-encodes | `signature_covers_exact_received_bytes` (appending an unknown field decodes to the same struct but fails verification) |
| Domain separation | `AERIS-v1/event\0`, `…/command\0`, `…/ack\0`, `…/audit\0` prefixes | `forged_events_are_rejected_and_attributed` case 4; `forged_commands_are_rejected` (event-domain signature by the real server key refused as a command) |
| Replay | `seq > last[source]` per source, `|t−now| ≤ 2 s`; S7 `replayed` events: seq only, not from the future, ≤ 300 s old | `replay::tests::*`, `replayed_and_duplicate_envelopes_are_rejected`, `s7_partition_buffer_is_accepted_once_in_order`, NATS `s7_partition_buffer_replays_once_over_jetstream` |
| Sequence overflow | Counters never wrap (`SequenceExhausted`); `u64::MAX` is terminal for a source | `counter_starts_at_one_and_refuses_to_wrap`, `max_sequence_is_terminal_for_a_source` |
| HLC | Counter overflow is an error; remote `l` > 2 s ahead rejected | `hlc::tests::*` |
| Audit chain | `h_0 = 0^256`, `h_n = BLAKE3(h_{n−1} ‖ bytes(D_n))`, `σ_n = Sign_srv(tag ‖ h_n)`; canonical JSON (sorted keys, NaN/∞ refused) | `chain_matches_formula`, `canonical_json_is_deterministic_and_sorted` |
| Tamper detection | modified record, recomputed hash, bad signature, missing, reordered, truncated (with checkpoint), empty chain | `tampered_record_is_detected_at_its_index`, `tamper_with_recomputed_hash_is_caught_by_signature_or_next_link`, `missing_record_is_detected`, `reordered_records_are_detected`, `invalid_signature_is_detected`, `truncation_needs_a_checkpoint_and_is_then_detected`, `empty_chain_verifies_to_genesis`; CLI retest below |

## 3. Key lifecycle

1. **Generation:** `aeris-simd keygen --out <dir>` draws 32-byte seeds from the OS CSPRNG (`getrandom`), writes `<id>.key` (hex, created with `O_EXCL` and mode `0600`, refuses to overwrite) and `public-keys.toml`. `.gitignore` excludes `*.key`, `deploy/keys/`, `deploy/.env`.
2. **Distribution:** only `public-keys.toml` is distributed (mounted read-only). Each service gets only its own secret: simd gets node keys + `sim-controller.key`; the server gets `server.key`. In the DC, `server.key` must be generated on the server host, not on a developer machine (deployment checklist).
3. **Loading:** refuses files readable/writable by group or others; parse errors never echo key material; `SigningIdentity` `Debug` prints `<redacted>`; secrets are `Zeroizing` buffers and `SigningKey` zeroizes on drop.
4. **Revocation:** `KeyRegistry::revoke(node)` — the harness revokes a node's key when its isolation verifies ("stop accepting B's key"). `aeris-server` must do the same.
5. **Rotation / exhaustion:** a sequence counter at `u64::MAX` or a suspected key compromise requires a new key and a registry update; there is no automatic rotation in v0.1.
6. **Assumptions:** keys of uncompromised nodes and the server are secret (threat model §2); clocks of server and simd are NTP-synchronised to well within 2 s (freshness for commands and non-replayed events depends on it).

## 4. Findings

Severity: CRITICAL / HIGH / MEDIUM / LOW. Status: FIXED (with retest), MITIGATED, OPEN (owner named).

| ID | Sev | Finding | Status / remediation | Retest |
|---|---|---|---|---|
| S-01 | CRITICAL | No event authentication existed (`signature_valid` was caller-supplied); forged/replayed events would be processed (T-03/T-04). | FIXED: `aeris-crypto` `EventVerifier` gate. | `forged_events_are_rejected_and_attributed`; NATS `forged_and_replayed_envelopes_are_rejected_over_jetstream` |
| S-02 | CRITICAL | `aeris-eval verify-audit` on `amirtha/ai-crates` prints a fixed success without reading anything. | MITIGATED: real `aeris-harness verify-audit` (+ `aeris_crypto::audit::verify_chain`). OPEN for the unmerged branch (owner: Amirtha/Dharshan, before merge). | CLI: valid chain exit 0 `{"ok":true}`; one-word edit → exit 2 `hash_mismatch n=1` |
| S-03 | HIGH | Commands to the simulator had no authenticity, so anyone able to publish could isolate nodes. | FIXED: server-signed commands, subject binding (command for node-b cannot be routed to node-a), replay window, signed acks; NATS ACLs restrict `aeris.cmd.>` publishing to the server user. | `commands_authenticate_and_bind_to_subject`, `forged_commands_are_rejected`; NATS `commands_are_authenticated_and_idempotent_over_nats` (forged isolate → `REJECTED`, node unchanged) |
| S-04 | HIGH | An auto gate (or a bug in it) could execute HIGH-impact actions. | FIXED: executor recomputes impact; AUTO only for LOW + reversible; human approval must name the exact action and have sufficient role. | `unauthorized_requests_send_nothing` (6 cases + NaN criticality), harness `without_approval_high_impact_never_runs`, sweep column "Unapproved non-LOW" = 0 |
| S-05 | HIGH | `Action::Recover` is LOW impact in `aeris-types`, so AUTO could re-admit a compromised node without the §6.13 validation (not built in v0.1). | MITIGATED in `aeris-exec` (Recover always needs a human). OPEN in `aeris-types` (owner: Kishanth) — reclassify. | `authz::tests::recover_never_runs_on_auto` |
| S-06 | HIGH | Policy is evaluated at recommendation time only; the world may change before execution. | FIXED: executor re-checks against a fresh observation; interim `TopologyGuard` enforces I1/I2/I3 until `aeris-policy` exists. OPEN: wire `aeris-policy` (Kesivarshini). | `policy_veto_blocks_isolating_the_last_relay` |
| S-07 | MEDIUM | Verification judged on topology alone would report success while a distrusted relay still carries traffic. | FIXED: verification uses AERIS's trust estimate (§6.7 interior-trust product); never ground truth. | `monitor_sends_nothing_and_is_judged_on_the_mission`, `verification_uses_trust_not_ground_truth` |
| S-08 | MEDIUM | A compromised node may set `replayed=true` to bypass the 2 s freshness bound. | MITIGATED: flag is signed (only the key holder), `seq > last` still applies, max age 300 s, never from the future. Residual: a compromised node can inject ≤300 s-old *new* readings. | `replayed_events_are_judged_on_sequence_not_freshness` |
| S-09 | MEDIUM | Integrity evidence is attributed to the *claimed* source; forged traffic could be used to lower an innocent node's trust (DoS, T-04). | MITIGATED: rejections aggregated to ≤1 evidence item per (node, reason) per drain; unattributable ones only counted; NATS ACLs prevent unauthenticated publishers. OPEN: weighting is a trust/fusion decision (Kishanth/Amirtha); `aeris-core::step` has no evidence input, so the evidence is produced but not yet consumed. | `forged_events_are_rejected_and_attributed` |
| S-10 | MEDIUM | Non-canonical UUID spellings could bypass id-based de-duplication. | FIXED: only canonical lowercase hyphenated UUIDv7 accepted. | `rejects_invalid_field_values` (uppercase, braces, v4) |
| S-11 | MEDIUM | Free-text fields could inject control characters into logs. | FIXED: control characters rejected in every text field. | `rejects_invalid_field_values` ("log injection") |
| S-12 | MEDIUM | Resource exhaustion: unbounded dedupe set in `aeris-core` (`processed_event_seqs`). | OPEN (owner: Kishanth). My components are bounded: 64 KiB envelopes (checked before decode), ≤8 sensor values, ≤256-byte strings, replay history 1024/source, sources limited to the registry, simd outbox 65 536 with back-pressure, partition buffer per node (oldest dropped and logged at error level, counted), command/ack cache 1024, executor ledger 10 000. | `envelope_limits_and_signature_length`, `history_is_bounded` |
| S-13 | MEDIUM | NaN/∞ in audit records would silently serialize as `null`. | FIXED: canonical serializer refuses non-finite floats. | `canonical_json_is_deterministic_and_sorted` |
| S-14 | MEDIUM | Audit truncation (dropping the newest records) is undetectable by a hash chain alone. | MITIGATED: `Checkpoint {n, head}` verification. OPEN: `aeris-server` must publish signed checkpoints with each decision. | `truncation_needs_a_checkpoint_and_is_then_detected` |
| S-15 | LOW | Rejection reasons are returned to the command sender (`bad signature`, `stale`). | ACCEPTED: the sender already controls the input; no secrets are disclosed. | — |
| S-16 | LOW | Toolchain pinned to a Windows-GNU triple; non-reproducible builds. | FIXED: pinned 1.100.0. | Workspace builds and tests on `rust:1.100.0` image (test report) |
| S-17 | LOW | GitHub Actions referenced by tag, not commit SHA. | OPEN: pin to SHAs when the repository moves to GitHub org settings with Dependabot. | — |

## 5. Infrastructure

| Control | State | Evidence |
|---|---|---|
| No committed credentials | `deploy/.env` and keys are git-ignored; compose has no default passwords (`${VAR:?}` fails fast) | `git check-ignore -v deploy/.env`; `docker compose config` errors without `.env` |
| NATS authentication + least privilege | Two users; simd may publish only `aeris.events.>` and its stream's API, subscribe only `aeris.cmd.>`; server may publish `aeris.cmd.>` and JetStream API | `deploy/nats/nats-server.conf`; `auth_required: true` from `/varz`; NATS tests run with these users |
| Network exposure | Ports bound to `127.0.0.1` by default; Postgres moved to 55432 locally to avoid clashing with an unrelated container | `docker compose ps` |
| Container privileges | NATS: uid 1000, read-only rootfs, `cap_drop: ALL`, `no-new-privileges`. Postgres: runtime uid 999, `cap_drop: ALL` + the 5 caps its entrypoint needs. simd image: uid 10001 (compose overrides to the key owner's uid), read-only, `cap_drop: ALL` | `docker top`, `docker inspect` output in the test report |
| TLS | Not enabled locally. NATS TLS template (`deploy/nats/tls.conf.example`) and client CA support (`AERIS_NATS_TLS_CA`) prepared; not exercised (no DC certificates available) | deployment checklist |
| Postgres auth | `scram-sha-256` for host connections | compose `POSTGRES_INITDB_ARGS` |
| Unsafe Rust | `#![forbid(unsafe_code)]` in every owned crate | source |

## 6. Dependency audit

See the test report §cargo-deny for the exact output. Licences are restricted to permissive ones; `openssl` is banned (rustls only); unknown registries/git sources are denied.

## 7. Residual risks

1. `aeris-server` and `aeris-policy` do not exist; the production ingest, approval store and policy are therefore untested. The executor and verifier are ready to be called by them.
2. Without `aeris-server` checkpoints, audit truncation is detectable only when a checkpoint is supplied.
3. A compromised node holds a valid key: it can send well-formed lies (S5). That is by design detected behaviourally, not cryptographically.
4. No mTLS between services (v0.2 per `08-rules-cuts-risks.md`).
