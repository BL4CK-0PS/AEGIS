//! AERIS Core Pipeline Engine.
//!
//! Orchestrates the full computational loop without any external I/O:
//! Events -> Anomaly/Fusion -> Trust -> Graph/Capabilities -> Risk -> Decision -> Verification.
//!
//! Defined in docs/v0.1/04-mathematical-model.md §6 and docs/v0.1/06-contracts.md.
//! Owner: KISHANTH

use aeris_anomaly::AnomalyEngine;
use aeris_decision::{
    DecisionParams, evaluate_rollout_samples, evaluate_summary, generate_candidates_for_incident,
    optimize_decision,
};
use aeris_estimator::{DiscreteNodeState, SwarmEstimator};
use aeris_explain::{ExplanationContext, ExplanationEngine};
use aeris_fusion::FusionEngine;
use aeris_mission::{Capabilities, LinkView, Minima, NodeView, Topology};
use aeris_trust::{TrustParams, TrustRegistry};
use aeris_types::{
    Action, Capability, CycleOutput, EntityId, EventPayload, Gate, Incident, IncidentStatus,
    Invariant, LinkId, Mode, Posterior, Proposal, VerifiedEvent,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Topological link configuration in the operational graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkState {
    pub id: LinkId,
    pub source: EntityId,
    pub target: EntityId,
    pub via: Option<EntityId>,
    pub availability: f64,
    pub active: bool,
}

/// Operational state of a node in the swarm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Healthy,
    Suspicious,
    Compromised,
    Isolated,
    Restricted,
}

/// The state of the AERIS core engine, maintained across ticks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreState {
    pub tick: u64,
    pub mode: Mode,
    pub recovery_timer_ticks: u64,
    pub nodes: HashMap<EntityId, NodeStatus>,
    pub links: HashMap<LinkId, LinkState>,
    pub trust_registry: TrustRegistry,
    pub incidents: Vec<Incident>,
    pub next_incident_num: u32,
    pub active_proposal: Option<Proposal>,
    pub last_action_executed: Option<Action>,
    pub verification_passed: Option<bool>,
    pub decision_params: DecisionParams,
    pub processed_event_seqs: HashSet<(String, u64)>,
    pub anomaly_engine: AnomalyEngine,
    pub fusion_engine: FusionEngine,
    pub swarm_estimator: SwarmEstimator,
    pub explainer: ExplanationEngine,
}

impl Default for CoreState {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreState {
    pub fn new() -> Self {
        let mut trust_reg = TrustRegistry::new(TrustParams::default());
        trust_reg.register_node("node-a".into());
        trust_reg.register_node("node-b".into());
        trust_reg.register_node("node-c".into());

        trust_reg.register_sensor("node-a/gps".into());
        trust_reg.register_sensor("node-b/gps".into());
        trust_reg.register_sensor("node-c/gps".into());
        // Camera reliability prior T^obs = camera_quality from docs/spec/world.toml,
        // with the same evidence mass (alpha + beta) as the default sensor prior.
        if let Some(cfg) = rollout_config() {
            for n in &cfg.nodes {
                let id = format!("{}/camera", n.id);
                trust_reg.register_sensor(id.clone());
                if let Some(t) = trust_reg.sensors.get_mut(&id) {
                    let mass = t.beta.alpha + t.beta.beta;
                    t.beta = aeris_types::Beta::new(
                        mass * n.camera_quality,
                        mass * (1.0 - n.camera_quality),
                    );
                }
            }
        }

        let mut nodes = HashMap::new();
        nodes.insert("node-a".into(), NodeStatus::Healthy);
        nodes.insert("node-b".into(), NodeStatus::Healthy);
        nodes.insert("node-c".into(), NodeStatus::Healthy);

        let mut links = HashMap::new();
        // L1 is A-B-C relayed through B (initially active)
        links.insert(
            "link-l1".into(),
            LinkState {
                id: "link-l1".into(),
                source: "node-a".into(),
                target: "node-c".into(),
                via: Some("node-b".into()),
                availability: 0.99,
                active: true,
            },
        );
        // L2 is direct A-C backup (initially standby)
        links.insert(
            "link-l2".into(),
            LinkState {
                id: "link-l2".into(),
                source: "node-a".into(),
                target: "node-c".into(),
                via: None,
                availability: 0.93,
                active: false,
            },
        );

        // Kalman filters start from the known launch state (world.toml), not the origin.
        let mut swarm_estimator = SwarmEstimator::default();
        if let Some(cfg) = rollout_config() {
            for n in &cfg.nodes {
                if let Some(kf) = swarm_estimator.kalman_filters.get_mut(&n.id) {
                    kf.state.p = n.position_m[0];
                    kf.state.v = n.velocity_mps[0];
                }
            }
        }

        Self {
            tick: 0,
            mode: Mode::Normal,
            recovery_timer_ticks: 0,
            nodes,
            links,
            trust_registry: trust_reg,
            incidents: Vec::new(),
            next_incident_num: 1,
            active_proposal: None,
            last_action_executed: None,
            verification_passed: None,
            decision_params: DecisionParams::default(),
            processed_event_seqs: HashSet::new(),
            anomaly_engine: AnomalyEngine::default(),
            fusion_engine: FusionEngine::default(),
            swarm_estimator,
            explainer: ExplanationEngine,
        }
    }

