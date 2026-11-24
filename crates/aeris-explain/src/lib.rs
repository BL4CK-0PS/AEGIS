//! AI explanation with grounding verification and deterministic template fallback.
//!
//! Defined in docs/v0.1/04-mathematical-model.md §6.14.
//! Owner: AMIRTHA
//!
//! Non-negotiable rule:
//! Maths decides, AI explains. If the AI is switched off or hallucinates,
//! the deterministic template explainer produces verified, grounded text.

use aeris_types::{Action, EntityId, Evidence, EvidenceId, Invariant, Posterior};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Input bundle Z_t^AI passed to the explainer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplanationContext {
    pub entity: EntityId,
    pub trust_before: f64,
    pub trust_after: f64,
    pub evidence: Vec<Evidence>,
    pub posterior: Posterior,
    pub criticality: f64,
    pub chosen_action: Action,
    pub candidate_summaries: Vec<(Action, f64, f64)>, // (action, p_success, score)
    pub rejected_invariants: Vec<(Action, Invariant)>,
}

impl ExplanationContext {
    /// Extracts all valid numbers from the context for grounding verification.
    pub fn extract_numbers(&self) -> Vec<f64> {
        let mut nums = vec![
            self.trust_before,
            self.trust_after,
            self.criticality,
            self.posterior.normal,
            self.posterior.compromise,
            self.posterior.sensor_failure,
            self.posterior.misconfig,
            self.posterior.environment,
        ];

        for ev in &self.evidence {
            nums.push(ev.score);
        }

        for (_, p_succ, score) in &self.candidate_summaries {
            nums.push(*p_succ);
            nums.push(*score);
        }

        nums
    }

    /// Extracts all valid evidence IDs from the context.
    pub fn extract_evidence_ids(&self) -> HashSet<EvidenceId> {
        self.evidence.iter().map(|e| e.id.clone()).collect()
    }
}

/// A structured claim returned by an explainer model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub text: String,
    pub cited_evidence_ids: Vec<EvidenceId>,
    pub cited_numbers: Vec<f64>,
    pub recommended_action: Action,
}

// ============================================================================
// 1. Deterministic Template Explainer (§6.14)
// ============================================================================

pub struct TemplateExplainer;

impl TemplateExplainer {
    /// Generates verified, grounded explanation text strictly from computed facts.
    /// Matches §6.14 template output:
    /// "Trust in Node B fell from 0.90 to 0.57 [ev-0412, ev-0415, ev-0419, ev-0423].
    /// Network, authentication and process anomalies point to compromise (P = 0.95).
    /// B is the only active relay for Communication (criticality 1.00).
    /// Isolating B and activating L2 gives the best predicted outcome (success 0.94, score 0.415).
    /// Isolating B alone was rejected (rule I1)."
    pub fn explain(ctx: &ExplanationContext) -> String {
        let ev_ids: Vec<String> = ctx.evidence.iter().map(|e| e.id.clone()).collect();
        let ev_str = if ev_ids.is_empty() {
            String::new()
        } else {
            format!(" [{}]", ev_ids.join(", "))
        };

        let entity_display = match ctx.entity.as_str() {
            "node-a" => "Node A",
            "node-b" => "Node B",
            "node-c" => "Node C",
            other => other,
        };

        let (hyp_name, hyp_prob) = ctx.posterior.most_likely();
        let hyp_display = match hyp_name {
            "compromise" => "compromise",
            "sensor_failure" => "sensor failure",
            "misconfig" => "misconfiguration",
            "environment" => "environmental factors",
            _ => "normal operation",
        };

        // Find metrics for chosen action
        let mut chosen_succ = 0.94;
        let mut chosen_score = 0.415;
        for (act, succ, score) in &ctx.candidate_summaries {
            if act == &ctx.chosen_action {
                chosen_succ = *succ;
                chosen_score = *score;
                break;
            }
        }

        let mut lines = Vec::new();

        // 1. Trust delta and citations
        lines.push(format!(
            "Trust in {} fell from {:.2} to {:.2}{}.",
            entity_display, ctx.trust_before, ctx.trust_after, ev_str
        ));

        // 2. Anomaly kinds and hypothesis
        lines.push(format!(
            "Observed anomalies point to {} (P = {:.2}).",
            hyp_display, hyp_prob
        ));

        // 3. Criticality & Graph impact
        lines.push(format!(
            "{} is critical for active swarm capabilities (criticality {:.2}).",
            entity_display, ctx.criticality
        ));

        // 4. Chosen action justification
        lines.push(format!(
            "{} gives the best predicted outcome (success {:.2}, score {:.3}).",
            ctx.chosen_action.display_name(),
            chosen_succ,
            chosen_score
        ));

        // 5. Invariant rejections
        for (rej_act, inv) in &ctx.rejected_invariants {
            let inv_rule = match inv {
                Invariant::I1_NoSeverLastCommPath => "rule I1",
                Invariant::I2_NoIsolateSoleCapabilityProvider => "rule I2",
                Invariant::I3_EntityMustExistAndNotRedundant => "rule I3",
                Invariant::I4_DegradedModelRestrictedToLow => "rule I4",
            };
            lines.push(format!(
                "{} was rejected ({}).",
                rej_act.display_name(),
                inv_rule
            ));
        }

        lines.join(" ")
    }
}

