//! AERIS mission model: capabilities, mission score, counterfactual
//! criticality, blast radius and risk (docs/v0.1/04-mathematical-model.md §6.7–6.9).
//!
//! Pure functions over a [`Topology`]: AERIS's *own view* of the world (which
//! nodes are active, which links are on, the estimated trust and localisation
//! variance of each node). No I/O, no randomness, no ground truth. Every
//! constant (coverage, camera quality, minima, σ_max) is supplied by the caller
//! from `docs/spec/world.toml`.
//!
//! Owner: KESIVARSHINI (scaffolded by KISHANTH during G4 integration).

use aeris_graph::{EdgeAttrs, EdgeKind, GraphModel, NodeAttrs, Traversal};
use aeris_types::{Action, Capability, EntityId, LinkId};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Required minimum level per capability, `c_k^min`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Minima {
    pub navigation: f64,
    pub communication: f64,
    pub observation: f64,
}

impl Minima {
    pub fn get(&self, cap: Capability) -> f64 {
        match cap {
            Capability::Navigation => self.navigation,
            Capability::Communication => self.communication,
            Capability::Observation => self.observation,
        }
    }

    fn set(&mut self, cap: Capability, value: f64) {
        match cap {
            Capability::Navigation => self.navigation = value,
            Capability::Communication => self.communication = value,
            Capability::Observation => self.observation = value,
        }
    }
}

/// AERIS's estimate of one node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeView {
    pub id: EntityId,
    /// False once the node is isolated: it provides nothing and relays nothing.
    pub active: bool,
    /// Aggregated node trust `T_v` (§6.6); a relay's trust scales every path through it.
    pub trust: f64,
    /// Fraction of the area the node's camera covers, `cov_v`.
    pub coverage: f64,
    /// Observation reliability `T_v^obs` (camera quality × camera sensor trust).
    pub obs_trust: f64,
    /// Kalman position variance `P_v` in m² (§6.3).
    pub position_variance: f64,
}

/// One communication link between the mission endpoints, possibly relayed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkView {
    pub id: LinkId,
    pub endpoints: [EntityId; 2],
    /// Interior relay nodes, in order from `endpoints[0]` to `endpoints[1]`.
    pub via: Vec<EntityId>,
    /// Availability of each hop (`via.len() + 1` entries).
    pub hop_availability: Vec<f64>,
    pub active: bool,
}

impl LinkView {
    fn hops(&self) -> Vec<(EntityId, EntityId, f64)> {
        let mut chain = Vec::with_capacity(self.via.len() + 2);
        chain.push(self.endpoints[0].clone());
        chain.extend(self.via.iter().cloned());
        chain.push(self.endpoints[1].clone());
        chain
            .windows(2)
            .enumerate()
            .map(|(i, w)| {
                let avail = self.hop_availability.get(i).copied().unwrap_or(0.0);
                (w[0].clone(), w[1].clone(), avail)
            })
            .collect()
    }
}

/// Capability levels `c_k` in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub navigation: f64,
    pub communication: f64,
    pub observation: f64,
}

impl Capabilities {
    pub fn get(&self, cap: Capability) -> f64 {
        match cap {
            Capability::Navigation => self.navigation,
            Capability::Communication => self.communication,
            Capability::Observation => self.observation,
        }
    }

    pub fn as_vec(&self) -> Vec<(Capability, f64)> {
        ALL_CAPS.iter().map(|c| (*c, self.get(*c))).collect()
    }
}

pub const ALL_CAPS: [Capability; 3] = [
    Capability::Navigation,
    Capability::Communication,
    Capability::Observation,
];

/// AERIS's view of the operational world.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Topology {
    pub nodes: Vec<NodeView>,
    pub links: Vec<LinkView>,
    /// Communication is measured between these two nodes.
    pub endpoints: [EntityId; 2],
    pub minima: Minima,
    /// Minima that apply to a capability after `degrade(k)`.
    pub degraded_minima: Minima,
    /// `σ_max` in metres for navigation.
    pub sigma_max: f64,
}

const MISSION_ID: &str = "mission";

fn cap_id(cap: Capability) -> EntityId {
    format!("capability/{cap:?}").to_lowercase()
}

impl Topology {
    pub fn node(&self, id: &str) -> Option<&NodeView> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn link(&self, id: &str) -> Option<&LinkView> {
        self.links.iter().find(|l| l.id == id)
    }

    fn active_set(&self) -> HashSet<EntityId> {
        self.nodes
            .iter()
            .filter(|n| n.active)
            .map(|n| n.id.clone())
            .collect()
    }

    /// Links that are switched on and whose endpoints and relays are all active.
    pub fn usable_links(&self) -> impl Iterator<Item = &LinkView> {
        let active = self.active_set();
        self.links.iter().filter(move |l| {
            l.active
                && l.endpoints.iter().all(|e| active.contains(e))
                && l.via.iter().all(|v| active.contains(v))
        })
    }

