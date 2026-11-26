use aeris_types::{Capability, EntityId, LinkId};
use petgraph::Direction::{Incoming, Outgoing};
use petgraph::graph::{DiGraph, EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    DependsOn,
    CommunicatesWith,
    RoutesThrough,
    Observes,
    Supports,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Traversal {
    Forward,
    Reverse,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeAttrs {
    pub id: EntityId,
    pub criticality: f64,
    pub capability: Option<Capability>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeAttrs {
    pub kind: EdgeKind,
    pub link_id: Option<LinkId>,
    pub availability: f64,
    pub latency_ms: f64,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    DuplicateEntity(EntityId),
    UnknownEntity(EntityId),
    MissingEndpoint { from: EntityId, to: EntityId },
    MissingLinkId { kind: EdgeKind },
    UnexpectedLinkId { kind: EdgeKind },
}

pub type OperationalGraph = DiGraph<NodeAttrs, EdgeAttrs>;

#[derive(Debug, Clone)]
pub struct GraphModel {
    graph: OperationalGraph,
    nodes: HashMap<EntityId, NodeIndex>,
}

impl Default for GraphModel {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphModel {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            nodes: HashMap::new(),
        }
    }

    pub fn add_node(&mut self, node: NodeAttrs) -> Result<NodeIndex, GraphError> {
        if self.nodes.contains_key(&node.id) {
            return Err(GraphError::DuplicateEntity(node.id));
        }

        let index = self.graph.add_node(node.clone());
        self.nodes.insert(node.id, index);
        Ok(index)
    }

    pub fn add_edge(
        &mut self,
        from: &EntityId,
        to: &EntityId,
        edge: EdgeAttrs,
    ) -> Result<EdgeIndex, GraphError> {
        let link_required = matches!(
            edge.kind,
            EdgeKind::CommunicatesWith | EdgeKind::RoutesThrough
        );

        if link_required && edge.link_id.is_none() {
            return Err(GraphError::MissingLinkId { kind: edge.kind });
        }

        if !link_required && edge.link_id.is_some() {
            return Err(GraphError::UnexpectedLinkId { kind: edge.kind });
        }

        let from_index = *self
            .nodes
            .get(from)
            .ok_or_else(|| GraphError::UnknownEntity(from.clone()))?;
        let to_index = *self
            .nodes
            .get(to)
            .ok_or_else(|| GraphError::UnknownEntity(to.clone()))?;

        Ok(self.graph.add_edge(from_index, to_index, edge))
    }

    pub fn node_index(&self, id: &EntityId) -> Option<NodeIndex> {
        self.nodes.get(id).copied()
    }

    pub fn reach(
        &self,
        from: &EntityId,
        kind: EdgeKind,
        traversal: Traversal,
    ) -> Result<HashSet<EntityId>, GraphError> {
        let start = self
            .node_index(from)
            .ok_or_else(|| GraphError::UnknownEntity(from.clone()))?;
        let direction = match traversal {
            Traversal::Forward => Outgoing,
            Traversal::Reverse => Incoming,
        };

        let mut queue = VecDeque::new();
        let mut seen_nodes = HashSet::new();
        let mut reached = HashSet::new();

        seen_nodes.insert(start);
        queue.push_back(start);

        while let Some(current) = queue.pop_front() {
            let mut next_nodes = Vec::new();

            if kind == EdgeKind::CommunicatesWith {
                for edge in self.graph.edges_directed(current, Outgoing) {
                    if edge.weight().kind == kind {
                        next_nodes.push(edge.target());
                    }
                }
                for edge in self.graph.edges_directed(current, Incoming) {
                    if edge.weight().kind == kind {
                        next_nodes.push(edge.source());
                    }
                }
            } else {
                for edge in self.graph.edges_directed(current, direction) {
                    if edge.weight().kind != kind {
                        continue;
                    }

                    let next = match direction {
                        Outgoing => edge.target(),
                        Incoming => edge.source(),
                    };
                    next_nodes.push(next);
                }
            }

            for next in next_nodes {
                if !seen_nodes.insert(next) {
                    continue;
                }

                let id = self.graph[next].id.clone();
                if id.as_str() != from.as_str() {
                    reached.insert(id);
                }
                queue.push_back(next);
            }
        }

        Ok(reached)
    }

    pub fn dependencies(&self, from: &EntityId) -> Result<HashSet<EntityId>, GraphError> {
        self.reach(from, EdgeKind::DependsOn, Traversal::Forward)
    }

    pub fn dependents(&self, from: &EntityId) -> Result<HashSet<EntityId>, GraphError> {
        self.reach(from, EdgeKind::DependsOn, Traversal::Reverse)
    }

    pub fn upstream(&self, from: &EntityId) -> Result<HashSet<EntityId>, GraphError> {
        self.reach(from, EdgeKind::RoutesThrough, Traversal::Reverse)
    }

    pub fn downstream(&self, from: &EntityId) -> Result<HashSet<EntityId>, GraphError> {
        self.reach(from, EdgeKind::RoutesThrough, Traversal::Forward)
    }

    pub fn neighbors(
        &self,
        from: &EntityId,
        kind: EdgeKind,
    ) -> Result<HashSet<EntityId>, GraphError> {
        self.reach(from, kind, Traversal::Forward)
    }

    pub fn blast_radius(
        &self,
        from: &EntityId,
        kind: EdgeKind,
        traversal: Traversal,
    ) -> Result<(f64, f64), GraphError> {
        let reached = self.reach(from, kind, traversal)?;
        let raw: f64 = reached
            .iter()
            .filter_map(|id| self.node_index(id))
            .map(|index| self.graph[index].criticality)
            .sum();
        let total: f64 = self.graph.node_weights().map(|node| node.criticality).sum();
        let normalized = if total == 0.0 { 0.0 } else { raw / total };
        Ok((raw, normalized))
    }

    pub fn communication_score<F>(
        &self,
        source: &EntityId,
        target: &EntityId,
        active_nodes: &HashSet<EntityId>,
        trust: F,
    ) -> Result<f64, GraphError>
    where
        F: Fn(&EntityId) -> f64,
    {
        let source_index = self
            .node_index(source)
            .ok_or_else(|| GraphError::UnknownEntity(source.clone()))?;
        let target_index = self
            .node_index(target)
            .ok_or_else(|| GraphError::UnknownEntity(target.clone()))?;

        if !active_nodes.contains(source) || !active_nodes.contains(target) {
            return Ok(0.0);
        }

        let mut best = 0.0;
        let mut visited = HashSet::new();
        visited.insert(source_index);

        self.communication_dfs(
            source_index,
            target_index,
            active_nodes,
            &trust,
            &mut visited,
            1.0,
            1.0,
            &mut best,
        );

        Ok(best.clamp(0.0, 1.0))
    }

    pub fn observation_score<F, G>(
        &self,
        active_nodes: &HashSet<EntityId>,
        coverage: F,
        sensor_trust: G,
    ) -> f64
    where
        F: Fn(&EntityId) -> f64,
        G: Fn(&EntityId) -> f64,
    {
        let mut miss = 1.0;
        let mut seen = false;

        for id in active_nodes {
            if self.node_index(id).is_none() {
                continue;
            }

            seen = true;
            let covered = coverage(id).clamp(0.0, 1.0) * sensor_trust(id).clamp(0.0, 1.0);
            miss *= 1.0 - covered;
        }

        if !seen {
            return 0.0;
        }

        (1.0 - miss).clamp(0.0, 1.0)
    }

    pub fn navigation_score<F>(
        &self,
        active_nodes: &HashSet<EntityId>,
        position_variance: F,
        sigma_max: f64,
    ) -> f64
    where
        F: Fn(&EntityId) -> f64,
    {
        if sigma_max <= 0.0 {
            return 0.0;
        }

        let mut score: f64 = 1.0;
        let mut seen = false;

        for id in active_nodes {
            if self.node_index(id).is_none() {
                continue;
            }

            seen = true;
            let deviation = position_variance(id).max(0.0).sqrt();
            score = score.min(1.0 - (deviation / sigma_max).min(1.0));
        }

        if !seen {
            return 0.0;
        }

        score.clamp(0.0, 1.0)
    }

    // Recursive DFS state is threaded explicitly; bundling it would only obscure the recursion.
    #[allow(clippy::too_many_arguments)]
    fn communication_dfs<F>(
        &self,
        current: NodeIndex,
        target: NodeIndex,
        active_nodes: &HashSet<EntityId>,
        trust: &F,
        visited: &mut HashSet<NodeIndex>,
        availability: f64,
        interior_trust: f64,
        best: &mut f64,
    ) where
        F: Fn(&EntityId) -> f64,
    {
        if current == target {
            let score = availability * interior_trust;
            if score > *best {
                *best = score;
            }
            return;
        }

        let edges: Vec<(NodeIndex, f64)> = self
            .graph
            .edges_directed(current, Outgoing)
            .filter(|edge| {
                let weight = edge.weight();
                weight.kind == EdgeKind::RoutesThrough && weight.active
            })
            .map(|edge| (edge.target(), edge.weight().availability.clamp(0.0, 1.0)))
            .collect();

        for (next, edge_availability) in edges {
            if visited.contains(&next) {
                continue;
            }

            let id = &self.graph[next].id;
            if !active_nodes.contains(id) {
                continue;
            }

            let next_trust = if next == target {
                1.0
            } else {
                trust(id).clamp(0.0, 1.0)
            };

            visited.insert(next);
            self.communication_dfs(
                next,
                target,
                active_nodes,
                trust,
                visited,
                availability * edge_availability,
                interior_trust * next_trust,
                best,
            );
            visited.remove(&next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(name: &str, criticality: f64) -> NodeAttrs {
        NodeAttrs {
            id: name.into(),
            criticality,
            capability: None,
        }
    }

    fn edge(kind: EdgeKind, link_id: Option<&str>, availability: f64, active: bool) -> EdgeAttrs {
        EdgeAttrs {
            kind,
            link_id: link_id.map(|id| id.to_string()),
            availability,
            latency_ms: 1.0,
            active,
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn add_node_and_edge_validate_contract() {
        let mut graph = GraphModel::new();
        let a = "node-a".to_string();
        let b = "node-b".to_string();

        graph.add_node(entity(&a, 0.0)).unwrap();
        graph.add_node(entity(&b, 0.0)).unwrap();

        assert_eq!(
            graph.add_node(entity(&a, 0.0)).unwrap_err(),
            GraphError::DuplicateEntity(a.clone())
        );

        assert_eq!(
            graph
                .add_edge(&a, &b, edge(EdgeKind::RoutesThrough, None, 0.99, true))
                .unwrap_err(),
            GraphError::MissingLinkId {
                kind: EdgeKind::RoutesThrough,
            }
        );

        assert_eq!(
            graph
                .add_edge(
                    &a,
                    &b,
                    edge(EdgeKind::DependsOn, Some("link-l1"), 0.99, true)
                )
                .unwrap_err(),
            GraphError::UnexpectedLinkId {
                kind: EdgeKind::DependsOn,
            }
        );

        graph
            .add_edge(&a, &b, edge(EdgeKind::DependsOn, None, 0.99, true))
            .unwrap();
    }

    #[test]
    fn depends_on_orientation_terminates_on_cycles() {
        let mut graph = GraphModel::new();
        let a = "node-a".to_string();
        let b = "node-b".to_string();
        let c = "node-c".to_string();

        for id in [&a, &b, &c] {
            graph.add_node(entity(id, 0.0)).unwrap();
        }

        for (from, to) in [(&a, &b), (&b, &c), (&c, &a)] {
            graph
                .add_edge(from, to, edge(EdgeKind::DependsOn, None, 1.0, true))
                .unwrap();
        }

        let dependencies = graph.dependencies(&b).unwrap();
        assert_eq!(dependencies.len(), 2);
        assert!(dependencies.contains(&a));
        assert!(dependencies.contains(&c));

        let dependents = graph.dependents(&b).unwrap();
        assert_eq!(dependents.len(), 2);
        assert!(dependents.contains(&a));
        assert!(dependents.contains(&c));
    }

    #[test]
    fn communicates_with_is_symmetric_for_neighbors() {
        let mut graph = GraphModel::new();
        let a = "node-a".to_string();
        let b = "node-b".to_string();

        graph.add_node(entity(&a, 0.0)).unwrap();
        graph.add_node(entity(&b, 0.0)).unwrap();
        graph
            .add_edge(
                &a,
                &b,
                edge(EdgeKind::CommunicatesWith, Some("link-l1"), 0.99, true),
            )
            .unwrap();

        let from_b = graph.neighbors(&b, EdgeKind::CommunicatesWith).unwrap();
        assert!(from_b.contains(&a));
        assert!(!from_b.contains(&b));
    }

    #[test]
    fn blast_radius_excludes_start_and_normalizes() {
        let mut graph = GraphModel::new();
        let a = "node-a".to_string();
        let b = "node-b".to_string();
        let c = "node-c".to_string();
        let d = "node-d".to_string();

        for (id, criticality) in [(&a, 0.2), (&b, 0.4), (&c, 0.3), (&d, 0.1)] {
            graph.add_node(entity(id, criticality)).unwrap();
        }

        graph
            .add_edge(&a, &b, edge(EdgeKind::DependsOn, None, 1.0, true))
            .unwrap();
        graph
            .add_edge(&c, &b, edge(EdgeKind::DependsOn, None, 1.0, true))
            .unwrap();

        let (raw, normalized) = graph
            .blast_radius(&b, EdgeKind::DependsOn, Traversal::Reverse)
            .unwrap();
        assert_close(raw, 0.5);
        assert_close(normalized, 0.5);

        let (raw_forward, normalized_forward) = graph
            .blast_radius(&b, EdgeKind::DependsOn, Traversal::Forward)
            .unwrap();
        assert_close(raw_forward, 0.0);
        assert_close(normalized_forward, 0.0);
    }

    #[test]
    fn communication_score_uses_active_routes_and_interior_trust() {
        let mut graph = GraphModel::new();
        let a = "node-a".to_string();
        let b = "node-b".to_string();
        let c = "node-c".to_string();

        for id in [&a, &b, &c] {
            graph.add_node(entity(id, 0.0)).unwrap();
        }

        graph
            .add_edge(
                &a,
                &b,
                edge(EdgeKind::RoutesThrough, Some("link-l1"), 0.99, true),
            )
            .unwrap();
        graph
            .add_edge(
                &b,
                &c,
                edge(EdgeKind::RoutesThrough, Some("link-l1"), 0.99, true),
            )
            .unwrap();
        graph
            .add_edge(
                &a,
                &c,
                edge(EdgeKind::RoutesThrough, Some("link-l2"), 0.93, false),
            )
            .unwrap();

        let trust = |id: &EntityId| {
            if id.as_str() == "node-b" { 0.92 } else { 1.0 }
        };

        let all_active: HashSet<EntityId> = [a.clone(), b.clone(), c.clone()].into();
        assert_close(
            graph
                .communication_score(&a, &c, &all_active, trust)
                .unwrap(),
            0.99 * 0.99 * 0.92,
        );

        let without_b: HashSet<EntityId> = [a.clone(), c.clone()].into();
        assert_close(
            graph
                .communication_score(&a, &c, &without_b, trust)
                .unwrap(),
            0.0,
        );

        graph
            .add_edge(
                &a,
                &c,
                edge(EdgeKind::RoutesThrough, Some("link-l2-active"), 0.93, true),
            )
            .unwrap();
        assert_close(
            graph
                .communication_score(&a, &c, &without_b, trust)
                .unwrap(),
            0.93,
        );
        assert_close(
            graph
                .communication_score(&a, &c, &all_active, trust)
                .unwrap(),
            0.93,
        );
    }

    #[test]
    fn observation_and_navigation_scores_follow_equations() {
        let mut graph = GraphModel::new();
        let a = "node-a".to_string();
        let b = "node-b".to_string();
        let c = "node-c".to_string();

        for id in [&a, &b, &c] {
            graph.add_node(entity(id, 0.0)).unwrap();
        }

        let active: HashSet<EntityId> = [a.clone(), b.clone(), c.clone()].into();

        let observation = graph.observation_score(&active, |_| 0.6, |_| 0.9);
        assert_close(observation, 1.0 - 0.46_f64.powf(3.0));

        let navigation = graph.navigation_score(
            &active,
            |id| match id.as_str() {
                "node-a" => 100.0,
                "node-b" => 25.0,
                _ => 400.0,
            },
            30.0,
        );
        assert_close(navigation, 1.0 - 400.0_f64.sqrt() / 30.0);
    }
}
