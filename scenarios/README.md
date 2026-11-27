# Scenario fault schedules (S1–S8)

Owner: DHARSHAN. Format is parsed by `aeris_sim::Scenario::from_toml_str`; the
world they run in is `docs/spec/world.toml`.

Each file lists **what is injected, where and when** (ground truth). The
detection pipeline never reads these files: it only sees signed events that
the simulator derives from the true world state plus noise plus the attack
term, `O_t = h(S_t) + V_t + A_t^adv`. Expected AERIS behaviour is described
in `docs/v0.1/01-definition-of-done.md`; it is not encoded here.

| File | Fault kinds |
|---|---|
| s1.toml | none |
| s2.toml | `gps_failure` on node-a |
| s3.toml | `link_degradation` on link-l1 |
| s4.toml | `compromise` of node-b |
| s5.toml | `false_telemetry` from node-b |
| s6.toml | `gps_failure` on node-a + `compromise` of node-b |
| s7.toml | `partition` of hops b–c and a–c for 20 s |
| s8.toml | `model_mismatch` (unmodelled accelerations) |

Fault kinds and their fields are documented on `aeris_sim::scenario::FaultKind`.
`start_s` is when the fault begins; `end_s` (optional) when it is removed.