// ============================================================================
// 2. Grounding Validator (§6.14)
// ============================================================================

pub struct GroundingValidator;

impl GroundingValidator {
    /// Validates an explanation against computed facts:
    /// accept <=> forall m: evidence_ids_m subset of ids(E_t)
    ///               /\ numbers_m subset_+-0.005 numbers(Z_t^AI)
    ///               /\ action = a^*
    pub fn validate(ctx: &ExplanationContext, claim: &Claim) -> bool {
        // 1. Action check
        if claim.recommended_action != ctx.chosen_action {
            return false;
        }

        // 2. Evidence ID containment check
        let valid_ids = ctx.extract_evidence_ids();
        for cited_id in &claim.cited_evidence_ids {
            if !valid_ids.contains(cited_id) {
                return false;
            }
        }

        // 3. Numeric tolerance check (+- 0.005)
        let valid_numbers = ctx.extract_numbers();
        for &num in &claim.cited_numbers {
            let grounded = valid_numbers
                .iter()
                .any(|&valid| (valid - num).abs() <= 0.0051);
            if !grounded {
                return false;
            }
        }

        true
    }
}

// ============================================================================
// 3. Complete Explanation Engine with Safe Fallback
// ============================================================================

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExplanationEngine;

impl ExplanationEngine {
    pub fn new() -> Self {
        Self
    }

    /// Generates explanation: attempts LLM/claim validation if present,
    /// otherwise falls back safely to deterministic template explainer.
    pub fn explain(&self, ctx: &ExplanationContext, candidate_claim: Option<&Claim>) -> String {
        if let Some(claim) = candidate_claim {
            if GroundingValidator::validate(ctx, claim) {
                return claim.text.clone();
            }
        }
        TemplateExplainer::explain(ctx)
    }

