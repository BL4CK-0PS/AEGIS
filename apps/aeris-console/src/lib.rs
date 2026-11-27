//! AERIS Operator Console UI and Renderer.
//!
//! Generates the operator mission dashboard, SVG operational graph,
//! incident and evidence panel, candidate options table, and verification panel.
//!
//! Defined in docs/v0.1/02-team-stack-layout.md and docs/v0.1/07-schedule.md.
//! Owner: KISHANTH

use aeris_types::{Capability, CycleOutput, Gate, Mode, TrustLabel};

/// Renders the complete HTML/SVG console view from a CycleOutput.
pub fn render_html_dashboard(cycle: &CycleOutput) -> String {
    let mode_color = match cycle.mode {
        Mode::Normal => "#22c55e",
        Mode::Degraded => "#eab308",
        Mode::Containment => "#f97316",
        Mode::HumanReview => "#ef4444",
        Mode::Recovery => "#3b82f6",
        Mode::Failsafe => "#9333ea",
    };

    let svg_graph = render_svg_graph(cycle);

    let mut capabilities_html = String::new();
    for (cap, score) in &cycle.capabilities {
        let (name, min) = match cap {
            Capability::Navigation => ("Navigation", 0.85),
            Capability::Communication => ("Communication", 0.90),
            Capability::Observation => ("Observation", 0.80),
        };
        let status_color = if *score >= min { "#22c55e" } else { "#ef4444" };
        let pct = (score * 100.0).clamp(0.0, 100.0);
        let min_pct = min * 100.0;

        capabilities_html.push_str(&format!(
            r#"<div style="margin-bottom: 12px;">
                <div style="display: flex; justify-content: space-between; font-size: 13px; font-weight: 500;">
                    <span>{}</span>
                    <span>{:.3} (min {:.2})</span>
                </div>
                <div style="background: #334155; border-radius: 4px; height: 8px; width: 100%; margin-top: 4px; position: relative;">
                    <div style="background: {}; height: 100%; border-radius: 4px; width: {:.1}%;"></div>
                    <div style="position: absolute; left: {:.1}%; top: -2px; bottom: -2px; width: 2px; background: #ffffff;"></div>
                </div>
            </div>"#,
            name, score, min, status_color, pct, min_pct
        ));
    }

    let mut incidents_html = String::new();
    if cycle.incidents.is_empty() {
        incidents_html.push_str(r#"<div style="color: #94a3b8; font-size: 14px;">No active incidents. System nominal.</div>"#);
    } else {
        for inc in &cycle.incidents {
            let ev_list = inc.evidence_ids.join(", ");
            incidents_html.push_str(&format!(
                r#"<div style="background: #1e293b; border-left: 4px solid #ef4444; padding: 12px; margin-bottom: 8px; border-radius: 4px;">
                    <div style="display: flex; justify-content: space-between; font-weight: bold; font-size: 14px;">
                        <span>Incident #{}: Target {}</span>
                        <span style="color: #ef4444;">Risk: {:.3}</span>
                    </div>
                    <div style="font-size: 12px; color: #94a3b8; margin-top: 4px;">
                        Status: {:?} | Opened at tick: {}
                    </div>
                    <div style="font-size: 12px; margin-top: 6px;">
                        Posterior: Compromise <strong>{:.1}%</strong>, Normal {:.1}%, Sensor Failure {:.1}%
                    </div>
                    <div style="font-size: 12px; color: #38bdf8; margin-top: 4px;">
                        Cited Evidence: [{}]
                    </div>
                </div>"#,
                inc.id, inc.entity, inc.risk, inc.status, inc.opened_at_tick,
                inc.posterior.compromise * 100.0, inc.posterior.normal * 100.0, inc.posterior.sensor_failure * 100.0,
                ev_list
            ));
        }
    }

    let mut decision_html = String::new();
    if let Some(ref prop) = cycle.proposal {
        let gate_badge = match &prop.gate {
            Gate::Auto => {
                r#"<span style="background: #166534; color: #4ade80; padding: 3px 8px; border-radius: 4px; font-size: 12px;">AUTO</span>"#
            }
            Gate::HumanReview { reason } => &format!(
                r#"<span style="background: #991b1b; color: #fca5a5; padding: 3px 8px; border-radius: 4px; font-size: 12px;" title="{}">HUMAN_REVIEW</span>"#,
                reason
            ),
            Gate::Reject { invariant } => &format!(
                r#"<span style="background: #7f1d1d; color: #f87171; padding: 3px 8px; border-radius: 4px; font-size: 12px;">REJECT ({:?})</span>"#,
                invariant
            ),
        };

        let mut rows = String::new();
        for cand in &prop.candidates {
            let is_recommended = cand.action == prop.recommended_action;
            let is_violated = !cand.violations.is_empty();

            let row_style = if is_recommended {
                "background: rgba(34, 197, 94, 0.15); font-weight: 600;"
            } else if is_violated {
                "color: #64748b; text-decoration: line-through;"
            } else {
                ""
            };

            let status_note = if is_violated {
                format!("Violates {:?}", cand.violations[0])
            } else if is_recommended {
                "RECOMMENDED".to_string()
            } else {
                "Feasible".to_string()
            };

            rows.push_str(&format!(
                r#"<tr style="border-bottom: 1px solid #334155; {}">
                    <td style="padding: 8px;">{}</td>
                    <td style="padding: 8px; text-align: right;">{:.2}</td>
                    <td style="padding: 8px; text-align: right;">{:.2}</td>
                    <td style="padding: 8px; text-align: right;">{:.3}</td>
                    <td style="padding: 8px; text-align: right;">{:.3}</td>
                    <td style="padding: 8px; text-align: right; color: {};">{:.3}</td>
                    <td style="padding: 8px; font-size: 12px;">{}</td>
                </tr>"#,
                row_style,
                cand.action.display_name(),
                cand.p_success,
                cand.disruption_cost,
                cand.exp_utility,
                cand.cvar,
                if is_recommended { "#4ade80" } else { "#e2e8f0" },
                cand.score,
                status_note
            ));
        }

        decision_html.push_str(&format!(
            r#"<div style="background: #0f172a; border: 1px solid #334155; border-radius: 8px; padding: 16px; margin-top: 16px;">
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px;">
                    <div>
                        <span style="font-size: 16px; font-weight: bold; margin-right: 8px;">Decision Proposal</span>
                        {}
                    </div>
                    <div style="font-size: 14px; font-weight: bold; color: #38bdf8;">
                        Target Recommendation: {}
                    </div>
                </div>

                <table style="width: 100%; border-collapse: collapse; font-size: 13px; text-align: left;">
                    <thead>
                        <tr style="border-bottom: 2px solid #475569; color: #94a3b8;">
                            <th style="padding: 8px;">Action</th>
                            <th style="padding: 8px; text-align: right;">P(success)</th>
                            <th style="padding: 8px; text-align: right;">d(a)</th>
                            <th style="padding: 8px; text-align: right;">E[U]</th>
                            <th style="padding: 8px; text-align: right;">CVaR_0.9</th>
                            <th style="padding: 8px; text-align: right;">Score J(a)</th>
                            <th style="padding: 8px;">Status</th>
                        </tr>
                    </thead>
                    <tbody>
                        {}
                    </tbody>
                </table>

                <div style="margin-top: 16px; padding: 12px; background: #1e293b; border-radius: 6px;">
                    <div style="font-weight: bold; font-size: 12px; color: #94a3b8; text-transform: uppercase;">Grounded AI Explanation ("Why?")</div>
                    <div style="font-size: 13px; color: #f1f5f9; margin-top: 6px; line-height: 1.4;">
                        {}
                    </div>
                </div>

                <div style="display: flex; gap: 12px; margin-top: 16px; justify-content: flex-end;">
                    <button style="background: #22c55e; color: white; border: none; padding: 8px 18px; border-radius: 6px; font-weight: bold; cursor: pointer;">APPROVE</button>
                    <button style="background: #3b82f6; color: white; border: none; padding: 8px 18px; border-radius: 6px; font-weight: bold; cursor: pointer;">MODIFY</button>
                    <button style="background: #ef4444; color: white; border: none; padding: 8px 18px; border-radius: 6px; font-weight: bold; cursor: pointer;">REJECT</button>
                </div>
            </div>"#,
            gate_badge, prop.recommended_action.display_name(), rows, prop.explanation
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>AERIS — Autonomous Mission Decision Console</title>
    <style>
        body {{
            margin: 0;
            padding: 24px;
            background: #0b0f17;
            color: #f8fafc;
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
        }}
        .grid {{
            display: grid;
            grid-template-columns: 2fr 1fr;
            gap: 20px;
        }}
        .card {{
            background: #131d2e;
            border: 1px solid #1e293b;
            border-radius: 8px;
            padding: 18px;
        }}
    </style>
</head>
<body>
    <div style="display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid #1e293b; padding-bottom: 16px; margin-bottom: 20px;">
        <div style="display: flex; align-items: center; gap: 16px;">
            <h1 style="margin: 0; font-size: 24px; letter-spacing: 1px;">AERIS CONSOLE</h1>
            <span style="background: {}; color: #ffffff; padding: 4px 10px; border-radius: 4px; font-weight: bold; font-size: 13px;">{}</span>
        </div>
        <div style="display: flex; gap: 24px; font-size: 14px; font-weight: 500;">
            <div>Tick: <strong>{}</strong></div>
            <div>Mission Score: <strong>{:.3}</strong></div>
        </div>
    </div>

    <div class="grid">
        <div style="display: flex; flex-direction: column; gap: 20px;">
            <div class="card">
                <h3 style="margin-top: 0; margin-bottom: 12px; font-size: 15px; color: #94a3b8;">OPERATIONAL TOPOLOGY & TRUST</h3>
                {}
            </div>
            {}
        </div>

        <div style="display: flex; flex-direction: column; gap: 20px;">
            <div class="card">
                <h3 style="margin-top: 0; margin-bottom: 16px; font-size: 15px; color: #94a3b8;">MISSION CAPABILITIES</h3>
                {}
            </div>

            <div class="card">
                <h3 style="margin-top: 0; margin-bottom: 16px; font-size: 15px; color: #94a3b8;">INCIDENT FEED</h3>
                {}
            </div>
        </div>
    </div>
</body>
</html>"#,
        mode_color,
        cycle.mode,
        cycle.tick,
        cycle.mission_score,
        svg_graph,
        decision_html,
        capabilities_html,
        incidents_html
    )
}

/// Renders the operational graph SVG with nodes coloured by trust,
/// isolated nodes dashed, and active/standby links.
pub fn render_svg_graph(cycle: &CycleOutput) -> String {
    // Find node trust scores and labels
    let get_node_trust = |node_id: &str| -> (f64, TrustLabel) {
        if let Some((_, tv)) = cycle.trust.iter().find(|(id, _)| id == node_id) {
            (tv.aggregate(None), tv.label(None))
        } else {
            (0.90, TrustLabel::Normal)
        }
    };

    let (t_a, lbl_a) = get_node_trust("node-a");
    let (t_b, lbl_b) = get_node_trust("node-b");
    let (t_c, lbl_c) = get_node_trust("node-c");

    let color_for_label = |lbl: TrustLabel| match lbl {
        TrustLabel::Normal => "#22c55e",
        TrustLabel::Suspicious => "#f97316",
        TrustLabel::Untrusted => "#ef4444",
    };

    let col_a = color_for_label(lbl_a);
    let col_b = color_for_label(lbl_b);
    let col_c = color_for_label(lbl_c);

    // Positions for Nodes A, B, C:
    // Node A (100, 120), Node B (300, 60), Node C (500, 120)
    // Primary link L1: A-B and B-C
    // Backup link L2: A-C direct (curves underneath at 180)
    let mut svg = String::new();
    svg.push_str("<svg viewBox=\"0 0 600 240\" width=\"100%\" height=\"240\" xmlns=\"http://www.w3.org/2000/svg\" style=\"background: #090d16; border-radius: 6px;\">\n");
    svg.push_str("  <line x1=\"100\" y1=\"120\" x2=\"300\" y2=\"60\" stroke=\"#38bdf8\" stroke-width=\"3\" />\n");
    svg.push_str("  <line x1=\"300\" y1=\"60\" x2=\"500\" y2=\"120\" stroke=\"#38bdf8\" stroke-width=\"3\" />\n");
    svg.push_str("  <text x=\"200\" y=\"80\" fill=\"#94a3b8\" font-size=\"11\" text-anchor=\"middle\">L1 (Relay)</text>\n");
    svg.push_str("  <path d=\"M 100 120 Q 300 210 500 120\" stroke=\"#64748b\" stroke-width=\"2\" stroke-dasharray=\"6,4\" fill=\"none\" />\n");
    svg.push_str("  <text x=\"300\" y=\"195\" fill=\"#64748b\" font-size=\"11\" text-anchor=\"middle\">L2 (Standby Backup)</text>\n");

    // Node A
    svg.push_str(&format!(
        "  <g transform=\"translate(100, 120)\"><circle r=\"28\" fill=\"#1e293b\" stroke=\"{}\" stroke-width=\"3\" /><text y=\"-4\" fill=\"white\" font-size=\"12\" font-weight=\"bold\" text-anchor=\"middle\">Node A</text><text y=\"14\" fill=\"{}\" font-size=\"10\" text-anchor=\"middle\">{:.2}</text></g>\n",
        col_a, col_a, t_a
    ));

    // Node B
    let dash_b = if lbl_b == TrustLabel::Untrusted {
        "stroke-dasharray=\"6,3\""
    } else {
        ""
    };
    svg.push_str(&format!(
        "  <g transform=\"translate(300, 60)\"><circle r=\"28\" fill=\"#1e293b\" stroke=\"{}\" stroke-width=\"3\" {} /><text y=\"-4\" fill=\"white\" font-size=\"12\" font-weight=\"bold\" text-anchor=\"middle\">Node B</text><text y=\"14\" fill=\"{}\" font-size=\"10\" text-anchor=\"middle\">{:.2}</text></g>\n",
        col_b, dash_b, col_b, t_b
    ));

    // Node C
    svg.push_str(&format!(
        "  <g transform=\"translate(500, 120)\"><circle r=\"28\" fill=\"#1e293b\" stroke=\"{}\" stroke-width=\"3\" /><text y=\"-4\" fill=\"white\" font-size=\"12\" font-weight=\"bold\" text-anchor=\"middle\">Node C</text><text y=\"14\" fill=\"{}\" font-size=\"10\" text-anchor=\"middle\">{:.2}</text></g>\n",
        col_c, col_c, t_c
    ));

    svg.push_str("</svg>");
    svg
}

/// Renders a plain-text console summary for CLI or headless mode.
pub fn render_terminal_summary(cycle: &CycleOutput) -> String {
    let mut out = String::new();
    out.push_str("====================================================================\n");
    out.push_str(&format!(
        " AERIS CONSOLE | TICK: {:04} | MODE: {:?} | MISSION SCORE: {:.3}\n",
        cycle.tick, cycle.mode, cycle.mission_score
    ));
    out.push_str("--------------------------------------------------------------------\n");
    out.push_str(" CAPABILITIES:\n");
    for (cap, val) in &cycle.capabilities {
        out.push_str(&format!("   - {:<15} : {:.3}\n", cap, val));
    }
    out.push_str(" TRUST VECTORS:\n");
    for (id, tv) in &cycle.trust {
        out.push_str(&format!(
            "   - {:<10} : Trust = {:.3} [{:?}]\n",
            id,
            tv.aggregate(None),
            tv.label(None)
        ));
    }
    if !cycle.incidents.is_empty() {
        out.push_str(" INCIDENTS:\n");
        for inc in &cycle.incidents {
            out.push_str(&format!(
                "   - Incident #{}: Target = {}, Risk = {:.3}, P(Compromise) = {:.1}%\n",
                inc.id,
                inc.entity,
                inc.risk,
                inc.posterior.compromise * 100.0
            ));
        }
    }
    if let Some(ref prop) = cycle.proposal {
        out.push_str(" DECISION PROPOSAL:\n");
        out.push_str(&format!(
            "   - Recommended: {}\n",
            prop.recommended_action.display_name()
        ));
        out.push_str(&format!("   - Gate: {:?}\n", prop.gate));
        out.push_str(&format!("   - Explanation: {}\n", prop.explanation));
    }
    out.push_str("====================================================================\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeris_types::{Capability, CycleOutput, Mode, TrustVector};

    #[test]
    fn test_render_html_dashboard() {
        let cycle = CycleOutput {
            tick: 10,
            mode: Mode::Normal,
            trust: vec![
                ("node-a".into(), TrustVector::default()),
                ("node-b".into(), TrustVector::default()),
                ("node-c".into(), TrustVector::default()),
            ],
            sensor_trust: vec![],
            capabilities: vec![
                (Capability::Navigation, 0.930),
                (Capability::Communication, 0.902),
                (Capability::Observation, 0.920),
            ],
            mission_score: 1.0,
            risks: vec![],
            incidents: vec![],
            proposal: None,
        };

        let html = render_html_dashboard(&cycle);
        assert!(html.contains("AERIS CONSOLE"));
        assert!(html.contains("NORMAL"));
        assert!(html.contains("Node A"));
        assert!(html.contains("Node B"));
        assert!(html.contains("Node C"));
    }
}