    /// True when the node relays at least one usable link.
    pub fn is_active_relay(&self, node: &str) -> bool {
        self.usable_links().any(|l| l.via.iter().any(|v| v == node))
    }

    /// Operational graph (§6.7): RoutesThrough hops of every link, and
    /// DependsOn edges capability → provider and mission → capability.
    ///
    /// Criticalities on the graph: entities carry the supplied `C_i`;
    /// capabilities and the mission carry 1, because removing any required
    /// capability sets `M(G) = 0` (the same counterfactual definition).
    pub fn graph(&self, criticality: &dyn Fn(&str) -> f64) -> GraphModel {
        let mut g = GraphModel::new();
        for n in &self.nodes {
            let _ = g.add_node(NodeAttrs {
                id: n.id.clone(),
                criticality: criticality(&n.id),
                capability: None,
            });
        }
        for cap in ALL_CAPS {
            let _ = g.add_node(NodeAttrs {
                id: cap_id(cap),
                criticality: 1.0,
                capability: Some(cap),
            });
        }
        let _ = g.add_node(NodeAttrs {
            id: MISSION_ID.into(),
            criticality: 1.0,
            capability: None,
        });

        for link in &self.links {
            for (a, b, avail) in link.hops() {
                let _ = g.add_edge(
                    &a,
                    &b,
                    EdgeAttrs {
                        kind: EdgeKind::RoutesThrough,
                        link_id: Some(link.id.clone()),
                        availability: avail,
                        latency_ms: 0.0,
                        active: link.active,
                    },
                );
            }
        }

        let depends = |g: &mut GraphModel, from: &EntityId, to: &EntityId| {
            let _ = g.add_edge(
                from,
                to,
                EdgeAttrs {
                    kind: EdgeKind::DependsOn,
                    link_id: None,
                    availability: 1.0,
                    latency_ms: 0.0,
                    active: true,
                },
            );
        };

        // Communication depends on the endpoints and on every relay of a usable path.
        let comm = cap_id(Capability::Communication);
        let mut comm_providers: Vec<EntityId> = self.endpoints.to_vec();
        for l in self.usable_links() {
            comm_providers.extend(l.via.iter().cloned());
        }
        comm_providers.sort();
        comm_providers.dedup();
        for p in &comm_providers {
            depends(&mut g, &comm, p);
        }
        // Observation and navigation depend on every active node.
        for n in self.nodes.iter().filter(|n| n.active) {
            depends(&mut g, &cap_id(Capability::Observation), &n.id);
            depends(&mut g, &cap_id(Capability::Navigation), &n.id);
        }
        for cap in ALL_CAPS {
            depends(&mut g, &MISSION_ID.into(), &cap_id(cap));
        }
        g
    }

    /// Capability levels (§6.7):
    /// - `c_comm = max over usable paths of Π avail_e · Π_{interior} T_v`
    /// - `c_obs  = 1 − Π_{active} (1 − cov_v · T_v^obs)`
    /// - `c_nav  = min_{active} (1 − min(1, √P_v / σ_max))`
    pub fn capabilities(&self) -> Capabilities {
        let g = self.graph(&|_| 0.0);
        let active = self.active_set();
        let trust = |id: &EntityId| self.node(id).map_or(0.0, |n| n.trust);
        let communication = g
            .communication_score(&self.endpoints[0], &self.endpoints[1], &active, trust)
            .unwrap_or(0.0);
        let observation = g.observation_score(
            &active,
            |id| self.node(id).map_or(0.0, |n| n.coverage),
            |id| self.node(id).map_or(0.0, |n| n.obs_trust),
        );
        let navigation = g.navigation_score(
            &active,
            |id| self.node(id).map_or(f64::INFINITY, |n| n.position_variance),
            self.sigma_max,
        );
        Capabilities {
            navigation,
            communication,
            observation,
        }
    }

    /// The counterfactual topology after action `a` (structural effect only;
    /// outcomes over time come from rollouts).
    pub fn apply(&self, action: &Action) -> Topology {
        let mut t = self.clone();
        t.apply_in_place(action);
        t
    }

    fn apply_in_place(&mut self, action: &Action) {
        match action {
            Action::Monitor | Action::Restrict(_) | Action::Failover { .. } => {}
            Action::Isolate(id) => {
                if let Some(n) = self.nodes.iter_mut().find(|n| &n.id == id) {
                    n.active = false;
                }
            }
            Action::Recover(id) => {
                if let Some(n) = self.nodes.iter_mut().find(|n| &n.id == id) {
                    n.active = true;
                }
            }
            Action::Reroute(id) => {
                if let Some(l) = self.links.iter_mut().find(|l| &l.id == id) {
                    l.active = true;
                }
            }
            Action::Degrade(cap) => {
                let reduced = self.degraded_minima.get(*cap);
                self.minima.set(*cap, reduced);
            }
            Action::Combo(actions) => {
                for a in actions {
                    self.apply_in_place(a);
                }
            }
        }
    }

