//! AERIS Console CLI / Standalone Runner.
//!
//! Provides headless, fixture playback, and dashboard generation.
//! Defined in docs/v0.1/07-schedule.md and docs/v0.1/09-demo-and-run.md.
//! Owner: KISHANTH

use aeris_console::{render_html_dashboard, render_terminal_summary};
use aeris_core::{CoreState, step};
use aeris_types::{EventPayload, VerifiedEvent};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    println!("=== AERIS Autonomous Mission Console v0.1.0 ===");

    let mut state = CoreState::new();

    // Run nominal tick
    let cycle0 = step(&mut state, &[], 42);
    println!("{}", render_terminal_summary(&cycle0));

    // Simulate scenario S4 injection
    println!("\n>>> INJECTING SCENARIO S4 (Node B Compromise) <<<\n");
    let attack_events = vec![
        VerifiedEvent {
            id: "evt-01".into(),
            source: "node-b".into(),
            seq: 1,
            hlc: Default::default(),
            t_unix_ms: 30100,
            replayed: false,
            payload: EventPayload::Net {
                pkts_per_sec: 2500.0,
                bytes_per_sec: 500000.0,
                loss: 0.01,
                latency_ms: 12.0,
            },
            signature_valid: true,
        },
        VerifiedEvent {
            id: "evt-02".into(),
            source: "node-b".into(),
            seq: 2,
            hlc: Default::default(),
            t_unix_ms: 30200,
            replayed: false,
            payload: EventPayload::Auth {
                service: "sshd".into(),
                success: false,
                principal: "root".into(),
            },
            signature_valid: true,
        },
        VerifiedEvent {
            id: "evt-03".into(),
            source: "node-b".into(),
            seq: 3,
            hlc: Default::default(),
            t_unix_ms: 30300,
            replayed: false,
            payload: EventPayload::Telemetry {
                cpu: 98.0,
                mem: 75.0,
                proc_count: 140,
            },
            signature_valid: true,
        },
    ];

    let cycle_attack = step(&mut state, &attack_events, 42);
    println!("{}", render_terminal_summary(&cycle_attack));

    if let Some(ref prop) = cycle_attack.proposal {
        println!(
            "\n>>> COMMANDER APPROVES: {} <<<\n",
            prop.recommended_action.display_name()
        );
        state.apply_action(&prop.recommended_action);

        let cycle_verified = step(&mut state, &[], 43);
        println!("{}", render_terminal_summary(&cycle_verified));
    }

    if args.iter().any(|a| a == "--html") {
        let html = render_html_dashboard(&cycle_attack);
        println!("\nHTML Dashboard Output ({} bytes)", html.len());
    }
}