    /// AERIS's own view of the world for the mission model: topology and
    /// constants from `docs/spec/world.toml`, activity from executed actions,
    /// trust from `aeris-trust`, localisation variance from `aeris-estimator`.
    /// Never ground truth.
    pub fn topology(&self) -> Topology {
        let prior = aeris_types::Beta::default().mean();
        let unmet = Minima {
            navigation: 1.0,
            communication: 1.0,
            observation: 1.0,
        };
        let Some(cfg) = rollout_config() else {
            return Topology {
                nodes: Vec::new(),
                links: Vec::new(),
                endpoints: [String::new(), String::new()],
                minima: unmet,
                degraded_minima: unmet,
                sigma_max: 0.0,
            };
        };
        let minima = |m: &aeris_sim::config::CapabilityMinima| Minima {
            navigation: m.navigation,
            communication: m.communication,
            observation: m.observation,
        };
        let nodes = cfg
            .nodes
            .iter()
            .map(|n| NodeView {
                id: n.id.clone(),
                active: self.nodes.get(&n.id) != Some(&NodeStatus::Isolated),
                trust: self.trust_registry.node_trust(&n.id).unwrap_or(prior),
                coverage: n.coverage,
                obs_trust: self
                    .trust_registry
                    .sensor_trust(&format!("{}/camera", n.id))
                    .unwrap_or(n.camera_quality),
                position_variance: self
                    .swarm_estimator
                    .kalman_filters
                    .get(&n.id)
                    .map_or(f64::INFINITY, |kf| kf.state.cov[0][0]),
            })
            .collect();
        let links = cfg
            .links
            .iter()
            .map(|l| LinkView {
                id: l.id.clone(),
                endpoints: l.endpoints.clone(),
                via: l.via.clone(),
                hop_availability: l.hop_availability.clone(),
                active: self.links.get(&l.id).map_or(l.active, |s| s.active),
            })
            .collect();
        Topology {
            nodes,
            links,
            endpoints: cfg.mission.endpoints.clone(),
            minima: minima(&cfg.mission.min),
            degraded_minima: minima(&cfg.mission.degraded_min),
            sigma_max: cfg.mission.sigma_max_m,
        }
    }

    /// Capability levels c_k (§6.7), computed by `aeris-mission`.
    pub fn compute_capabilities(&self) -> Vec<(Capability, f64)> {
        self.topology().capabilities().as_vec()
    }