    /// The topology with node `id` removed (`G ∖ v_i`).
    pub fn without(&self, id: &str) -> Topology {
        self.apply(&Action::Isolate(id.to_string()))
    }

    /// Same topology with every trust, availability and localisation made
    /// perfect: whether a provider exists at all, regardless of how much we
    /// believe it. Used by the hard invariants.
    pub fn structural(&self) -> Topology {
        let mut t = self.clone();
        for n in &mut t.nodes {
            n.trust = 1.0;
            n.obs_trust = 1.0;
            n.position_variance = 0.0;
        }
        for l in &mut t.links {
            l.hop_availability.fill(1.0);
        }
        t
    }

    /// Continuous mission score `M(G) = Π_k min(1, c_k / c_k^min)`.
    pub fn mission_score(&self) -> f64 {
        mission_score(&self.capabilities(), &self.minima)
    }

    /// Counterfactual criticality `C_i = (M(G) − M(G∖v_i)) / M(G)`, 0 when `M(G) = 0`.
    pub fn criticality(&self, id: &str) -> f64 {
        let m_full = self.mission_score();
        if m_full <= 0.0 {
            return 0.0;
        }
        let m_removed = self.without(id).mission_score();
        ((m_full - m_removed) / m_full).clamp(0.0, 1.0)
    }

    /// Normalised blast radius `B̂(v) = Σ_{u ∈ Reach(v)} C_u / Σ_{u ∈ V} C_u`,
    /// where Reach(v) is everything that depends on v.
    pub fn blast_radius(&self, id: &str) -> f64 {
        let g = self.graph(&|n| self.criticality(n));
        g.blast_radius(&id.to_string(), EdgeKind::DependsOn, Traversal::Reverse)
            .map_or(0.0, |(_, normalised)| normalised)
    }
}

/// `M(G) = Π_k min(1, c_k / c_k^min)`.
pub fn mission_score(caps: &Capabilities, minima: &Minima) -> f64 {
    ALL_CAPS
        .iter()
        .map(|c| {
            let min = minima.get(*c);
            if min <= 0.0 {
                1.0
            } else {
                (caps.get(*c) / min).min(1.0)
            }
        })
        .product()
}

/// `MissionSuccess = ∧_k (c_k ≥ c_k^min)`.
pub fn mission_success(caps: &Capabilities, minima: &Minima) -> bool {
    ALL_CAPS.iter().all(|c| caps.get(*c) >= minima.get(*c))
}

/// Noisy-OR impact `Impact = 1 − (1 − C)(1 − B̂)`.
pub fn impact(criticality: f64, blast_radius: f64) -> f64 {
    (1.0 - (1.0 - criticality.clamp(0.0, 1.0)) * (1.0 - blast_radius.clamp(0.0, 1.0)))
        .clamp(0.0, 1.0)
}

