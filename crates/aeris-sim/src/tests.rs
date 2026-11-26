use super::*;
use aeris_types::{Action as A, Capability};
use aeris_wire::{CommandOp, NodeMode, SensorKind, WirePayload, commands_for_action};
use std::sync::Arc;

fn cfg() -> Arc<WorldConfig> {
    Arc::new(WorldConfig::frozen().unwrap())
}

fn world(id: &str, seed: u64) -> World {
    let c = cfg();
    let s = Arc::new(Scenario::builtin(id, &c).unwrap());
    World::new(c, s, seed).unwrap()
}

fn run(w: &mut World, ticks: u64) -> Vec<Observation> {
    (0..ticks).flat_map(|_| w.step()).collect()
}

fn of<'a>(obs: &'a [Observation], node: &'a str) -> impl Iterator<Item = &'a Observation> {
    obs.iter().filter(move |o| o.node == node)
}

fn sensor(o: &Observation, k: SensorKind) -> Option<[f64; 2]> {
    match &o.payload {
        WirePayload::Sensor { kind, value, .. } if *kind == k => Some([value[0], value[1]]),
        _ => None,
    }
}

fn cpu(o: &Observation) -> Option<f64> {
    match o.payload {
        WirePayload::Telemetry { cpu_pct, .. } => Some(cpu_pct),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[test]
fn frozen_world_matches_spec_topology() {
    let c = cfg();
    let ids: Vec<_> = c.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["node-a", "node-b", "node-c"]);
    let l1 = &c.links[c.link_index("link-l1").unwrap()];
    assert_eq!(l1.path(), ["node-a", "node-b", "node-c"]);
    assert!(l1.active);
    let l2 = &c.links[c.link_index("link-l2").unwrap()];
    assert_eq!(l2.path(), ["node-a", "node-c"]);
    assert!(!l2.active, "L2 is standby at start");
    assert_eq!(c.world.tick_hz, 10);
}

#[test]
fn world_minima_agree_with_params_toml() {
    // params.toml (owner K) and world.toml (owner D) both state the mission
    // minima and coverage; they must never drift apart.
    let p: toml::Value = toml::from_str(include_str!("../../../docs/spec/params.toml")).unwrap();
    let m = &p["mission"];
    let c = cfg();
    assert_eq!(m["c_min_nav"].as_float(), Some(c.mission.min.navigation));
    assert_eq!(
        m["c_min_comm"].as_float(),
        Some(c.mission.min.communication)
    );
    assert_eq!(m["c_min_obs"].as_float(), Some(c.mission.min.observation));
    assert_eq!(m["sigma_max"].as_float(), Some(c.mission.sigma_max_m));
    for n in &c.nodes {
        assert_eq!(m["area_coverage_per_node"].as_float(), Some(n.coverage));
    }
    let d = &p["decision"];
    let spec = RolloutSpec::v01(0);
    assert_eq!(d["rollouts_n"].as_integer(), Some(spec.n as i64));
    assert_eq!(
        d["horizon_k"].as_integer(),
        Some(i64::from(spec.horizon_ticks))
    );
    assert_eq!(
        p["system"]["tick_rate_hz"].as_float(),
        Some(f64::from(c.world.tick_hz))
    );
}

#[test]
fn healthy_capabilities_follow_the_formulas() {
    let w = world("s1", 1);
    let caps = w.capabilities();
    let s = &w.config().sensors;
    // c_nav = 1 - sigma/30 with sigma from GPS (2.5) and camera (4.0) fused.
    let sigma = 1.0 / (1.0 / s.gps_sigma_m.powi(2) + 1.0 / s.camera_sigma_m.powi(2)).sqrt();
    assert!((caps.navigation - (1.0 - sigma / 30.0)).abs() < 1e-12);
    // c_comm = 0.99 * 0.99 * (honest relay) over L1.
    assert!((caps.communication - 0.99 * 0.99).abs() < 1e-12);
    // c_obs = 1 - (1 - 0.6 * 0.948)^3
    assert!((caps.observation - (1.0 - (1.0 - 0.6 * 0.948f64).powi(3))).abs() < 1e-12);
    assert!(w.mission_ok());
}

#[test]
fn all_eight_scenarios_parse() {
    let c = cfg();
    for (id, _) in scenario::BUILTIN {
        let s = Scenario::builtin(id, &c).unwrap();
        assert_eq!(s.scenario.id, id);
        assert_eq!(s.duration_ticks(10), 600);
    }
    let kinds = |id: &str| -> Vec<&'static str> {
        Scenario::builtin(id, &c)
            .unwrap()
            .faults
            .iter()
            .map(Fault::kind)
            .collect()
    };
    assert!(kinds("s1").is_empty());
    assert_eq!(kinds("s2"), ["gps_failure"]);
    assert_eq!(kinds("s3"), ["link_degradation"]);
    assert_eq!(kinds("s4"), ["compromise"]);
    assert_eq!(kinds("s5"), ["false_telemetry"]);
    assert_eq!(kinds("s6"), ["gps_failure", "compromise"]);
    assert_eq!(kinds("s7"), ["partition"]);
    assert_eq!(kinds("s8"), ["model_mismatch"]);
    assert!(Scenario::builtin("s9", &c).is_err());
}