    /// Continuous mission score M(G) against the minima in `docs/spec/world.toml`.
    pub fn compute_mission_score(&self, capabilities: &[(Capability, f64)]) -> f64 {
        aeris_mission::mission_score(&caps_from_vec(capabilities), &self.topology().minima)
    }

    /// Counterfactual criticality C_i = (M(G) - M(G minus v_i)) / M(G).
    pub fn compute_criticality(&self, entity: &str) -> f64 {
        self.topology().criticality(entity)
    }

    /// Applies an action directly to state (execution stage)
    pub fn apply_action(&mut self, action: &Action) {
        self.last_action_executed = Some(action.clone());
        self.execute_action_internal(action);
    }

    fn execute_action_internal(&mut self, action: &Action) {
        match action {
            Action::Monitor => {}
            Action::Restrict(id) => {
                self.nodes.insert(id.clone(), NodeStatus::Restricted);
            }
            Action::Isolate(id) => {
                self.nodes.insert(id.clone(), NodeStatus::Isolated);
                if let Some(hmm) = self.swarm_estimator.hmm_filters.get_mut(id) {
                    hmm.force_state(DiscreteNodeState::Isolated);
                }
            }
            Action::Reroute(link_id) => {
                if let Some(link) = self.links.get_mut(link_id) {
                    link.active = true;
                }
            }
            Action::Combo(actions) => {
                for a in actions {
                    self.execute_action_internal(a);
                }
            }
            Action::Failover { .. } => {}
            Action::Degrade(_) => {}
            Action::Recover(id) => {
                self.nodes.insert(id.clone(), NodeStatus::Healthy);
                if let Some(hmm) = self.swarm_estimator.hmm_filters.get_mut(id) {
                    hmm.force_state(DiscreteNodeState::Normal);
                }
            }
        }
    }
}

fn caps_from_vec(v: &[(Capability, f64)]) -> Capabilities {
    let get = |c: Capability| v.iter().find(|(k, _)| *k == c).map_or(0.0, |(_, x)| *x);
    Capabilities {
        navigation: get(Capability::Navigation),
        communication: get(Capability::Communication),
        observation: get(Capability::Observation),
    }
}

/// The frozen simulated world used for counterfactual rollouts.
fn rollout_config() -> Option<std::sync::Arc<aeris_sim::WorldConfig>> {
    static CFG: std::sync::OnceLock<Option<std::sync::Arc<aeris_sim::WorldConfig>>> =
        std::sync::OnceLock::new();
    CFG.get_or_init(|| {
        aeris_sim::WorldConfig::frozen()
            .ok()
            .map(std::sync::Arc::new)
    })
    .clone()
}

/// Rollout base world built from the pipeline's own view of the topology
/// (node statuses and link activation). No ground truth is available here.
fn rollout_world(state: &CoreState, seed: u64) -> Option<aeris_sim::World> {
    let cfg = rollout_config()?;
    let mut nodes: Vec<(String, aeris_wire::NodeMode)> = cfg
        .nodes
        .iter()
        .map(|n| {
            let mode = match state.nodes.get(&n.id) {
                Some(NodeStatus::Isolated) => aeris_wire::NodeMode::Isolated,
                Some(NodeStatus::Restricted) => aeris_wire::NodeMode::Restricted,
                _ => aeris_wire::NodeMode::Active,
            };
            (n.id.clone(), mode)
        })
        .collect();
    nodes.sort_by(|a, b| a.0.cmp(&b.0));
    let links = cfg
        .links
        .iter()
        .map(|l| aeris_wire::LinkObservation {
            link: l.id.clone(),
            active: state.links.get(&l.id).map_or(l.active, |s| s.active),
            up: true,
            availability: l.hop_availability.iter().product(),
            latency_ms: l.latency_ms.iter().sum(),
            loss: 0.0,
        })
        .collect();
    let obs = aeris_wire::WorldObservation {
        tick: state.tick,
        t_unix_ms: 0,
        nodes,
        links,
        degraded: Vec::new(),
    };
    aeris_sim::World::from_observation(
        std::sync::Arc::clone(&cfg),
        &obs,
        aeris_sim::rng::derive_seed(seed, "core-rollout-base", state.tick),
    )
    .ok()
}

/// Belief for rollouts: compromise / sensor-failure probabilities from the
/// fused posteriors of nodes known to the world model.
fn rollout_belief(
    incident: &Incident,
    posteriors: &HashMap<EntityId, Posterior>,
) -> aeris_sim::Belief {
    let Some(cfg) = rollout_config() else {
        return aeris_sim::Belief::default();
    };
    let mut nodes = Vec::new();
    for n in &cfg.nodes {
        let post = if n.id == incident.entity {
            Some(&incident.posterior)
        } else {
            posteriors.get(&n.id)
        };
        if let Some(p) = post {
            nodes.push(aeris_sim::NodeBelief {
                node: n.id.clone(),
                p_compromised: p.compromise.clamp(0.0, 1.0),
                p_gps_failed: p.sensor_failure.clamp(0.0, 1.0),
            });
        }
    }
    aeris_sim::Belief { nodes }
}

/// The main entry point: executes one cycle of the computational loop without I/O.
pub fn step(state: &mut CoreState, events: &[VerifiedEvent], seed: u64) -> CycleOutput {
    state.tick += 1;

    // ------------------------------------------------------------------------
    // Stage 1: Ingest & Deduplication of Events
    // ------------------------------------------------------------------------
    let mut valid_events = Vec::new();
    for ev in events {
        if !ev.signature_valid {
            continue;
        }
        let key = (ev.source.clone(), ev.seq);
        if !state.processed_event_seqs.insert(key) {
            // Duplicate event, skip
            continue;
        }
        valid_events.push(ev.clone());
    }

    // ------------------------------------------------------------------------
    // Stage 2: State Estimation Prediction & Kalman Filter Updates
    // ------------------------------------------------------------------------
    state.swarm_estimator.predict_all();

    for ev in &valid_events {
        if let EventPayload::Sensor { kind, value, sigma } = &ev.payload {
            let sensor_entity = format!("{}/{}", ev.source, kind);
            let t_obs = state
                .trust_registry
                .sensor_trust(&sensor_entity)
                .unwrap_or(0.90);
            if let Some(val) = value.first()
                && let Some(kf) = state.swarm_estimator.kalman_filters.get_mut(&ev.source)
            {
                kf.update_sensor(*val, *sigma, t_obs);
            }
        }
    }

    // ------------------------------------------------------------------------
    // Stage 3: Anomaly Detection (Extract Evidence from Events)
    // ------------------------------------------------------------------------
    let evidence_batch = state.anomaly_engine.process_events(&valid_events);

    // ------------------------------------------------------------------------
    // Stage 4: Hypothesis Fusion & HMM Forward Filter
    // ------------------------------------------------------------------------
    let posterior_by_entity = state.fusion_engine.fuse_all_by_entity(&evidence_batch);

    for (entity, post) in &posterior_by_entity {
        if let Some(hmm) = state.swarm_estimator.hmm_filters.get_mut(entity) {
            hmm.update(post);
        }
    }

    // ------------------------------------------------------------------------
    // Stage 5: Trust Updates
    // ------------------------------------------------------------------------
    state.trust_registry.update_from_evidence(&evidence_batch);

    // ------------------------------------------------------------------------
    // Stage 6 & 7: Graph, Capabilities & Risk
    // ------------------------------------------------------------------------
    let capabilities = state.compute_capabilities();
    let mission_score = state.compute_mission_score(&capabilities);

    let topology = state.topology();
    let mut risks = Vec::new();
    // (entity, risk, posterior) of the highest-risk node above the containment threshold.
    let mut high_risk: Option<(EntityId, f64, Posterior)> = None;

    for node in &topology.nodes {
        let post = posterior_by_entity
            .get(&node.id)
            .cloned()
            .unwrap_or_default();
        let criticality = topology.criticality(&node.id);
        let blast_radius = topology.blast_radius(&node.id);
        let impact = aeris_mission::impact(criticality, blast_radius);
        let risk = aeris_mission::risk(post.normal, impact);

        risks.push((node.id.clone(), risk));

        if risk > 0.60 && high_risk.as_ref().is_none_or(|(_, r, _)| risk > *r) {
            high_risk = Some((node.id.clone(), risk, post));
        }
    }

    // ------------------------------------------------------------------------
    // Incident Management
    // ------------------------------------------------------------------------
    if let Some((target, target_risk, post)) = high_risk {
        let has_open = state
            .incidents
            .iter()
            .any(|inc| inc.entity == target && inc.status == IncidentStatus::Open);
        if !has_open {
            let inc_id = format!("inc-{:02}", state.next_incident_num);
            state.next_incident_num += 1;
            let ev_ids: Vec<String> = evidence_batch.iter().map(|e| e.id.clone()).collect();
            state.incidents.push(Incident {
                id: inc_id,
                entity: target,
                opened_at_tick: state.tick,
                status: IncidentStatus::Open,
                posterior: post,
                evidence_ids: ev_ids,
                risk: target_risk,
            });
        }
    }

    // ------------------------------------------------------------------------
    // Mode State Machine
    // ------------------------------------------------------------------------
    let max_risk = risks.iter().map(|(_, r)| *r).fold(0.0f64, f64::max);
    if state.mode == Mode::Normal && max_risk > 0.30 && max_risk <= 0.60 {
        state.mode = Mode::Degraded;
    } else if (state.mode == Mode::Normal || state.mode == Mode::Degraded) && max_risk > 0.60 {
        state.mode = Mode::Containment;
    }

    // ------------------------------------------------------------------------
    // Stage 8 & 9: Decision Optimization (Rollout Simulation)
    // ------------------------------------------------------------------------
    let open_incident = state
        .incidents
        .iter()
        .find(|inc| inc.status == IncidentStatus::Open)
        .cloned();

    let mut proposal = None;

    if let Some(incident) = open_incident {
        let target_node = incident.entity.clone();
        // A standby link between the mission endpoints that does not route
        // through the target. If none exists, reroute candidates are vetoed (I3).
        let backup_link = topology
            .links
            .iter()
            .find(|l| !l.active && !l.via.contains(&target_node))
            .map(|l| l.id.clone())
            .unwrap_or_default();

        let raw_actions = generate_candidates_for_incident(&target_node, &backup_link);
        let mut candidates = Vec::new();

        // Counterfactual rollouts (§6.10) run in aeris-sim from AERIS's own
        // view of the topology and its belief about hidden state; ground truth
        // never enters. Integration by DHARSHAN; replaces a hardcoded
        // p_success table (see docs/audit/dharshan-initial-audit.md C-3).
        let rollout_spec = aeris_sim::RolloutSpec {
            seed: seed ^ state.tick,
            ..aeris_sim::RolloutSpec::v01(0)
        };
        let rollout_base = rollout_world(state, seed);
        let belief = rollout_belief(&incident, &posterior_by_entity);

        for action in raw_actions {
            // Safety monitor (aeris-policy) vetoes before scoring.
            let violations = aeris_policy::check_invariants(&topology, &action);

            if !violations.is_empty() {
                // Invariant violated: candidate is marked and never scored
                let mut cand = evaluate_summary(action, 0.0, 500, &state.decision_params);
                cand.violations = violations;
                candidates.push(cand);
                continue;
            }

            // Simulate N rollouts of Γ(a) for the feasible candidate.
            let batch = match (&rollout_base, aeris_wire::commands_for_action(&action)) {
                (Some(base), Ok(plan)) => {
                    aeris_sim::run_rollouts(base, &belief, &plan, &rollout_spec).ok()
                }
                _ => None,
            };
            match batch {
                Some(b) => {
                    let cand =
                        evaluate_rollout_samples(action, &b.successes(), &state.decision_params);
                    candidates.push(cand);
                }
                None => {
                    // Not executable in the world model: infeasible, never scored.
                    let mut cand = evaluate_summary(action, 0.0, 500, &state.decision_params);
                    cand.violations =
                        vec![aeris_types::Invariant::I3_EntityMustExistAndNotRedundant];
                    candidates.push(cand);
                }
            }
        }

        let target_crit = topology.criticality(&target_node);
        let uncertainty = incident.posterior.normalized_entropy();

        let model_health = state.swarm_estimator.overall_model_health();
        let mut prop = optimize_decision(
            format!("dec-{:02}", state.tick),
            incident.id.clone(),
            candidates.clone(),
            target_crit,
            uncertainty,
            model_health,
            &state.decision_params,
        );

        // Stage 10: Grounded Explanation Generation via aeris-explain
        let candidate_summaries: Vec<(Action, f64, f64)> = candidates
            .iter()
            .map(|c| (c.action.clone(), c.p_success, c.score))
            .collect();

        let rejected_invariants: Vec<(Action, Invariant)> = candidates
            .iter()
            .filter_map(|c| c.violations.first().map(|v| (c.action.clone(), v.clone())))
            .collect();

        // No literals from the spec's worked example (audit H-4): the
        // "before" value is the configured trust prior.
        let prior = aeris_types::Beta::default().mean();
        let target_node_trust = state
            .trust_registry
            .node_trust(&target_node)
            .unwrap_or(prior);

        let expl_ctx = ExplanationContext {
            entity: target_node.clone(),
            trust_before: prior,
            trust_after: target_node_trust,
            evidence: evidence_batch.clone(),
            posterior: incident.posterior.clone(),
            criticality: target_crit,
            chosen_action: prop.recommended_action.clone(),
            candidate_summaries,
            rejected_invariants,
        };

        prop.explanation = state.explainer.explain(&expl_ctx, None);
        prop.cited_evidence = incident.evidence_ids.clone();

        state.active_proposal = Some(prop.clone());
        proposal = Some(prop);

        // If high-impact or human review required, set mode
        if let Some(ref p) = proposal
            && matches!(p.gate, Gate::HumanReview { .. })
        {
            state.mode = Mode::HumanReview;
        }
    }

    // ------------------------------------------------------------------------
    // Verification Stage (if action was executed)
    // ------------------------------------------------------------------------
    if let Some(ref last_act) = state.last_action_executed {
        // §6.13: the executed action's structural effect is observed, and every
        // capability meets its (possibly degraded) minimum.
        let now = state.topology();
        let expected = now.apply(last_act);
        let structure_ok = expected
            .nodes
            .iter()
            .all(|n| now.node(&n.id).is_some_and(|m| m.active == n.active))
            && expected
                .links
                .iter()
                .all(|l| now.link(&l.id).is_some_and(|m| m.active == l.active));
        let is_recovered = !matches!(last_act, Action::Monitor)
            && structure_ok
            && aeris_mission::mission_success(&caps_from_vec(&capabilities), &expected.minima);
        state.verification_passed = Some(is_recovered);
        if is_recovered {
            state.mode = Mode::Recovery;
            state.recovery_timer_ticks += 1;
            // Close incident
            for inc in &mut state.incidents {
                if inc.status == IncidentStatus::Open {
                    inc.status = IncidentStatus::Closed;
                }
            }
            if state.recovery_timer_ticks >= 10 && max_risk < 0.30 {
                state.mode = Mode::Normal;
            }
        }
    }

    // Assemble trust map and sensor trust map for cycle output
    let mut trust_vec = Vec::new();
    for (node_id, tracker) in &state.trust_registry.nodes {
        trust_vec.push((node_id.clone(), tracker.vector));
    }

    let mut sensor_vec = Vec::new();
    for (sensor_id, tracker) in &state.trust_registry.sensors {
        sensor_vec.push((sensor_id.clone(), tracker.trust_score()));
    }

    CycleOutput {
        tick: state.tick,
        mode: state.mode,
        trust: trust_vec,
        sensor_trust: sensor_vec,
        capabilities,
        mission_score,
        risks,
        incidents: state.incidents.clone(),
        proposal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full end-to-end integration test of Scenario S4 (Golden Path):
    /// 1. Normal state: mode is NORMAL, capabilities all >= minimum, M(G) = 1.0.
    /// 2. Compromise injected on Node B: anomalies arrive, trust in B drops, comm drops,
    ///    risk rises, incident opened, mode moves to CONTAINMENT / HUMAN_REVIEW.
    /// 3. Decision proposed: Isolate B alone rejected by I1; Isolate B + Reroute L2 selected
    ///    as argmax J(a) with gate HUMAN_REVIEW.
    /// 4. Commander approves and action is applied: Node B is isolated, Link L2 activated.
    /// 5. Verification: comm restored to >= 0.90, mission score = 1.0, mode -> RECOVERY.
    #[test]
    fn test_scenario_s4_full_loop() {
        let mut state = CoreState::new();

        // Step 1: Normal tick with nominal events
        let out_normal = step(&mut state, &[], 42);
        assert_eq!(out_normal.mode, Mode::Normal);
        assert!((out_normal.mission_score - 1.0).abs() < 1e-4);
        assert!(out_normal.incidents.is_empty());
        assert!(out_normal.proposal.is_none());

        // Step 2: Inject attack on Node B at t = 30s
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
            VerifiedEvent {
                id: "evt-04".into(),
                source: "node-b".into(),
                seq: 4,
                hlc: Default::default(),
                t_unix_ms: 30400,
                replayed: false,
                payload: EventPayload::Sensor {
                    kind: "gps".into(),
                    value: vec![120.0, 0.0],
                    sigma: 8.5,
                },
                signature_valid: true,
            },
        ];

        let out_attack = step(&mut state, &attack_events, 42);

        // Verify that an incident is opened on Node B
        assert_eq!(out_attack.incidents.len(), 1);
        let inc = &out_attack.incidents[0];
        assert_eq!(inc.entity, "node-b");
        assert_eq!(inc.status, IncidentStatus::Open);
        assert!(inc.posterior.compromise > 0.90);

        // Verify proposal is generated
        assert!(out_attack.proposal.is_some());
        let prop = out_attack.proposal.unwrap();

        // The recommended action MUST be Isolate B + Reroute L2
        let expected_action = Action::Combo(vec![
            Action::Isolate("node-b".into()),
            Action::Reroute("link-l2".into()),
        ]);
        assert_eq!(prop.recommended_action, expected_action);

        // Check that Isolate alone was vetoed by invariant I1
        let isolate_cand = prop
            .candidates
            .iter()
            .find(|c| matches!(c.action, Action::Isolate(_)))
            .expect("isolate candidate must exist");
        assert!(
            isolate_cand
                .violations
                .contains(&aeris_types::Invariant::I1_NoSeverLastCommPath)
        );

        // Gate is HumanReview because B's criticality is 1.0 (High impact)
        assert!(matches!(prop.gate, Gate::HumanReview { .. }));
        assert_eq!(out_attack.mode, Mode::HumanReview);

        // Step 3: Human Commander APPROVES the recommended action
        state.apply_action(&prop.recommended_action);

        // Step 4: Step the cycle to verify the executed action
        let out_verified = step(&mut state, &[], 43);

        // Verification succeeded: mode is now RECOVERY, mission score is 1.0!
        assert_eq!(state.verification_passed, Some(true));
        assert_eq!(out_verified.mode, Mode::Recovery);
        assert!((out_verified.mission_score - 1.0).abs() < 1e-4);

        // Incident is now closed
        assert_eq!(out_verified.incidents[0].status, IncidentStatus::Closed);
    }
}
