use super::*;
use aeris_types::Hlc;
use proptest::prelude::*;

fn uuid7(ms: u64, r: u8) -> String {
    uuid::Builder::from_unix_timestamp_millis(ms, &[r; 10])
        .into_uuid()
        .hyphenated()
        .to_string()
}

fn sample(payload: WirePayload) -> WireEvent {
    WireEvent {
        id: uuid7(1_798_761_600_000, 7),
        source: "node-b".into(),
        seq: 42,
        hlc: Hlc {
            l: 1_798_761_600_000,
            c: 3,
        },
        t_unix_ms: 1_798_761_600_000,
        replayed: false,
        confidence: 0.95,
        provenance: Provenance {
            producer: "aeris-sim/node-b".into(),
            producer_version: "0.1.0".into(),
        },
        payload,
    }
}

fn all_payloads() -> Vec<WirePayload> {
    vec![
        WirePayload::Telemetry {
            cpu_pct: 42.5,
            mem_pct: 36.0,
            proc_count: 15,
        },
        WirePayload::Net {
            pkts_per_sec: 1450.0,
            bytes_per_sec: 250_000.0,
            peers: 3,
            latency_ms: 12.0,
            loss: 0.01,
        },
        WirePayload::Auth {
            service: "sshd".into(),
            success: false,
            principal: "root".into(),
        },
        WirePayload::Process {
            name: "kworker-x".into(),
            started: true,
            hash: "deadbeef".into(),
        },
        WirePayload::Sensor {
            kind: SensorKind::Gps,
            value: vec![120.0, -3.5],
            sigma: 2.5,
        },
        WirePayload::Sensor {
            kind: SensorKind::Imu,
            value: vec![5.0, 0.0],
            sigma: 0.05,
        },
        WirePayload::Sensor {
            kind: SensorKind::Camera,
            value: vec![1.0, 2.0],
            sigma: 4.0,
        },
        WirePayload::Link {
            link_id: "link-l2".into(),
            up: true,
            availability: 0.93,
        },
    ]
}

#[test]
fn every_payload_round_trips_losslessly() {
    for p in all_payloads() {
        let ev = sample(p);
        let bytes = encode_body(&ev).unwrap();
        let back = decode_body(&bytes).unwrap();
        assert_eq!(back, ev);
        // Re-encoding a decoded struct yields identical bytes for prost, but
        // receivers must not rely on it; signatures are checked on the
        // received bytes. This just pins prost's behaviour for our schema.
        assert_eq!(encode_body(&back).unwrap(), bytes);
    }
}

#[test]
fn domain_projection_matches_aeris_types() {
    for p in all_payloads() {
        let ev = sample(p.clone());
        let v = ev.to_verified_event(true);
        assert_eq!(v.id, ev.id);
        assert_eq!(v.source, ev.source);
        assert_eq!(v.seq, ev.seq);
        assert_eq!(v.hlc, ev.hlc);
        assert_eq!(v.t_unix_ms, ev.t_unix_ms);
        assert!(v.signature_valid);
        // Domain -> wire -> domain is the identity on the domain type.
        let back = WirePayload::from_domain(&v.payload).unwrap();
        assert_eq!(back.to_domain(), v.payload);
        // The only lossy field is Net.peers.
        match (&p, &back) {
            (WirePayload::Net { .. }, WirePayload::Net { peers, .. }) => assert_eq!(*peers, 0),
            _ => assert_eq!(back, p),
        }
    }
}