#[test]
fn malformed_scenarios_are_rejected() {
    let c = cfg();
    let head = "[scenario]\nid = \"x\"\nname = \"n\"\ndescription = \"d\"\nduration_s = 60.0\n";
    let mm = "kind = \"model_mismatch\"\naccel_sigma_mps2 = 1.0\n";
    let bad = [
        ("missing header", format!("[[faults]]\n{mm}start_s = 1.0\n")),
        ("not toml", "this is = = not toml".to_string()),
        (
            "unknown kind",
            format!("{head}[[faults]]\nkind = \"meteor\"\nstart_s = 1.0\n"),
        ),
        (
            "unknown field",
            format!("{head}[[faults]]\n{mm}start_s = 1.0\ntypo = 3\n"),
        ),
        (
            "wrong type",
            format!("{head}[[faults]]\n{mm}start_s = \"soon\"\n"),
        ),
        (
            "unknown node",
            format!(
                "{head}[[faults]]\nkind = \"gps_failure\"\nnode = \"node-z\"\nstart_s = 1.0\nerror_sigma_m = 1.0\nbias_walk_mps = 0.0\n"
            ),
        ),
        (
            "unknown link",
            format!(
                "{head}[[faults]]\nkind = \"link_degradation\"\nlink = \"link-l9\"\nstart_s = 1.0\nextra_latency_ms = 1.0\nextra_loss = 0.1\n"
            ),
        ),
        (
            "negative start",
            format!("{head}[[faults]]\n{mm}start_s = -1.0\n"),
        ),
        (
            "end before start",
            format!("{head}[[faults]]\n{mm}start_s = 10.0\nend_s = 5.0\n"),
        ),
        (
            "starts after the run",
            format!("{head}[[faults]]\n{mm}start_s = 61.0\n"),
        ),
        (
            "probability > 1",
            format!(
                "{head}[[faults]]\nkind = \"link_degradation\"\nlink = \"link-l1\"\nstart_s = 1.0\nextra_latency_ms = 1.0\nextra_loss = 1.5\n"
            ),
        ),
        ("NaN", format!("{head}[[faults]]\n{mm}start_s = nan\n")),
        (
            "no such hop",
            format!(
                "{head}[[faults]]\nkind = \"partition\"\nhops = [[\"node-b\", \"node-b\"]]\nstart_s = 1.0\n"
            ),
        ),
        (
            "empty partition",
            format!("{head}[[faults]]\nkind = \"partition\"\nhops = []\nstart_s = 1.0\n"),
        ),
        ("zero duration", head.replace("60.0", "0.0")),
        ("bad id", head.replace("\"x\"", "\"S.4\"")),
    ];
    for (name, text) in bad {
        assert!(
            Scenario::from_toml_str(&text, &c).is_err(),
            "accepted: {name}"
        );
    }
    // A compromise without its attacker block.
    let s4 = include_str!("../../../scenarios/s4.toml");
    let cut = &s4[..s4.find("[faults.attacker]").unwrap()];
    assert!(Scenario::from_toml_str(cut, &c).is_err());
}