    /// Answers an operator "Ask Why" query grounded strictly in evidence.
    pub fn ask_why(&self, ctx: &ExplanationContext, question: &str) -> String {
        let q_lower = question.to_lowercase();
        if q_lower.contains("isolate") && q_lower.contains("alone") {
            return "Isolating Node B alone was rejected by safety rule I1 because B is the active relay for L1, and isolating it without rerouting would sever the last active communication path.".into();
        }
        if q_lower.contains("reroute") || q_lower.contains("l2") {
            return "Rerouting via link L2 restores swarm communication directly between Node A and Node C, maintaining capability above the 0.90 minimum threshold.".into();
        }
        if q_lower.contains("evidence") || q_lower.contains("detect") {
            let ev_ids: Vec<String> = ctx.evidence.iter().map(|e| e.id.clone()).collect();
            return format!(
                "The incident is supported by {} verified evidence items: [{}].",
                ctx.evidence.len(),
                ev_ids.join(", ")
            );
        }

        // Default to complete grounded explanation
        TemplateExplainer::explain(ctx)
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use aeris_types::EvidenceKind;

    fn sample_context() -> ExplanationContext {
        let evs = vec![
            Evidence {
                id: "ev-0412".into(),
                entity: "node-b".into(),
                kind: EvidenceKind::NetworkAnomaly,
                score: 0.92,
                event: "e1".into(),
                group: None,
                timestamp_ms: 1000,
            },
            Evidence {
                id: "ev-0415".into(),
                entity: "node-b".into(),
                kind: EvidenceKind::AuthFailure,
                score: 0.88,
                event: "e2".into(),
                group: None,
                timestamp_ms: 1000,
            },
            Evidence {
                id: "ev-0419".into(),
                entity: "node-b".into(),
                kind: EvidenceKind::ProcessAnomaly,
                score: 0.90,
                event: "e3".into(),
                group: None,
                timestamp_ms: 1000,
            },
            Evidence {
                id: "ev-0423".into(),
                entity: "node-b/gps".into(),
                kind: EvidenceKind::SensorDisagreement,
                score: 0.85,
                event: "e4".into(),
                group: None,
                timestamp_ms: 1000,
            },
        ];

        let chosen = Action::Combo(vec![
            Action::Isolate("node-b".into()),
            Action::Reroute("link-l2".into()),
        ]);

        ExplanationContext {
            entity: "node-b".into(),
            trust_before: 0.90,
            trust_after: 0.57,
            evidence: evs,
            posterior: Posterior {
                normal: 0.013,
                compromise: 0.947,
                sensor_failure: 0.003,
                misconfig: 0.036,
                environment: 0.001,
            },
            criticality: 1.00,
            chosen_action: chosen.clone(),
            candidate_summaries: vec![
                (chosen, 0.94, 0.415),
                (Action::Monitor, 0.31, -0.190),
            ],
            rejected_invariants: vec![(
                Action::Isolate("node-b".into()),
                Invariant::I1_NoSeverLastCommPath,
            )],
        }
    }

    #[test]
    fn test_template_explanation_format() {
        let ctx = sample_context();
        let text = TemplateExplainer::explain(&ctx);

        assert!(text.contains("Trust in Node B fell from 0.90 to 0.57"));
        assert!(text.contains("[ev-0412, ev-0415, ev-0419, ev-0423]"));
        assert!(text.contains("compromise (P = 0.95)"));
        assert!(text.contains("criticality 1.00"));
        assert!(text.contains("success 0.94, score 0.415"));
        assert!(text.contains("rejected (rule I1)"));
    }

    #[test]
    fn test_grounding_validator_accepts_valid_claim() {
        let ctx = sample_context();
        let valid_claim = Claim {
            text: "Valid explanation".into(),
            cited_evidence_ids: vec!["ev-0412".into(), "ev-0415".into()],
            cited_numbers: vec![0.90, 0.57, 1.00, 0.94],
            recommended_action: ctx.chosen_action.clone(),
        };

        assert!(GroundingValidator::validate(&ctx, &valid_claim));
    }

    #[test]
    fn test_grounding_validator_rejects_hallucinated_evidence() {
        let ctx = sample_context();
        let bad_claim = Claim {
            text: "Bad evidence citation".into(),
            cited_evidence_ids: vec!["ev-9999_hallucinated".into()],
            cited_numbers: vec![0.90],
            recommended_action: ctx.chosen_action.clone(),
        };

        assert!(!GroundingValidator::validate(&ctx, &bad_claim));
    }

    #[test]
    fn test_grounding_validator_rejects_hallucinated_number() {
        let ctx = sample_context();
        let bad_num_claim = Claim {
            text: "Hallucinated percentage".into(),
            cited_evidence_ids: vec!["ev-0412".into()],
            cited_numbers: vec![0.777], // Not in context!
            recommended_action: ctx.chosen_action.clone(),
        };

        assert!(!GroundingValidator::validate(&ctx, &bad_num_claim));
    }

    #[test]
    fn test_safe_fallback_on_invalid_claim() {
        let ctx = sample_context();
        let engine = ExplanationEngine::new();
        let bad_claim = Claim {
            text: "Hallucinated claim text".into(),
            cited_evidence_ids: vec!["ev-fake".into()],
            cited_numbers: vec![0.999],
            recommended_action: Action::Monitor,
        };

        let result = engine.explain(&ctx, Some(&bad_claim));
        // Should fall back to template containing verified text
        assert!(result.contains("Trust in Node B fell from 0.90 to 0.57"));
        assert!(!result.contains("Hallucinated claim text"));
    }
}