/// `R = P(fail | E) · Impact`, with `P(fail | E) = 1 − P(normal | E)`.
pub fn risk(p_normal: f64, impact: f64) -> f64 {
    ((1.0 - p_normal.clamp(0.0, 1.0)) * impact).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMA: Minima = Minima {
        navigation: 0.85,
        communication: 0.90,
        observation: 0.80,
    };

    fn node(id: &str, trust: f64) -> NodeView {
        NodeView {
            id: id.into(),
            active: true,
            trust,
            coverage: 0.6,
            obs_trust: 0.948,
            position_variance: 2.1 * 2.1,
        }
    }

    /// The frozen S4 world (docs/spec/world.toml): L1 A–B–C, L2 A–C standby.
    fn s4_world(t_b: f64) -> Topology {
        Topology {
            nodes: vec![
                node("node-a", 0.9),
                node("node-b", t_b),
                node("node-c", 0.9),
            ],
            links: vec![
                LinkView {
                    id: "link-l1".into(),
                    endpoints: ["node-a".into(), "node-c".into()],
                    via: vec!["node-b".into()],
                    hop_availability: vec![0.99, 0.99],
                    active: true,
                },
                LinkView {
                    id: "link-l2".into(),
                    endpoints: ["node-a".into(), "node-c".into()],
                    via: vec![],
                    hop_availability: vec![0.93],
                    active: false,
                },
            ],
            endpoints: ["node-a".into(), "node-c".into()],
            minima: MINIMA,
            degraded_minima: Minima {
                navigation: 0.70,
                communication: 0.25,
                observation: 0.50,
            },
            sigma_max: 30.0,
        }
    }

    fn assert_close(actual: f64, expected: f64, what: &str) {
        assert!(
            (actual - expected).abs() < 5e-4,
            "{what}: expected {expected:.4}, got {actual:.4}"
        );
    }

    /// §6.8 worked table, row 1: all healthy, T_B = 0.92, L1 active.
    #[test]
    fn healthy_world_meets_every_minimum() {
        let w = s4_world(0.92);
        let caps = w.capabilities();
        assert_close(caps.navigation, 0.930, "nav");
        assert_close(caps.communication, 0.99 * 0.99 * 0.92, "comm");
        assert_close(caps.observation, 0.920, "obs");
        assert_close(w.mission_score(), 1.0, "M(G)");
        assert!(mission_success(&caps, &w.minima));
    }

    /// Row 2: B suspicious (T_B = 0.565) drags communication below its minimum.
    #[test]
    fn distrusted_relay_breaks_communication() {
        let w = s4_world(0.565);
        let caps = w.capabilities();
        assert_close(caps.communication, 0.9801 * 0.565, "comm");
        assert_close(w.mission_score(), 0.9801 * 0.565 / 0.90, "M(G)");
        assert!(!mission_success(&caps, &w.minima));
    }

    /// Row 3: B removed with L2 still off cuts communication to zero.
    #[test]
    fn removing_sole_relay_zeroes_communication() {
        let w = s4_world(0.92).without("node-b");
        let caps = w.capabilities();
        assert_eq!(caps.communication, 0.0);
        assert_close(
            caps.observation,
            1.0 - (1.0 - 0.6 * 0.948f64).powi(2),
            "obs",
        );
        assert_eq!(w.mission_score(), 0.0);
    }

    /// Row 4: isolate B and activate L2 restores the mission.
    #[test]
    fn isolate_and_reroute_restores_mission() {
        let a = Action::Combo(vec![
            Action::Isolate("node-b".into()),
            Action::Reroute("link-l2".into()),
        ]);
        let w = s4_world(0.565).apply(&a);
        let caps = w.capabilities();
        assert_close(caps.communication, 0.930, "comm");
        assert_close(
            caps.observation,
            1.0 - (1.0 - 0.6 * 0.948f64).powi(2),
            "obs",
        );
        assert_close(w.mission_score(), 1.0, "M(G)");
    }

    /// C_B = 1 while B is the only active relay; 0 once L2 is on.
    #[test]
    fn criticality_is_counterfactual() {
        assert_close(
            s4_world(0.92).criticality("node-b"),
            1.0,
            "C_B with L1 only",
        );
        let rerouted = s4_world(0.92).apply(&Action::Reroute("link-l2".into()));
        assert_close(rerouted.criticality("node-b"), 0.0, "C_B with L2 on");
        assert_eq!(s4_world(0.92).without("node-b").criticality("node-a"), 0.0);
    }

    #[test]
    fn relay_detection_follows_usable_links() {
        let w = s4_world(0.92);
        assert!(w.is_active_relay("node-b"));
        assert!(!w.is_active_relay("node-a"));
        assert!(!w.without("node-b").is_active_relay("node-b"));
    }

    /// B̂ is the share of total criticality that depends on the node.
    #[test]
    fn blast_radius_counts_dependents() {
        // Dependents of B: three capabilities + mission (criticality 1 each);
        // total = C_A + C_B + C_C + 4 = 7.
        assert_close(s4_world(0.92).blast_radius("node-b"), 4.0 / 7.0, "B̂(B)");
        assert_eq!(s4_world(0.92).blast_radius("unknown"), 0.0);
    }

    #[test]
    fn degrade_lowers_the_minimum() {
        let w = s4_world(0.565).apply(&Action::Degrade(Capability::Communication));
        assert_close(w.minima.communication, 0.25, "degraded min");
        assert_close(w.mission_score(), 1.0, "M(G)");
    }

    /// §6.9: R_B = 0.987 for P(normal) = 0.013 and impact 1.
    #[test]
    fn risk_and_impact() {
        assert_close(impact(1.0, 0.2), 1.0, "impact(1, .2)");
        assert_close(impact(0.0, 0.0), 0.0, "impact(0, 0)");
        assert_close(impact(0.5, 0.5), 0.75, "impact(.5, .5)");
        assert_close(risk(0.013, 1.0), 0.987, "R_B");
        assert_close(risk(1.0, 1.0), 0.0, "risk of a normal node");
    }

    #[test]
    fn navigation_degrades_with_variance() {
        let mut w = s4_world(0.92);
        w.nodes[0].position_variance = 30.0 * 30.0;
        assert_eq!(w.capabilities().navigation, 0.0);
    }

    #[test]
    fn structural_view_ignores_belief() {
        let caps = s4_world(0.1).structural().capabilities();
        assert_eq!(caps.communication, 1.0);
        assert_eq!(caps.navigation, 1.0);
    }
}