#[test]
fn malformed_world_is_rejected() {
    let good = config::FROZEN_WORLD_TOML;
    for (name, text) in [
        (
            "hop count",
            good.replace(
                "hop_availability = [0.99, 0.99]",
                "hop_availability = [0.99]",
            ),
        ),
        (
            "unknown via",
            good.replace("via = [\"node-b\"]", "via = [\"node-q\"]"),
        ),
        (
            "bad prob",
            good.replace("fade_prob_per_tick = 0.0005", "fade_prob_per_tick = 2.0"),
        ),
        ("zero hz", good.replace("tick_hz = 10", "tick_hz = 0")),
        ("unknown key", format!("{good}\n[extra]\nx = 1\n")),
        (
            "dup node",
            good.replace("id = \"node-c\"", "id = \"node-a\""),
        ),
    ] {
        assert!(
            WorldConfig::from_toml_str(&text).is_err(),
            "accepted: {name}"
        );
    }
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

fn trace(id: &str, seed: u64, ticks: u64) -> String {
    let mut w = world(id, seed);
    let mut s = String::new();
    for _ in 0..ticks {
        let o = w.step();
        s.push_str(&serde_json::to_string(&o).unwrap());
        s.push_str(&serde_json::to_string(&w.ground_truth()).unwrap());
    }
    s
}

#[test]
fn identical_seeds_give_identical_worlds() {
    for id in ["s1", "s4", "s6", "s7", "s8"] {
        assert_eq!(trace(id, 42, 450), trace(id, 42, 450), "{id}");
    }
    assert_ne!(trace("s4", 42, 50), trace("s4", 43, 50));
}

#[test]
fn ten_hz_tick_and_time() {
    let mut w = world("s1", 1);
    assert_eq!(w.tick_ms(), 100);
    let o = w.step();
    assert_eq!(w.tick(), 1);
    assert!(o.iter().all(|x| x.tick == 1 && x.sim_time_ms == 100));
    run(&mut w, 9);
    assert_eq!(w.sim_time_ms(), 1_000);
}

#[test]
fn every_node_reports_every_sensor_each_tick() {
    let mut w = world("s1", 3);
    let o = w.step();
    for n in ["node-a", "node-b", "node-c"] {
        for k in [SensorKind::Gps, SensorKind::Imu, SensorKind::Camera] {
            assert_eq!(
                of(&o, n).filter(|x| sensor(x, k).is_some()).count(),
                1,
                "{n} {k}"
            );
        }
        assert!(of(&o, n).any(|x| cpu(x).is_some()));
        assert!(of(&o, n).any(|x| matches!(x.payload, WirePayload::Net { .. })));
    }
    for x in &o {
        x.payload.validate().unwrap();
        assert_eq!(x.delivery, Delivery::Uplink);
    }
}

// ---------------------------------------------------------------------------
// Fault injection (ground truth vs observation)
// ---------------------------------------------------------------------------

#[test]
fn s1_is_quiet() {
    let mut w = world("s1", 11);
    let o = run(&mut w, 600);
    assert!(w.ground_truth().active_faults.is_empty());
    assert!(
        !o.iter()
            .any(|x| matches!(&x.payload, WirePayload::Auth { success: false, .. }))
    );
    assert!(
        !o.iter()
            .any(|x| matches!(x.payload, WirePayload::Process { .. }))
    );
    assert!(w.ground_truth().nodes.iter().all(|n| !n.compromised));
}

fn disagreement(obs: &[Observation], node: &str) -> f64 {
    let gps: Vec<_> = of(obs, node)
        .filter_map(|x| sensor(x, SensorKind::Gps))
        .collect();
    let cam: Vec<_> = of(obs, node)
        .filter_map(|x| sensor(x, SensorKind::Camera))
        .collect();
    gps.iter()
        .zip(&cam)
        .map(|(g, c)| (g[0] - c[0]).hypot(g[1] - c[1]))
        .sum::<f64>()
        / gps.len() as f64
}

#[test]
fn s2_gps_failure_corrupts_readings_but_not_identity() {
    let mut w = world("s2", 5);
    let before = run(&mut w, 299);
    let after = run(&mut w, 200);
    let gt = w.ground_truth();
    assert!(gt.nodes[0].gps_failed && !gt.nodes[0].compromised);
    assert!(disagreement(&before, "node-a") < 10.0);
    assert!(disagreement(&after, "node-a") > 3.0 * disagreement(&before, "node-a"));
    assert!(disagreement(&after, "node-b") < 10.0);
    // Navigation survives on the camera alone: 1 - 4/30.
    let nav = w.capabilities().navigation;
    assert!((nav - (1.0 - 4.0 / 30.0)).abs() < 1e-9, "{nav}");
    assert!(w.mission_ok());
}

#[test]
fn s3_degrades_l1_measurably() {
    let mut w = world("s3", 5);
    run(&mut w, 299);
    let before = w.observe().link("link-l1").unwrap().clone();
    run(&mut w, 5);
    let l1 = w.observe().link("link-l1").unwrap().clone();
    assert!(l1.availability < 0.8 * before.availability);
    assert!(l1.latency_ms > before.latency_ms + 500.0);
    assert!(l1.loss > 0.4);
    assert!(w.capabilities().communication < w.config().mission.min.communication);
}

#[test]
fn s4_compromise_shows_only_through_behaviour() {
    let mut w = world("s4", 9);
    let before = run(&mut w, 299);
    let after = run(&mut w, 30);
    assert!(w.ground_truth().nodes[1].compromised);
    let max_cpu = |obs: &[Observation], n: &str| of(obs, n).filter_map(cpu).fold(0.0, f64::max);
    assert!(max_cpu(&before, "node-b") < 60.0);
    assert!(max_cpu(&after, "node-b") > 80.0);
    assert!(max_cpu(&after, "node-a") < 60.0);
    let b: Vec<_> = of(&after, "node-b").collect();
    assert!(b.iter().any(
        |x| matches!(&x.payload, WirePayload::Net { pkts_per_sec, .. } if *pkts_per_sec > 1000.0)
    ));
    assert!(
        b.iter()
            .filter(|x| matches!(&x.payload, WirePayload::Auth { success: false, .. }))
            .count()
            > 5
    );
    assert_eq!(
        b.iter()
            .filter(|x| matches!(x.payload, WirePayload::Process { started: true, .. }))
            .count(),
        1
    );
    // Camera now disagrees with GPS on B by roughly the attacker's offset.
    assert!(disagreement(&after, "node-b") > 15.0);
    // Observations carry no labels.
    let text = serde_json::to_string(&after).unwrap();
    for leak in [
        "compromis",
        "attacker",
        "\"s4\"",
        "ground",
        "fault",
        "truth",
    ] {
        assert!(!text.contains(leak), "observation leaks {leak:?}");
    }
    // Ground truth: B's relay now breaks communication over L1.
    assert!(w.capabilities().communication < 0.9);
}

#[test]
fn s5_false_telemetry_keeps_identity() {
    let mut w = world("s5", 4);
    run(&mut w, 299);
    let after = run(&mut w, 200);
    let gt = w.ground_truth();
    assert!(gt.nodes[1].false_telemetry && !gt.nodes[1].compromised);
    let cpus: Vec<f64> = of(&after, "node-b").filter_map(cpu).collect();
    let mean = w.config().telemetry.cpu_mean_pct;
    assert!(cpus.iter().all(|c| *c == mean), "flat-lined");
    // Reported position has drifted ~20 s * |(2.5, -1.5)| ≈ 58 m from truth.
    let last_gps = of(&after, "node-b")
        .filter_map(|x| sensor(x, SensorKind::Gps))
        .last()
        .unwrap();
    let truth = w.nodes[1].pos_for_test();
    assert!((last_gps[0] - truth[0]).hypot(last_gps[1] - truth[1]) > 40.0);
    assert!(
        !after
            .iter()
            .any(|x| matches!(&x.payload, WirePayload::Auth { success: false, .. }))
    );
    assert!(
        w.mission_ok(),
        "false reports do not change the true mission"
    );
}

#[test]
fn s6_has_both_faults() {
    let mut w = world("s6", 2);
    run(&mut w, 320);
    let gt = w.ground_truth();
    assert!(gt.nodes[0].gps_failed);
    assert!(gt.nodes[1].compromised);
}

#[test]
fn s7_partitions_c_for_twenty_seconds() {
    let mut w = world("s7", 8);
    let pre = run(&mut w, 299);
    assert!(of(&pre, "node-c").all(|x| x.delivery == Delivery::Uplink));
    let during = run(&mut w, 200); // ticks 300..=499
    assert!(of(&during, "node-c").all(|x| x.delivery == Delivery::Partitioned));
    assert!(of(&during, "node-a").all(|x| x.delivery == Delivery::Uplink));
    assert!(of(&during, "node-b").all(|x| x.delivery == Delivery::Uplink));
    let obs = w.observe();
    assert!(!obs.link("link-l1").unwrap().up && !obs.link("link-l2").unwrap().up);
    assert!(
        during
            .iter()
            .any(|x| matches!(&x.payload, WirePayload::Link { up: false, .. }))
    );
    let post = run(&mut w, 10); // healed at tick 500
    assert!(of(&post, "node-c").all(|x| x.delivery == Delivery::Uplink));
    assert!(w.observe().link("link-l1").unwrap().up);
    assert!(of(&during, "node-c").count() >= 200 * 5);
}

#[test]
fn s8_model_mismatch_shakes_the_dynamics() {
    let speed_var = |id: &str| {
        let mut w = world(id, 21);
        run(&mut w, 300);
        let o = run(&mut w, 200);
        let v: Vec<f64> = of(&o, "node-a")
            .filter_map(|x| sensor(x, SensorKind::Imu))
            .map(|v| v[0])
            .collect();
        let d: Vec<f64> = v.windows(2).map(|p| p[1] - p[0]).collect();
        d.iter().map(|x| x * x).sum::<f64>() / d.len() as f64
    };
    // S1: diff var ≈ 2·0.05² (IMU noise) = 0.005; S8 adds (4 m/s² · 0.1 s)² = 0.16.
    assert!(speed_var("s8") > 10.0 * speed_var("s1"));
}

// ---------------------------------------------------------------------------
// Commands (action effects)
// ---------------------------------------------------------------------------

fn combo() -> A {
    A::Combo(vec![
        A::Isolate("node-b".into()),
        A::Reroute("link-l2".into()),
    ])
}

fn plan(a: &A) -> Vec<CommandOp> {
    commands_for_action(a).unwrap()
}

#[test]
fn isolate_plus_reroute_restores_mission_under_s4() {
    let mut w = world("s4", 9);
    run(&mut w, 305);
    assert!(!w.mission_ok());
    for op in plan(&combo()) {
        assert_eq!(w.apply_command(&op), Ok(ApplyOutcome::Applied));
    }
    let o = w.observe();
    assert_eq!(o.node_mode("node-b"), Some(NodeMode::Isolated));
    assert!(o.link("link-l2").unwrap().active);
    let caps = w.capabilities();
    assert!((caps.communication - 0.93).abs() < 1e-12);
    assert!(w.mission_ok(), "{caps:?}");
    let next = w.step();
    assert!(of(&next, "node-b").all(|x| x.delivery == Delivery::Quarantined));
}

#[test]
fn isolate_alone_severs_communication() {
    let mut w = world("s1", 1);
    w.apply_command(&CommandOp::Isolate {
        node: "node-b".into(),
    })
    .unwrap();
    assert_eq!(w.capabilities().communication, 0.0);
    assert!(of(&w.step(), "node-c").all(|x| x.delivery == Delivery::Partitioned));
}

#[test]
fn commands_are_idempotent_and_validated() {
    let mut w = world("s1", 1);
    let iso = CommandOp::Isolate {
        node: "node-b".into(),
    };
    assert_eq!(w.apply_command(&iso), Ok(ApplyOutcome::Applied));
    assert_eq!(w.apply_command(&iso), Ok(ApplyOutcome::AlreadyApplied));
    assert!(matches!(
        w.apply_command(&CommandOp::Restrict {
            node: "node-b".into()
        }),
        Err(SimError::InvalidTransition(_))
    ));
    assert_eq!(
        w.apply_command(&CommandOp::Recover {
            node: "node-b".into()
        }),
        Ok(ApplyOutcome::Applied)
    );
    assert_eq!(
        w.apply_command(&CommandOp::Isolate {
            node: "node-q".into()
        }),
        Err(SimError::UnknownEntity("node-q".into()))
    );
    assert_eq!(
        w.apply_command(&CommandOp::ActivateLink {
            link: "link-l9".into()
        }),
        Err(SimError::UnknownEntity("link-l9".into()))
    );
    let deg = CommandOp::Degrade {
        capability: Capability::Communication,
    };
    assert_eq!(w.apply_command(&deg), Ok(ApplyOutcome::Applied));
    assert_eq!(w.apply_command(&deg), Ok(ApplyOutcome::AlreadyApplied));
    assert!((w.effective_minima().communication - 0.25).abs() < 1e-12);
    assert_eq!(w.observe().degraded, vec![Capability::Communication]);
    assert_eq!(
        w.apply_command(&CommandOp::Observe),
        Ok(ApplyOutcome::AlreadyApplied)
    );
}

#[test]
fn failover_moves_relay_role() {
    let mut w = world("s1", 1);
    let fo = CommandOp::Failover {
        from: "node-b".into(),
        to: "node-a".into(),
    };
    assert_eq!(w.apply_command(&fo), Ok(ApplyOutcome::Applied));
    let o = w.observe();
    assert_eq!(o.node_mode("node-b"), Some(NodeMode::Standby));
    assert!(!o.link("link-l1").unwrap().active);
    assert!(o.link("link-l2").unwrap().active);
    assert!(w.mission_ok());
    let mut w = world("s1", 1);
    w.apply_command(&CommandOp::Isolate {
        node: "node-a".into(),
    })
    .unwrap();
    assert!(matches!(
        w.apply_command(&fo),
        Err(SimError::InvalidTransition(_))
    ));
}

#[test]
fn restrict_stops_beaconing_but_not_the_attacker() {
    let mut w = world("s4", 9);
    run(&mut w, 300);
    w.apply_command(&CommandOp::Restrict {
        node: "node-b".into(),
    })
    .unwrap();
    let o = run(&mut w, 20);
    assert!(!of(&o, "node-b").any(
        |x| matches!(&x.payload, WirePayload::Net { pkts_per_sec, .. } if *pkts_per_sec > 1000.0)
    ));
    assert!(w.ground_truth().nodes[1].compromised);
    assert!(w.capabilities().communication < 0.9);
}

#[test]
fn actuator_faults_ack_without_effect_or_refuse() {
    let c = cfg();
    let text = "[scenario]\nid = \"act\"\nname = \"n\"\ndescription = \"d\"\nduration_s = 60.0\n\
        [[faults]]\nkind = \"actuator_fault\"\ntarget = \"link-l2\"\nbehavior = \"ack_no_effect\"\nstart_s = 0.0\n\
        [[faults]]\nkind = \"actuator_fault\"\ntarget = \"node-c\"\nbehavior = \"refuse\"\nstart_s = 0.0\n";
    let s = Arc::new(Scenario::from_toml_str(text, &c).unwrap());
    let mut w = World::new(c, s, 1).unwrap();
    w.step();
    assert_eq!(
        w.apply_command(&CommandOp::ActivateLink {
            link: "link-l2".into()
        }),
        Ok(ApplyOutcome::Applied)
    );
    assert!(
        !w.observe().link("link-l2").unwrap().active,
        "acked but not applied"
    );
    assert_eq!(
        w.apply_command(&CommandOp::Isolate {
            node: "node-c".into()
        }),
        Err(SimError::Refused("node-c".into()))
    );
}

// ---------------------------------------------------------------------------
// Fork, rollouts, isolation, reproducibility
// ---------------------------------------------------------------------------

fn belief_b(p: f64) -> Belief {
    Belief {
        nodes: vec![NodeBelief {
            node: "node-b".into(),
            p_compromised: p,
            p_gps_failed: 0.0,
        }],
    }
}

fn small_spec(seed: u64) -> RolloutSpec {
    RolloutSpec {
        n: 120,
        horizon_ticks: 100,
        verify_tick: 30,
        seed,
    }
}

fn base() -> World {
    World::from_observation(cfg(), &world("s1", 1).observe(), [3; 32]).unwrap()
}

#[test]
fn rollouts_are_reproducible() {
    let b = base();
    let x = run_rollouts(&b, &belief_b(0.9), &plan(&combo()), &small_spec(77)).unwrap();
    let y = run_rollouts(&b, &belief_b(0.9), &plan(&combo()), &small_spec(77)).unwrap();
    assert_eq!(x, y);
    let c = run_rollouts(&b, &belief_b(0.9), &plan(&A::Monitor), &small_spec(78)).unwrap();
    let d = run_rollouts(&b, &belief_b(0.9), &plan(&A::Monitor), &small_spec(77)).unwrap();
    assert_ne!(c, d, "different seeds give different samples");
}

#[test]
fn rollouts_never_touch_the_live_world() {
    let mut live = world("s4", 5);
    let mut twin = world("s4", 5);
    for t in 0..400u64 {
        if t.is_multiple_of(50) {
            let b = World::from_observation(cfg(), &live.observe(), [t as u8; 32]).unwrap();
            for a in [A::Monitor, combo(), A::Reroute("link-l2".into())] {
                run_rollouts(&b, &belief_b(0.95), &plan(&a), &small_spec(t)).unwrap();
            }
            // Forking the live world itself must not disturb it either.
            let mut f = live.fork([9; 32]);
            f.apply_command(&CommandOp::Isolate {
                node: "node-b".into(),
            })
            .unwrap();
            run(&mut f, 3);
        }
        assert_eq!(live.step(), twin.step(), "diverged at tick {t}");
    }
    assert_eq!(live.ground_truth(), twin.ground_truth());
}

#[test]
fn candidates_are_isolated_and_share_random_numbers() {
    let b = base();
    let spec = small_spec(5);
    let belief = belief_b(0.5);
    let plans: Vec<_> = [A::Monitor, combo(), A::Reroute("link-l2".into())]
        .iter()
        .map(plan)
        .collect();
    let together = rollout::evaluate_plans(&b, &belief, &plans, &spec);
    // A candidate evaluated alone equals the same candidate in a batch.
    for (p, r) in plans.iter().zip(&together) {
        assert_eq!(
            r.as_ref().unwrap(),
            &run_rollouts(&b, &belief, p, &spec).unwrap()
        );
    }
    // Common random numbers: rollout n samples the same hidden state for
    // every candidate.
    let m = together[0].as_ref().unwrap();
    let c = together[1].as_ref().unwrap();
    for (x, y) in m.outcomes.iter().zip(&c.outcomes) {
        assert_eq!(x.sampled_compromised, y.sampled_compromised);
    }
    let k = m
        .outcomes
        .iter()
        .filter(|o| !o.sampled_compromised.is_empty())
        .count();
    assert!(k > 30 && k < 90, "~50% sampled compromised, got {k}/120");
}

#[test]
fn rollout_base_from_observation_carries_no_ground_truth() {
    let mut live = world("s4", 5);
    run(&mut live, 400);
    assert!(live.ground_truth().nodes[1].compromised);
    let b = World::from_observation(cfg(), &live.observe(), [0; 32]).unwrap();
    assert!(
        b.ground_truth()
            .nodes
            .iter()
            .all(|n| !n.compromised && !n.gps_failed)
    );
    // Believed healthy, monitoring succeeds in the reconstructed world even
    // though the live B is compromised: rollouts only know the belief.
    let r = run_rollouts(&b, &belief_b(0.0), &plan(&A::Monitor), &small_spec(1)).unwrap();
    assert!(r.p_success() > 0.8);
}

#[test]
fn outcomes_respond_to_belief_not_to_a_script() {
    let b = base();
    let spec = small_spec(11);
    let p = |belief: f64, a: &A| {
        run_rollouts(&b, &belief_b(belief), &plan(a), &spec)
            .unwrap()
            .p_success()
    };
    assert!(
        p(0.0, &A::Monitor) > 0.8,
        "believed healthy: no action needed"
    );
    assert!(p(0.95, &A::Monitor) < 0.2);
    assert!(p(0.95, &combo()) > 0.8);
    assert_eq!(p(0.95, &A::Isolate("node-b".into())), 0.0);
    assert!(p(0.95, &A::Reroute("link-l2".into())) < p(0.95, &combo()));
}

#[test]
fn rollout_outcomes_are_complete() {
    let b = base();
    let r = run_rollouts(&b, &belief_b(1.0), &plan(&A::Monitor), &small_spec(2)).unwrap();
    assert_eq!(r.outcomes.len(), 120);
    assert_eq!(r.successes().len(), 120);
    for o in &r.outcomes {
        assert_eq!(o.sampled_compromised, ["node-b"]);
        assert!(!o.success);
        assert_eq!(o.mission_loss, 1.0);
        assert_eq!(o.first_failure, Some((1, Capability::Communication)));
        assert!(o.min_capabilities.communication <= o.final_capabilities.communication);
        assert_eq!(o.final_modes.len(), 3);
        assert_eq!(o.final_active_links, ["link-l1"]);
    }
    let c = run_rollouts(&b, &belief_b(1.0), &plan(&combo()), &small_spec(2)).unwrap();
    assert!((c.median_verify_capabilities().communication - 0.93).abs() < 1e-12);
    assert!(
        c.outcomes
            .iter()
            .all(|o| o.final_active_links.contains(&"link-l2".to_string()))
    );
}

#[test]
fn invalid_rollout_requests_fail_loudly() {
    let b = base();
    let bad_plan = vec![CommandOp::ActivateLink {
        link: "link-l9".into(),
    }];
    assert!(run_rollouts(&b, &Belief::default(), &bad_plan, &small_spec(1)).is_err());
    assert!(run_rollouts(&b, &belief_b(1.5), &[], &small_spec(1)).is_err());
    assert!(run_rollouts(&b, &belief_b(f64::NAN), &[], &small_spec(1)).is_err());
    let mut spec = small_spec(1);
    spec.verify_tick = 101;
    assert!(run_rollouts(&b, &Belief::default(), &[], &spec).is_err());
    spec = small_spec(1);
    spec.n = 0;
    assert!(run_rollouts(&b, &Belief::default(), &[], &spec).is_err());
    let unknown = Belief {
        nodes: vec![NodeBelief {
            node: "node-q".into(),
            p_compromised: 0.5,
            p_gps_failed: 0.0,
        }],
    };
    assert!(run_rollouts(&b, &unknown, &[], &small_spec(1)).is_err());
}

#[test]
fn expected_topology_and_observed_capabilities() {
    let c = cfg();
    let obs = world("s1", 1).observe();
    let exp = expected_topology(&c, &obs, &plan(&combo())).unwrap();
    assert_eq!(exp.node_mode("node-b"), Some(NodeMode::Isolated));
    assert!(exp.link("link-l2").unwrap().active);
    let caps = capabilities_from_observation(&c, &exp).unwrap();
    assert!(caps.meets(&c.mission.min), "{caps:?}");
    let mut bad = exp.clone();
    for l in &mut bad.links {
        if l.link == "link-l2" {
            l.availability = 0.5;
        }
    }
    assert!(
        capabilities_from_observation(&c, &bad)
            .unwrap()
            .communication
            < 0.9
    );
}

#[test]
fn capability_estimate_uses_interior_trust_only() {
    let c = cfg();
    let o = world("s1", 1).observe();
    let full = estimate_capabilities(&c, &o, &[]).unwrap();
    assert!((full.communication - 0.99 * 0.99).abs() < 1e-12);
    // §6.6 worked example: T_B = 0.565 -> c_comm = 0.99 * 0.99 * 0.565.
    let t = vec![("node-b".to_string(), 0.565), ("node-a".to_string(), 0.1)];
    let low = estimate_capabilities(&c, &o, &t).unwrap();
    assert!((low.communication - 0.99 * 0.99 * 0.565).abs() < 1e-12);
    // Endpoint trust does not enter c_comm (interior nodes only).
    let t = vec![("node-a".to_string(), 0.1)];
    assert_eq!(
        estimate_capabilities(&c, &o, &t).unwrap().communication,
        full.communication
    );
}