#[test]
fn sensor_kind_names_match_core_expectations() {
    // aeris-core builds sensor entity ids as "<source>/<kind>".
    let v = sample(WirePayload::Sensor {
        kind: SensorKind::Gps,
        value: vec![1.0, 2.0],
        sigma: 2.5,
    })
    .to_verified_event(true);
    match v.payload {
        aeris_types::EventPayload::Sensor { kind, .. } => assert_eq!(kind, "gps"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn rejects_invalid_field_values() {
    let base = sample(all_payloads()[0].clone());
    let cases: Vec<(&str, WireEvent)> = vec![
        (
            "seq 0",
            WireEvent {
                seq: 0,
                ..base.clone()
            },
        ),
        (
            "negative time",
            WireEvent {
                t_unix_ms: -1,
                ..base.clone()
            },
        ),
        (
            "confidence > 1",
            WireEvent {
                confidence: 1.5,
                ..base.clone()
            },
        ),
        (
            "confidence NaN",
            WireEvent {
                confidence: f64::NAN,
                ..base.clone()
            },
        ),
        (
            "bad source",
            WireEvent {
                source: "Node.B".into(),
                ..base.clone()
            },
        ),
        (
            "subject wildcard in source",
            WireEvent {
                source: "node-*".into(),
                ..base.clone()
            },
        ),
        (
            "too many source parts",
            WireEvent {
                source: "node-b/gps/x".into(),
                ..base.clone()
            },
        ),
        (
            "uuid v4",
            WireEvent {
                id: "4c1f6f3a-2b1e-4d7a-9a3e-0e1c2d3b4a59".into(),
                ..base.clone()
            },
        ),
        (
            "uppercase uuid",
            WireEvent {
                id: base.id.to_uppercase(),
                ..base.clone()
            },
        ),
        (
            "braced uuid",
            WireEvent {
                id: format!("{{{}}}", base.id),
                ..base.clone()
            },
        ),
        (
            "cpu > 100",
            sample(WirePayload::Telemetry {
                cpu_pct: 101.0,
                mem_pct: 1.0,
                proc_count: 1,
            }),
        ),
        (
            "infinite pkts",
            sample(WirePayload::Net {
                pkts_per_sec: f64::INFINITY,
                bytes_per_sec: 0.0,
                peers: 0,
                latency_ms: 0.0,
                loss: 0.0,
            }),
        ),
        (
            "loss > 1",
            sample(WirePayload::Net {
                pkts_per_sec: 0.0,
                bytes_per_sec: 0.0,
                peers: 0,
                latency_ms: 0.0,
                loss: 2.0,
            }),
        ),
        (
            "empty sensor",
            sample(WirePayload::Sensor {
                kind: SensorKind::Gps,
                value: vec![],
                sigma: 1.0,
            }),
        ),
        (
            "zero sigma",
            sample(WirePayload::Sensor {
                kind: SensorKind::Gps,
                value: vec![1.0],
                sigma: 0.0,
            }),
        ),
        (
            "too many sensor values",
            sample(WirePayload::Sensor {
                kind: SensorKind::Gps,
                value: vec![0.0; MAX_SENSOR_VALUES + 1],
                sigma: 1.0,
            }),
        ),
        (
            "control char in principal (log injection)",
            sample(WirePayload::Auth {
                service: "sshd".into(),
                success: false,
                principal: "root\nFAKE LOG LINE".into(),
            }),
        ),
        (
            "oversized service",
            sample(WirePayload::Auth {
                service: "x".repeat(MAX_TEXT_LEN + 1),
                success: true,
                principal: String::new(),
            }),
        ),
        (
            "non-hex hash",
            sample(WirePayload::Process {
                name: "p".into(),
                started: true,
                hash: "zz".into(),
            }),
        ),
        (
            "bad link id",
            sample(WirePayload::Link {
                link_id: "link.l2".into(),
                up: true,
                availability: 0.5,
            }),
        ),
    ];
    for (name, ev) in cases {
        assert!(encode_body(&ev).is_err(), "encode accepted: {name}");
        // The decoder must reject it too, even if a hostile encoder sent it.
        let raw = ev.to_pb().encode_to_vec();
        assert!(decode_body(&raw).is_err(), "decode accepted: {name}");
    }
}

#[test]
fn rejects_missing_required_fields() {
    let good = sample(all_payloads()[0].clone()).to_pb();
    let mut no_hlc = good.clone();
    no_hlc.hlc = None;
    assert_eq!(
        decode_body(&no_hlc.encode_to_vec()),
        Err(WireError::Missing("hlc"))
    );
    let mut no_payload = good.clone();
    no_payload.payload = None;
    assert_eq!(
        decode_body(&no_payload.encode_to_vec()),
        Err(WireError::Missing("payload"))
    );
    let mut no_conf = good.clone();
    no_conf.confidence = None;
    assert_eq!(
        decode_body(&no_conf.encode_to_vec()),
        Err(WireError::Missing("confidence"))
    );
    let mut no_prov = good;
    no_prov.provenance = None;
    assert_eq!(
        decode_body(&no_prov.encode_to_vec()),
        Err(WireError::Missing("provenance"))
    );
    assert_eq!(decode_body(&[]), Err(WireError::Missing("hlc")));
}

#[test]
fn unknown_sensor_enum_is_rejected_not_defaulted() {
    let mut pbody = sample(all_payloads()[4].clone()).to_pb();
    if let Some(pb::body::Payload::Sensor(s)) = pbody.payload.as_mut() {
        s.kind = 99;
    }
    assert_eq!(
        decode_body(&pbody.encode_to_vec()),
        Err(WireError::UnknownEnum {
            field: "sensor.kind",
            value: 99
        })
    );
    if let Some(pb::body::Payload::Sensor(s)) = pbody.payload.as_mut() {
        s.kind = 0;
    }
    assert_eq!(
        decode_body(&pbody.encode_to_vec()),
        Err(WireError::Missing("sensor.kind"))
    );
}

#[test]
fn unknown_payload_variant_from_newer_schema_is_rejected() {
    // A future producer adds oneof field 20. Old decoders skip the unknown
    // field, so the payload is absent: explicit error, not a silent default.
    let mut bytes = sample(all_payloads()[0].clone()).to_pb();
    bytes.payload = None;
    let mut raw = bytes.encode_to_vec();
    // field 20, wire type 2 (len-delimited), length 0
    raw.extend_from_slice(&[0xA2, 0x01, 0x00]);
    assert_eq!(decode_body(&raw), Err(WireError::Missing("payload")));
}

#[test]
fn unknown_fields_are_ignored_for_forward_compatibility() {
    let ev = sample(all_payloads()[0].clone());
    let mut raw = encode_body(&ev).unwrap();
    // field 99 varint = 1: an additive field from a newer schema.
    raw.extend_from_slice(&[0x98, 0x06, 0x01]);
    assert_eq!(decode_body(&raw).unwrap(), ev);
}

#[test]
fn malformed_and_truncated_bytes_are_rejected() {
    let ev = sample(all_payloads()[1].clone());
    let raw = encode_body(&ev).unwrap();
    for cut in 1..raw.len() {
        // Every strict prefix must fail cleanly (no panic); most are
        // Malformed, some decode to a body missing required fields.
        assert!(decode_body(&raw[..cut]).is_err(), "prefix {cut} accepted");
    }
    assert!(matches!(
        decode_body(&[
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01
        ]),
        Err(WireError::Malformed(_))
    ));
}

#[test]
fn envelope_limits_and_signature_length() {
    let body = encode_body(&sample(all_payloads()[0].clone())).unwrap();
    let env = encode_envelope(&body, &[7u8; 64]);
    let raw = decode_envelope(&env).unwrap();
    assert_eq!(raw.body, body, "body bytes must be preserved exactly");
    assert_eq!(raw.signature, [7u8; 64]);

    let short_sig = pb::Envelope {
        body: body.clone(),
        signature: vec![1; 63],
    }
    .encode_to_vec();
    assert!(matches!(
        decode_envelope(&short_sig),
        Err(WireError::Invalid {
            field: "signature",
            ..
        })
    ));
    let empty_body = pb::Envelope {
        body: vec![],
        signature: vec![1; 64],
    }
    .encode_to_vec();
    assert_eq!(
        decode_envelope(&empty_body),
        Err(WireError::Missing("body"))
    );
    let huge = vec![0u8; MAX_ENVELOPE_BYTES + 1];
    assert!(matches!(
        decode_envelope(&huge),
        Err(WireError::TooLarge { .. })
    ));
}

fn cmd(op: CommandOp) -> Command {
    Command {
        id: uuid7(1_798_761_600_000, 9),
        decision_id: "dec-01".into(),
        issuer: "server".into(),
        seq: 1,
        t_unix_ms: 1_798_761_600_000,
        op,
    }
}

#[test]
fn commands_round_trip_and_route_to_target_subject() {
    let ops = vec![
        (
            CommandOp::Isolate {
                node: "node-b".into(),
            },
            "aeris.cmd.node-b",
        ),
        (
            CommandOp::Restrict {
                node: "node-b".into(),
            },
            "aeris.cmd.node-b",
        ),
        (
            CommandOp::Recover {
                node: "node-b".into(),
            },
            "aeris.cmd.node-b",
        ),
        (
            CommandOp::ActivateLink {
                link: "link-l2".into(),
            },
            "aeris.cmd.link-l2",
        ),
        (
            CommandOp::Failover {
                from: "node-b".into(),
                to: "node-a".into(),
            },
            "aeris.cmd.node-b",
        ),
        (
            CommandOp::Degrade {
                capability: aeris_types::Capability::Communication,
            },
            "aeris.cmd.mission",
        ),
        (CommandOp::Observe, "aeris.cmd.world"),
    ];
    for (op, subject) in ops {
        let c = cmd(op);
        let bytes = encode_command(&c).unwrap();
        assert_eq!(decode_command(&bytes).unwrap(), c);
        assert_eq!(c.subject(), subject);
    }
}

#[test]
fn invalid_commands_are_rejected() {
    let mut c = cmd(CommandOp::Isolate {
        node: "node-b".into(),
    });
    c.decision_id.clear();
    assert!(
        encode_command(&c).is_err(),
        "mutating command needs decision"
    );
    let c = cmd(CommandOp::Failover {
        from: "node-b".into(),
        to: "node-b".into(),
    });
    assert!(encode_command(&c).is_err());
    let mut raw = pb::CommandBody {
        id: uuid7(1, 1),
        decision_id: "d".into(),
        issuer: "server".into(),
        seq: 1,
        t_unix_ms: 1,
        op: Some(pb::command_body::Op::Degrade(pb::CapabilityTarget {
            capability: 42,
        })),
    };
    assert!(matches!(
        decode_command(&raw.encode_to_vec()),
        Err(WireError::UnknownEnum { .. })
    ));
    raw.op = None;
    assert_eq!(
        decode_command(&raw.encode_to_vec()),
        Err(WireError::Missing("op"))
    );
}

#[test]
fn acks_round_trip_with_observation() {
    let ack = CommandAck {
        command_id: uuid7(5, 5),
        status: AckStatus::Applied,
        reason: String::new(),
        observation: Some(WorldObservation {
            tick: 310,
            t_unix_ms: 31_000,
            nodes: vec![
                ("node-a".into(), NodeMode::Active),
                ("node-b".into(), NodeMode::Isolated),
            ],
            links: vec![LinkObservation {
                link: "link-l2".into(),
                active: true,
                up: true,
                availability: 0.93,
                latency_ms: 25.0,
                loss: 0.02,
            }],
            degraded: vec![aeris_types::Capability::Communication],
        }),
        t_unix_ms: 31_000,
    };
    let bytes = encode_ack(&ack).unwrap();
    let back = decode_ack(&bytes).unwrap();
    assert_eq!(back, ack);
    let obs = back.observation.unwrap();
    assert_eq!(obs.node_mode("node-b"), Some(NodeMode::Isolated));
    assert!(obs.link("link-l2").unwrap().active);

    let bad = pb::CommandAck {
        command_id: "x".into(),
        status: 77,
        ..Default::default()
    };
    assert!(matches!(
        decode_ack(&bad.encode_to_vec()),
        Err(WireError::UnknownEnum { .. })
    ));
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = decode_envelope(&bytes);
        let _ = decode_body(&bytes);
        let _ = decode_command(&bytes);
        let _ = decode_ack(&bytes);
    }

    #[test]
    fn valid_telemetry_round_trips(
        cpu in 0.0f64..=100.0,
        mem in 0.0f64..=100.0,
        procs in any::<u32>(),
        seq in 1u64..,
        c in any::<u32>(),
        conf in 0.0f64..=1.0,
        replayed in any::<bool>(),
    ) {
        let mut ev = sample(WirePayload::Telemetry { cpu_pct: cpu, mem_pct: mem, proc_count: procs });
        ev.seq = seq;
        ev.hlc.c = c;
        ev.confidence = conf;
        ev.replayed = replayed;
        let bytes = encode_body(&ev).unwrap();
        prop_assert_eq!(decode_body(&bytes).unwrap(), ev);
    }
}

#[test]
fn gamma_orders_path_activation_before_isolation() {
    use aeris_types::Action as A;
    let combo = A::Combo(vec![
        A::Isolate("node-b".into()),
        A::Reroute("link-l2".into()),
    ]);
    assert_eq!(
        commands_for_action(&combo).unwrap(),
        vec![
            CommandOp::ActivateLink {
                link: "link-l2".into()
            },
            CommandOp::Isolate {
                node: "node-b".into()
            },
        ]
    );
    assert!(commands_for_action(&A::Monitor).unwrap().is_empty());
    assert_eq!(
        commands_for_action(&A::Degrade(aeris_types::Capability::Observation)).unwrap(),
        vec![CommandOp::Degrade {
            capability: aeris_types::Capability::Observation
        }]
    );
    assert_eq!(
        commands_for_action(&A::Combo(vec![A::Combo(vec![A::Monitor])])),
        Err(PlanError::NestedCombo)
    );
    assert_eq!(
        commands_for_action(&A::Combo(vec![])),
        Err(PlanError::EmptyCombo)
    );
    assert_eq!(
        commands_for_action(&A::Combo(vec![
            A::Isolate("node-b".into()),
            A::Recover("node-b".into())
        ])),
        Err(PlanError::Conflict("node-b".into()))
    );
    // Duplicates collapse; Monitor inside a combo is a no-op.
    assert_eq!(
        commands_for_action(&A::Combo(vec![
            A::Monitor,
            A::Isolate("node-b".into()),
            A::Isolate("node-b".into())
        ]))
        .unwrap()
        .len(),
        1
    );
    assert!(matches!(
        commands_for_action(&A::Isolate("Node B".into())),
        Err(PlanError::Invalid(_))
    ));
}
