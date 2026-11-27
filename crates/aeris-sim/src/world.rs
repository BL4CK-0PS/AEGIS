//! The simulated world: `S(t+1) = F(S(t), a, W(t))` at `tick_hz`.
//!
//! # Ground truth vs observations
//!
//! A [`World`] holds the true state, including who is compromised and which
//! sensors are failing. That truth leaves the world through exactly two doors:
//!
//! * [`World::step`] returns [`Observation`]s: `O = h(S) + V + A_adv`. They
//!   never contain labels; a compromised node simply *behaves* differently.
//! * [`World::ground_truth`] is for the evaluator only (precision/recall,
//!   mission success). Nothing on the detection path may call it, and the
//!   simulator daemon never publishes it.
//!
//! [`World::observe`] returns what a real control plane can measure
//! (topology, link measurements) and is what execution verification uses.
//!
//! # Randomness
//!
//! Each world owns two ChaCha8 streams derived from the seed: `env` (W:
//! motion noise, radio fades, attacker spread — everything that changes the
//! truth) and `obs` (V: measurement noise). Every tick draws a fixed number of
//! values from `env` regardless of state, so two forks that take different
//! actions still see the same disturbances (common random numbers).

use crate::config::{CapabilityMinima, WorldConfig};
use crate::rng::{normal_pair, stream, unit};
use crate::scenario::{ActuatorBehavior, AttackerProfile, Fault, Scenario};
use crate::{Capabilities, SimError};
use aeris_types::Capability;
use aeris_wire::{CommandOp, LinkObservation, NodeMode, SensorKind, WirePayload};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Producer version stamped into provenance.
pub const PRODUCER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How an observation leaves the node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// A route to the uplink exists: send now.
    Uplink,
    /// No route (partition): buffer and replay later (S7).
    Partitioned,
    /// The node is isolated: its traffic is not carried at all.
    Quarantined,
}

/// One reading produced by a node in one tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// Tick the reading belongs to.
    pub tick: u64,
    /// Sim time of the reading, ms since sim start.
    pub sim_time_ms: i64,
    /// Producing node (event source).
    pub node: String,
    /// Payload.
    pub payload: WirePayload,
    /// Producer confidence.
    pub confidence: f64,
    /// Producer identity, e.g. `aeris-sim/node-b/gps`.
    pub producer: String,
    /// Routing outcome.
    pub delivery: Delivery,
}

/// Outcome of applying a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyOutcome {
    /// State changed.
    Applied,
    /// Already in the commanded state.
    AlreadyApplied,
}

/// Evaluator-only view of one node's true state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeTruth {
    /// Node id.
    pub id: String,
    /// Mode.
    pub mode: NodeMode,
    /// Attacker present.
    pub compromised: bool,
    /// Tick the attacker arrived.
    pub compromised_since: Option<u64>,
    /// GPS receiver failed.
    pub gps_failed: bool,
    /// GPS externally spoofed.
    pub gps_spoofed: bool,
    /// Node reports false telemetry.
    pub false_telemetry: bool,
    /// True onboard localisation error, metres.
    pub loc_sigma_m: f64,
}

/// Evaluator-only snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroundTruth {
    /// Tick.
    pub tick: u64,
    /// Per node.
    pub nodes: Vec<NodeTruth>,
    /// True capability levels.
    pub capabilities: Capabilities,
    /// Every capability meets its (possibly degraded) minimum.
    pub mission_ok: bool,
    /// Kinds of faults active now.
    pub active_faults: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Compromise {
    pub(crate) profile: Arc<AttackerProfile>,
    pub(crate) since: u64,
    pub(crate) announced: bool,
}

#[derive(Debug, Clone)]
struct GpsFailure {
    sigma: f64,
    walk: f64,
    bias: [f64; 2],
}

#[derive(Debug, Clone)]
struct GpsSpoof {
    offset: [f64; 2],
    ramp_ticks: u64,
    since: u64,
}

#[derive(Debug, Clone)]
struct FalseTelemetry {
    drift: [f64; 2],
    flatline: bool,
    since: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct NodeState {
    pub(crate) mode: NodeMode,
    pos: [f64; 2],
    vel: [f64; 2],
    pub(crate) compromise: Option<Compromise>,
    false_tel: Option<FalseTelemetry>,
    pub(crate) gps_failure_flag: bool,
    gps_failure: Option<GpsFailure>,
    gps_spoof: Option<GpsSpoof>,
    loc_sigma: f64,
}

#[cfg(test)]
impl NodeState {
    pub(crate) fn pos_for_test(&self) -> [f64; 2] {
        self.pos
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HopState {
    fade_left: u32,
    pub(crate) cut: u32,
    pub(crate) extra_loss: f64,
    extra_latency: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct LinkState {
    pub(crate) active: bool,
    pub(crate) hops: Vec<HopState>,
}

/// The simulated world. Cloning (via [`World::fork`]) yields a fully
/// independent copy: configuration and attacker profiles are shared through
/// `Arc` and are immutable; all mutable state is owned.
#[derive(Debug, Clone)]
pub struct World {
    pub(crate) cfg: Arc<WorldConfig>,
    scenario: Arc<Scenario>,
    fault_active: Vec<bool>,
    pub(crate) tick: u64,
    pub(crate) nodes: Vec<NodeState>,
    pub(crate) links: Vec<LinkState>,
    pub(crate) degraded: [bool; 3],
    accel_sigma_override: Option<f64>,
    actuator_faults: Vec<(String, ActuatorBehavior)>,
    pub(crate) rng_env: ChaCha8Rng,
    rng_obs: ChaCha8Rng,
}

pub(crate) fn cap_index(c: Capability) -> usize {
    match c {
        Capability::Navigation => 0,
        Capability::Communication => 1,
        Capability::Observation => 2,
    }
}

const ALL_CAPS: [Capability; 3] = [
    Capability::Navigation,
    Capability::Communication,
    Capability::Observation,
];

fn routed(mode: NodeMode) -> bool {
    matches!(mode, NodeMode::Active | NodeMode::Restricted)
}

impl World {
    /// A live world running `scenario` with `seed`.
    pub fn new(
        cfg: Arc<WorldConfig>,
        scenario: Arc<Scenario>,
        seed: u64,
    ) -> Result<Self, SimError> {
        cfg.validate()?;
        scenario.validate(&cfg)?;
        let nodes = cfg
            .nodes
            .iter()
            .map(|n| NodeState {
                mode: NodeMode::Active,
                pos: n.position_m,
                vel: n.velocity_mps,
                compromise: None,
                false_tel: None,
                gps_failure_flag: false,
                gps_failure: None,
                gps_spoof: None,
                loc_sigma: 0.0,
            })
            .collect();
        let links = cfg
            .links
            .iter()
            .map(|l| LinkState {
                active: l.active,
                hops: vec![HopState::default(); l.hop_availability.len()],
            })
            .collect();
        let mut w = Self {
            fault_active: vec![false; scenario.faults.len()],
            cfg,
            scenario,
            tick: 0,
            nodes,
            links,
            degraded: [false; 3],
            accel_sigma_override: None,
            actuator_faults: Vec::new(),
            rng_env: stream(seed, "env", 0),
            rng_obs: stream(seed, "obs", 0),
        };
        for i in 0..w.nodes.len() {
            w.nodes[i].loc_sigma = w.fix_sigma(i).unwrap_or(0.0);
        }
        w.sync_faults();
        Ok(w)
    }

    /// A world with no scenario (no injected faults).
    pub fn quiet(cfg: Arc<WorldConfig>, seed: u64) -> Result<Self, SimError> {
        let s = Scenario {
            scenario: crate::scenario::ScenarioMeta {
                id: "quiet".into(),
                name: "no faults".into(),
                description: String::new(),
                duration_s: 86_400.0,
            },
            faults: Vec::new(),
        };
        Self::new(cfg, Arc::new(s), seed)
    }

    /// An independent copy with fresh random streams derived from
    /// `stream_seed`. Changes to the fork never affect `self`.
    pub fn fork(&self, stream_seed: [u8; 32]) -> Self {
        use rand::SeedableRng;
        let mut w = self.clone();
        w.rng_env = ChaCha8Rng::from_seed(stream_seed);
        let mut obs_seed = stream_seed;
        obs_seed[0] ^= 0x5A;
        w.rng_obs = ChaCha8Rng::from_seed(obs_seed);
        w
    }

    /// Configuration.
    pub fn config(&self) -> &Arc<WorldConfig> {
        &self.cfg
    }

    /// Scenario.
    pub fn scenario(&self) -> &Arc<Scenario> {
        &self.scenario
    }

    /// Current tick.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Milliseconds per tick.
    pub fn tick_ms(&self) -> i64 {
        1_000 / i64::from(self.cfg.world.tick_hz)
    }

    /// Sim time of the current tick, ms.
    pub fn sim_time_ms(&self) -> i64 {
        self.tick as i64 * self.tick_ms()
    }

    fn dt(&self) -> f64 {
        1.0 / f64::from(self.cfg.world.tick_hz)
    }

    // ------------------------------------------------------------------
    // Faults
    // ------------------------------------------------------------------

    fn fault_should_be_active(&self, f: &Fault) -> bool {
        let hz = f64::from(self.cfg.world.tick_hz);
        let (s, e) = f.window();
        let start = (s * hz).round() as u64;
        let end = e.map(|e| (e * hz).round() as u64);
        self.tick >= start && end.is_none_or(|e| self.tick < e)
    }

    fn sync_faults(&mut self) {
        let scenario = Arc::clone(&self.scenario);
        for (i, f) in scenario.faults.iter().enumerate() {
            let should = self.fault_should_be_active(f);
            if should != self.fault_active[i] {
                self.fault_active[i] = should;
                self.set_fault(f, should);
            }
        }
    }

    fn hop_indices(&self, a: &str, b: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (li, l) in self.cfg.links.iter().enumerate() {
            for (hi, w) in l.path().windows(2).enumerate() {
                if (w[0] == a && w[1] == b) || (w[0] == b && w[1] == a) {
                    out.push((li, hi));
                }
            }
        }
        out
    }

    fn set_fault(&mut self, f: &Fault, on: bool) {
        let tick = self.tick;
        let idx = |w: &Self, n: &str| w.cfg.node_index(n).expect("validated node");
        match f {
            Fault::GpsFailure {
                node,
                error_sigma_m,
                bias_walk_mps,
                ..
            } => {
                let i = idx(self, node);
                self.nodes[i].gps_failure_flag = on;
                self.nodes[i].gps_failure = on.then_some(GpsFailure {
                    sigma: *error_sigma_m,
                    walk: *bias_walk_mps,
                    bias: [0.0; 2],
                });
            }
            Fault::GpsSpoof {
                node,
                offset_m,
                ramp_s,
                ..
            } => {
                let i = idx(self, node);
                let ramp_ticks = (ramp_s * f64::from(self.cfg.world.tick_hz)).round() as u64;
                self.nodes[i].gps_spoof = on.then_some(GpsSpoof {
                    offset: *offset_m,
                    ramp_ticks,
                    since: tick,
                });
            }
            Fault::LinkDegradation {
                link,
                extra_latency_ms,
                extra_loss,
                ..
            } => {
                let li = self.cfg.link_index(link).expect("validated link");
                for h in &mut self.links[li].hops {
                    h.extra_latency = if on { *extra_latency_ms } else { 0.0 };
                    h.extra_loss = if on { *extra_loss } else { 0.0 };
                }
            }
            Fault::Compromise { node, attacker, .. } => {
                let i = idx(self, node);
                self.nodes[i].compromise = on.then(|| Compromise {
                    profile: Arc::new(attacker.clone()),
                    since: tick,
                    announced: false,
                });
            }
            Fault::FalseTelemetry {
                node,
                position_drift_mps,
                flatline_cpu,
                ..
            } => {
                let i = idx(self, node);
                self.nodes[i].false_tel = on.then_some(FalseTelemetry {
                    drift: *position_drift_mps,
                    flatline: *flatline_cpu,
                    since: tick,
                });
            }
            Fault::Partition { hops, .. } => {
                for [a, b] in hops {
                    for (li, hi) in self.hop_indices(a, b) {
                        let h = &mut self.links[li].hops[hi];
                        h.cut = if on {
                            h.cut + 1
                        } else {
                            h.cut.saturating_sub(1)
                        };
                    }
                }
            }
            Fault::ModelMismatch {
                accel_sigma_mps2, ..
            } => {
                self.accel_sigma_override = on.then_some(*accel_sigma_mps2);
            }
            Fault::ActuatorFault {
                target, behavior, ..
            } => {
                if on {
                    self.actuator_faults.push((target.clone(), *behavior));
                } else {
                    self.actuator_faults.retain(|(t, _)| t != target);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Transition F
    // ------------------------------------------------------------------

    /// Advances the world one tick and returns that tick's observations.
    pub fn step(&mut self) -> Vec<Observation> {
        self.tick += 1;
        self.sync_faults();
        self.advance_truth();
        self.observations()
    }

    /// Advances only the true state by one tick (used by rollouts, which
    /// need no observations). Draws a fixed number of `env` values.
    // Index loops: the body reads nodes[i], nodes[j] and adj[i][j] together.
    #[allow(clippy::needless_range_loop)]
    pub(crate) fn advance_truth(&mut self) {
        let dt = self.dt();
        let accel_sigma = self
            .accel_sigma_override
            .unwrap_or(self.cfg.environment.process_noise_accel_mps2);
        let half = self.cfg.world.area_half_width_m;
        // 1. Motion (2 normals per node).
        for n in &mut self.nodes {
            let (ax, ay) = normal_pair(&mut self.rng_env);
            n.vel[0] += ax * accel_sigma * dt;
            n.vel[1] += ay * accel_sigma * dt;
            for k in 0..2 {
                n.pos[k] += n.vel[k] * dt;
                if n.pos[k].abs() > half {
                    n.pos[k] = n.pos[k].clamp(-half, half);
                    n.vel[k] = -n.vel[k];
                }
            }
        }
        // 2. Radio fades (1 uniform per hop).
        let env = &self.cfg.environment;
        for l in &mut self.links {
            for h in &mut l.hops {
                let u = unit(&mut self.rng_env);
                if h.fade_left > 0 {
                    h.fade_left -= 1;
                } else if u < env.fade_prob_per_tick {
                    h.fade_left = env.fade_duration_ticks;
                }
            }
        }
        // 3. Lateral movement (1 uniform per ordered node pair).
        let n = self.nodes.len();
        let adj = self.adjacency();
        let mut newly: Vec<(usize, Arc<AttackerProfile>)> = Vec::new();
        for i in 0..n {
            for j in 0..n {
                if i == j {
                    continue;
                }
                let u = unit(&mut self.rng_env);
                let Some(c) = &self.nodes[i].compromise else {
                    continue;
                };
                if !routed(self.nodes[i].mode)
                    || !routed(self.nodes[j].mode)
                    || self.nodes[j].compromise.is_some()
                    || !adj[i][j]
                {
                    continue;
                }
                let mut hazard = c.profile.lateral_hazard_per_tick;
                if self.nodes[i].mode == NodeMode::Restricted {
                    hazard *= self.cfg.actions.restrict_hazard_factor;
                }
                if u < hazard {
                    newly.push((j, Arc::clone(&c.profile)));
                }
            }
        }
        for (j, profile) in newly {
            if self.nodes[j].compromise.is_none() {
                self.nodes[j].compromise = Some(Compromise {
                    profile,
                    since: self.tick,
                    announced: false,
                });
            }
        }
        // 4. Onboard localisation error.
        let drift = self.cfg.sensors.dead_reckoning_drift_mps * dt;
        for i in 0..n {
            self.nodes[i].loc_sigma = match self.fix_sigma(i) {
                Some(s) => s,
                None => self.nodes[i].loc_sigma + drift,
            };
        }
    }

    /// Localisation error from usable position fixes, or `None` if no fix.
    fn fix_sigma(&self, i: usize) -> Option<f64> {
        let s = &self.cfg.sensors;
        let node = &self.nodes[i];
        let mut info = 1.0 / (s.camera_sigma_m * s.camera_sigma_m);
        if !node.gps_failure_flag {
            info += 1.0 / (s.gps_sigma_m * s.gps_sigma_m);
        }
        let sigma = 1.0 / info.sqrt();
        // An undetected spoof drags the onboard solution by its offset.
        let bias = node.gps_spoof.as_ref().map_or(0.0, |sp| {
            let f = self.spoof_fraction(sp);
            (sp.offset[0] * f).hypot(sp.offset[1] * f)
        });
        Some(sigma.hypot(bias))
    }

    fn spoof_fraction(&self, sp: &GpsSpoof) -> f64 {
        if sp.ramp_ticks == 0 {
            1.0
        } else {
            ((self.tick.saturating_sub(sp.since)) as f64 / sp.ramp_ticks as f64).min(1.0)
        }
    }

    /// `adj[i][j]`: i and j share an up hop of an active link and both route.
    fn adjacency(&self) -> Vec<Vec<bool>> {
        let n = self.nodes.len();
        let mut adj = vec![vec![false; n]; n];
        for (li, l) in self.cfg.links.iter().enumerate() {
            if !self.links[li].active {
                continue;
            }
            let path: Vec<usize> = l
                .path()
                .iter()
                .map(|id| self.cfg.node_index(id).expect("validated"))
                .collect();
            for (hi, w) in path.windows(2).enumerate() {
                if self.links[li].hops[hi].cut == 0 {
                    adj[w[0]][w[1]] = true;
                    adj[w[1]][w[0]] = true;
                }
            }
        }
        adj
    }

    // ------------------------------------------------------------------
    // Capabilities and mission (ground truth)
    // ------------------------------------------------------------------

    fn hop_availability(&self, li: usize, hi: usize) -> f64 {
        let h = &self.links[li].hops[hi];
        if h.cut > 0 {
            return 0.0;
        }
        let mut a = self.cfg.links[li].hop_availability[hi] * (1.0 - h.extra_loss);
        if h.fade_left > 0 {
            a *= self.cfg.environment.fade_availability_factor;
        }
        a
    }

    fn link_availability(&self, li: usize) -> f64 {
        (0..self.links[li].hops.len())
            .map(|hi| self.hop_availability(li, hi))
            .product()
    }

    fn integrity(&self, i: usize) -> f64 {
        self.nodes[i]
            .compromise
            .as_ref()
            .map_or(1.0, |c| c.profile.relay_integrity)
    }

    /// True capability levels (§6.7 on ground truth).
    pub fn capabilities(&self) -> Capabilities {
        let cfg = &self.cfg;
        // Communication between the mission endpoints.
        let mut comm = 0.0f64;
        let ends = &cfg.mission.endpoints;
        for (li, l) in cfg.links.iter().enumerate() {
            let same_ends = (l.endpoints[0] == ends[0] && l.endpoints[1] == ends[1])
                || (l.endpoints[0] == ends[1] && l.endpoints[1] == ends[0]);
            if !same_ends || !self.links[li].active {
                continue;
            }
            let mut v = self.link_availability(li);
            for id in l.path() {
                let i = cfg.node_index(id).expect("validated");
                if !routed(self.nodes[i].mode) {
                    v = 0.0;
                    break;
                }
                v *= self.integrity(i);
            }
            comm = comm.max(v);
        }
        // Observation.
        let mut miss = 1.0;
        let mut nav = f64::INFINITY;
        for (i, n) in self.nodes.iter().enumerate() {
            if !routed(n.mode) {
                continue;
            }
            let q = cfg.nodes[i].camera_quality
                * n.compromise
                    .as_ref()
                    .map_or(1.0, |c| c.profile.observation_integrity);
            miss *= 1.0 - cfg.nodes[i].coverage * q;
            nav = nav.min(1.0 - (n.loc_sigma / cfg.mission.sigma_max_m).min(1.0));
        }
        Capabilities {
            navigation: if nav.is_finite() { nav } else { 0.0 },
            communication: comm,
            observation: 1.0 - miss,
        }
    }

    /// Minima currently in force (degraded where `degrade(k)` applied).
    pub fn effective_minima(&self) -> CapabilityMinima {
        let m = &self.cfg.mission;
        let pick = |c: Capability| {
            if self.degraded[cap_index(c)] {
                m.degraded_min.get(c)
            } else {
                m.min.get(c)
            }
        };
        CapabilityMinima {
            navigation: pick(Capability::Navigation),
            communication: pick(Capability::Communication),
            observation: pick(Capability::Observation),
        }
    }

    /// `true` when every capability meets its minimum.
    pub fn mission_ok(&self) -> bool {
        self.capabilities().meets(&self.effective_minima())
    }

    /// Evaluator-only ground truth. Never publish this.
    pub fn ground_truth(&self) -> GroundTruth {
        let caps = self.capabilities();
        GroundTruth {
            tick: self.tick,
            nodes: self
                .nodes
                .iter()
                .zip(&self.cfg.nodes)
                .map(|(n, c)| NodeTruth {
                    id: c.id.clone(),
                    mode: n.mode,
                    compromised: n.compromise.is_some(),
                    compromised_since: n.compromise.as_ref().map(|c| c.since),
                    gps_failed: n.gps_failure_flag,
                    gps_spoofed: n.gps_spoof.is_some(),
                    false_telemetry: n.false_tel.is_some(),
                    loc_sigma_m: n.loc_sigma,
                })
                .collect(),
            mission_ok: caps.meets(&self.effective_minima()),
            capabilities: caps,
            active_faults: self
                .scenario
                .faults
                .iter()
                .zip(&self.fault_active)
                .filter(|(_, a)| **a)
                .map(|(f, _)| f.kind().to_string())
                .collect(),
        }
    }

    // ------------------------------------------------------------------
    // Observable state (what a control plane can measure)
    // ------------------------------------------------------------------

    fn link_latency_loss(&self, li: usize) -> (f64, f64) {
        let cfg = &self.cfg.links[li];
        let mut lat = 0.0;
        let mut deliver = 1.0;
        for (hi, h) in self.links[li].hops.iter().enumerate() {
            lat += cfg.latency_ms[hi] + h.extra_latency;
            deliver *= (1.0 - cfg.loss[hi] - h.extra_loss).max(0.0);
        }
        (lat, (1.0 - deliver).clamp(0.0, 1.0))
    }

    /// Topology and link measurements. Contains no attacker ground truth.
    pub fn observe(&self) -> aeris_wire::WorldObservation {
        let mut nodes: Vec<(String, NodeMode)> = self
            .cfg
            .nodes
            .iter()
            .zip(&self.nodes)
            .map(|(c, n)| (c.id.clone(), n.mode))
            .collect();
        nodes.sort_by(|a, b| a.0.cmp(&b.0));
        let mut links: Vec<LinkObservation> = self
            .cfg
            .links
            .iter()
            .enumerate()
            .map(|(li, l)| {
                let (latency_ms, loss) = self.link_latency_loss(li);
                LinkObservation {
                    link: l.id.clone(),
                    active: self.links[li].active,
                    up: self.links[li].hops.iter().all(|h| h.cut == 0),
                    availability: self.link_availability(li),
                    latency_ms,
                    loss,
                }
            })
            .collect();
        links.sort_by(|a, b| a.link.cmp(&b.link));
        aeris_wire::WorldObservation {
            tick: self.tick,
            t_unix_ms: self.sim_time_ms(),
            nodes,
            links,
            degraded: ALL_CAPS
                .into_iter()
                .filter(|c| self.degraded[cap_index(*c)])
                .collect(),
        }
    }

    /// Can node `i`'s traffic reach the uplink node right now?
    fn reaches_uplink(&self, i: usize) -> bool {
        let up = self
            .cfg
            .node_index(&self.cfg.mission.uplink_node)
            .expect("validated");
        if self.nodes[i].mode == NodeMode::Isolated {
            return false;
        }
        if i == up {
            return true;
        }
        let adj = self.adjacency_for_uplink();
        let mut seen = vec![false; self.nodes.len()];
        let mut stack = vec![i];
        seen[i] = true;
        while let Some(u) = stack.pop() {
            if u == up {
                return true;
            }
            for v in 0..self.nodes.len() {
                // Only routing nodes relay other nodes' traffic.
                if adj[u][v] && !seen[v] && (v == up || routed(self.nodes[v].mode)) {
                    seen[v] = true;
                    stack.push(v);
                }
            }
        }
        false
    }

    fn adjacency_for_uplink(&self) -> Vec<Vec<bool>> {
        // Like `adjacency`, but a standby node may still send its own traffic.
        let n = self.nodes.len();
        let mut adj = vec![vec![false; n]; n];
        for (li, l) in self.cfg.links.iter().enumerate() {
            if !self.links[li].active {
                continue;
            }
            let path: Vec<usize> = l
                .path()
                .iter()
                .map(|id| self.cfg.node_index(id).expect("validated"))
                .collect();
            for (hi, w) in path.windows(2).enumerate() {
                if self.links[li].hops[hi].cut == 0 {
                    adj[w[0]][w[1]] = true;
                    adj[w[1]][w[0]] = true;
                }
            }
        }
        adj
    }

    // ------------------------------------------------------------------
    // Observations O = h(S) + V + A_adv
    // ------------------------------------------------------------------

    fn observations(&mut self) -> Vec<Observation> {
        let cfg = Arc::clone(&self.cfg);
        let t = &cfg.telemetry;
        let s = &cfg.sensors;
        let hz = f64::from(cfg.world.tick_hz);
        let dt = self.dt();
        let tick = self.tick;
        let sim_ms = self.sim_time_ms();
        let mut out = Vec::new();
        for i in 0..self.nodes.len() {
            let id = cfg.nodes[i].id.clone();
            let delivery = if self.nodes[i].mode == NodeMode::Isolated {
                Delivery::Quarantined
            } else if self.reaches_uplink(i) {
                Delivery::Uplink
            } else {
                Delivery::Partitioned
            };
            let mk = |payload: WirePayload, confidence: f64, producer: String| Observation {
                tick,
                sim_time_ms: sim_ms,
                node: id.clone(),
                payload,
                confidence,
                producer,
                delivery,
            };
            let rng = &mut self.rng_obs;
            let node = &mut self.nodes[i];
            let attack = node.compromise.clone();
            let beaconing = attack.is_some() && node.mode != NodeMode::Restricted;

            // Telemetry.
            let (n1, n2) = normal_pair(rng);
            let (mut cpu, mut mem) = (
                t.cpu_mean_pct + n1 * t.cpu_sd_pct,
                t.mem_mean_pct + n2 * t.mem_sd_pct,
            );
            let mut procs = t.proc_count;
            if let Some(a) = &attack {
                cpu += a.profile.cpu_extra_pct;
                mem += a.profile.mem_extra_pct;
                procs += a.profile.rogue_procs;
            }
            if node.false_tel.as_ref().is_some_and(|f| f.flatline) {
                cpu = t.cpu_mean_pct;
            }
            out.push(mk(
                WirePayload::Telemetry {
                    cpu_pct: cpu.clamp(0.0, 100.0),
                    mem_pct: mem.clamp(0.0, 100.0),
                    proc_count: procs,
                },
                1.0,
                format!("aeris-sim/{id}"),
            ));

            // Network.
            let (n1, n2) = normal_pair(rng);
            let mut pkts = (t.pkts_mean_per_s + n1 * t.pkts_sd_per_s).max(0.0);
            let mut peers = t.peers;
            if beaconing && let Some(a) = &attack {
                pkts += a.profile.beacon_pkts_per_sec;
                peers += a.profile.beacon_peers;
            }
            let (lat, loss) = self.node_route_metrics_cached(i);
            out.push(mk(
                WirePayload::Net {
                    pkts_per_sec: pkts,
                    bytes_per_sec: pkts * t.bytes_per_pkt,
                    peers,
                    latency_ms: (lat + n2 * t.latency_jitter_ms).max(0.0),
                    loss,
                },
                1.0,
                format!("aeris-sim/{id}"),
            ));

            // Sensors.
            let node = &mut self.nodes[i];
            let rng = &mut self.rng_obs;
            let since_ticks = |since: u64| tick.saturating_sub(since) as f64 * dt;
            let lie = node.false_tel.as_ref().map_or([0.0; 2], |f| {
                [
                    f.drift[0] * since_ticks(f.since),
                    f.drift[1] * since_ticks(f.since),
                ]
            });
            let (gx, gy) = normal_pair(rng);
            let mut gps = [
                node.pos[0] + gx * s.gps_sigma_m,
                node.pos[1] + gy * s.gps_sigma_m,
            ];
            if let Some(g) = node.gps_failure.as_mut() {
                let (bx, by) = normal_pair(rng);
                g.bias[0] += bx * g.walk * dt;
                g.bias[1] += by * g.walk * dt;
                let (ex, ey) = normal_pair(rng);
                gps[0] += g.bias[0] + ex * g.sigma;
                gps[1] += g.bias[1] + ey * g.sigma;
            }
            if let Some(sp) = &node.gps_spoof {
                let f = if sp.ramp_ticks == 0 {
                    1.0
                } else {
                    (tick.saturating_sub(sp.since) as f64 / sp.ramp_ticks as f64).min(1.0)
                };
                gps[0] += sp.offset[0] * f;
                gps[1] += sp.offset[1] * f;
            }
            gps[0] += lie[0];
            gps[1] += lie[1];
            let (cx, cy) = normal_pair(rng);
            let mut cam = [
                node.pos[0] + cx * s.camera_sigma_m,
                node.pos[1] + cy * s.camera_sigma_m,
            ];
            if let Some(a) = &attack {
                cam[0] += a.profile.camera_report_offset_m[0];
                cam[1] += a.profile.camera_report_offset_m[1];
            }
            cam[0] += lie[0];
            cam[1] += lie[1];
            let (ix, iy) = normal_pair(rng);
            let imu = [
                node.vel[0] + ix * s.imu_sigma_mps,
                node.vel[1] + iy * s.imu_sigma_mps,
            ];
            for (kind, value, sigma, conf) in [
                (SensorKind::Gps, gps, s.gps_sigma_m, s.gps_confidence),
                (SensorKind::Imu, imu, s.imu_sigma_mps, s.imu_confidence),
                (
                    SensorKind::Camera,
                    cam,
                    s.camera_sigma_m,
                    s.camera_confidence,
                ),
            ] {
                out.push(mk(
                    WirePayload::Sensor {
                        kind,
                        value: value.to_vec(),
                        sigma,
                    },
                    conf,
                    format!("aeris-sim/{id}/{kind}"),
                ));
            }

            // Authentication.
            let u_legit = unit(rng);
            let u_fail = unit(rng);
            if u_legit < t.legit_auth_rate_hz / hz {
                out.push(mk(
                    WirePayload::Auth {
                        service: t.legit_auth_service.clone(),
                        success: true,
                        principal: t.legit_auth_principal.clone(),
                    },
                    1.0,
                    format!("aeris-sim/{id}/auth"),
                ));
            }
            if let Some(a) = &attack
                && u_fail < a.profile.auth_fail_rate_hz / hz
            {
                out.push(mk(
                    WirePayload::Auth {
                        service: a.profile.auth_service.clone(),
                        success: false,
                        principal: a.profile.auth_principal.clone(),
                    },
                    1.0,
                    format!("aeris-sim/{id}/auth"),
                ));
            }

            // Implant process start (once).
            if let Some(c) = node.compromise.as_mut()
                && !c.announced
            {
                c.announced = true;
                out.push(mk(
                    WirePayload::Process {
                        name: c.profile.rogue_process_name.clone(),
                        started: true,
                        hash: c.profile.rogue_process_hash.clone(),
                    },
                    1.0,
                    format!("aeris-sim/{id}/proc"),
                ));
            }

            // Link status from the uplink node.
            if id == cfg.mission.uplink_node
                && tick.is_multiple_of(u64::from(t.link_status_period_ticks))
            {
                for (li, l) in cfg.links.iter().enumerate() {
                    out.push(mk(
                        WirePayload::Link {
                            link_id: l.id.clone(),
                            up: self.links[li].hops.iter().all(|h| h.cut == 0),
                            availability: self.link_availability(li),
                        },
                        1.0,
                        format!("aeris-sim/{id}/link"),
                    ));
                }
            }
        }
        out
    }

    /// Latency and loss of the best active, up link that carries node `i`.
    fn node_route_metrics_cached(&self, i: usize) -> (f64, f64) {
        let id = &self.cfg.nodes[i].id;
        let mut best: Option<(f64, f64)> = None;
        for (li, l) in self.cfg.links.iter().enumerate() {
            let ls = &self.links[li];
            if !ls.active || ls.hops.iter().any(|h| h.cut > 0) || !l.path().contains(&id.as_str()) {
                continue;
            }
            let m = self.link_latency_loss(li);
            if best.is_none_or(|b| m.0 < b.0) {
                best = Some(m);
            }
        }
        best.unwrap_or((0.0, 1.0))
    }

    // ------------------------------------------------------------------
    // Commands
    // ------------------------------------------------------------------

    /// Applies one command. Errors are reported, never swallowed; on error
    /// the world is unchanged.
    pub fn apply_command(&mut self, op: &CommandOp) -> Result<ApplyOutcome, SimError> {
        let target = op.target_entity();
        if let Some((_, b)) = self.actuator_faults.iter().find(|(t, _)| *t == target) {
            return match b {
                ActuatorBehavior::AckNoEffect => Ok(ApplyOutcome::Applied),
                ActuatorBehavior::Refuse => Err(SimError::Refused(target)),
            };
        }
        let node = |w: &Self, n: &str| {
            w.cfg
                .node_index(n)
                .ok_or_else(|| SimError::UnknownEntity(n.to_string()))
        };
        let set_mode = |w: &mut Self, i: usize, m: NodeMode| {
            if w.nodes[i].mode == m {
                ApplyOutcome::AlreadyApplied
            } else {
                w.nodes[i].mode = m;
                ApplyOutcome::Applied
            }
        };
        match op {
            CommandOp::Isolate { node: n } => {
                let i = node(self, n)?;
                Ok(set_mode(self, i, NodeMode::Isolated))
            }
            CommandOp::Restrict { node: n } => {
                let i = node(self, n)?;
                if self.nodes[i].mode == NodeMode::Isolated {
                    return Err(SimError::InvalidTransition(format!(
                        "{n} is isolated; recover it before restricting"
                    )));
                }
                Ok(set_mode(self, i, NodeMode::Restricted))
            }
            CommandOp::Recover { node: n } => {
                let i = node(self, n)?;
                Ok(set_mode(self, i, NodeMode::Active))
            }
            CommandOp::ActivateLink { link } => {
                let li = self
                    .cfg
                    .link_index(link)
                    .ok_or_else(|| SimError::UnknownEntity(link.clone()))?;
                if self.links[li].active {
                    Ok(ApplyOutcome::AlreadyApplied)
                } else {
                    self.links[li].active = true;
                    Ok(ApplyOutcome::Applied)
                }
            }
            CommandOp::Failover { from, to } => {
                let f = node(self, from)?;
                let t = node(self, to)?;
                if !routed(self.nodes[t].mode) {
                    return Err(SimError::InvalidTransition(format!(
                        "failover target {to} is not routing"
                    )));
                }
                if self.nodes[f].mode == NodeMode::Standby {
                    return Ok(ApplyOutcome::AlreadyApplied);
                }
                self.nodes[f].mode = NodeMode::Standby;
                for (li, l) in self.cfg.links.iter().enumerate() {
                    let via_from = l.via.iter().any(|v| v == from);
                    if via_from {
                        self.links[li].active = false;
                    } else if !self.links[li].active {
                        self.links[li].active = true;
                    }
                }
                Ok(ApplyOutcome::Applied)
            }
            CommandOp::Degrade { capability } => {
                let k = cap_index(*capability);
                if self.degraded[k] {
                    Ok(ApplyOutcome::AlreadyApplied)
                } else {
                    self.degraded[k] = true;
                    Ok(ApplyOutcome::Applied)
                }
            }
            CommandOp::Observe => Ok(ApplyOutcome::AlreadyApplied),
        }
    }
}

/// Builds the expected observable state after `plan` from `obs` — the
/// topology part of the expected state for verification.
pub fn expected_topology(
    cfg: &Arc<WorldConfig>,
    obs: &aeris_wire::WorldObservation,
    plan: &[CommandOp],
) -> Result<aeris_wire::WorldObservation, SimError> {
    let mut w = World::from_observation(Arc::clone(cfg), obs, [0u8; 32])?;
    for op in plan {
        w.apply_command(op)?;
    }
    let mut out = w.observe();
    out.tick = obs.tick;
    out.t_unix_ms = obs.t_unix_ms;
    Ok(out)
}

impl World {
    /// A world reconstructed from an observation: topology, link
    /// degradation and degraded capabilities as measured, with **no**
    /// ground truth about attackers or sensor faults. This is the base for
    /// counterfactual rollouts, which then sample hidden state from belief.
    pub fn from_observation(
        cfg: Arc<WorldConfig>,
        obs: &aeris_wire::WorldObservation,
        stream_seed: [u8; 32],
    ) -> Result<Self, SimError> {
        let mut w = Self::quiet(Arc::clone(&cfg), 0)?.fork(stream_seed);
        w.tick = obs.tick;
        for (id, mode) in &obs.nodes {
            let i = cfg
                .node_index(id)
                .ok_or_else(|| SimError::UnknownEntity(id.clone()))?;
            w.nodes[i].mode = *mode;
        }
        for l in &obs.links {
            let li = cfg
                .link_index(&l.link)
                .ok_or_else(|| SimError::UnknownEntity(l.link.clone()))?;
            w.links[li].active = l.active;
            for h in &mut w.links[li].hops {
                h.cut = u32::from(!l.up);
            }
            if l.up {
                let nominal: f64 = cfg.links[li].hop_availability.iter().product();
                if nominal > 0.0 {
                    let factor = (l.availability / nominal).clamp(0.0, 1.0);
                    // Persist the measured degradation on the first hop.
                    w.links[li].hops[0].extra_loss = 1.0 - factor;
                }
            }
        }
        for c in &obs.degraded {
            w.degraded[cap_index(*c)] = true;
        }
        Ok(w)
    }
}

/// Capability estimate from an observation alone: topology and measured
/// link availability, assuming every routing node is honest and every sensor
/// healthy. Equivalent to [`estimate_capabilities`] with no trust input.
pub fn capabilities_from_observation(
    cfg: &Arc<WorldConfig>,
    obs: &aeris_wire::WorldObservation,
) -> Result<Capabilities, SimError> {
    estimate_capabilities(cfg, obs, &[])
}

/// AERIS's capability estimate (§6.7) from an observation and its current
/// node trust: `c_comm = max over active up paths of ∏ avail_e · ∏ T_v` over
/// the path's *interior* nodes. Nodes missing from `node_trust` count as
/// fully trusted. Navigation and observation use nominal sensor quality
/// (sensor health is not observable from topology). Never reads ground truth.
pub fn estimate_capabilities(
    cfg: &Arc<WorldConfig>,
    obs: &aeris_wire::WorldObservation,
    node_trust: &[(String, f64)],
) -> Result<Capabilities, SimError> {
    let mut w = World::from_observation(Arc::clone(cfg), obs, [0u8; 32])?;
    for i in 0..w.nodes.len() {
        w.nodes[i].loc_sigma = w.fix_sigma(i).unwrap_or(0.0);
    }
    let mut caps = w.capabilities();
    let trust = |id: &str| {
        node_trust
            .iter()
            .find(|(n, _)| n == id)
            .map_or(1.0, |(_, t)| t.clamp(0.0, 1.0))
    };
    let ends = &cfg.mission.endpoints;
    let mut comm = 0.0f64;
    for (li, l) in cfg.links.iter().enumerate() {
        let same_ends = (l.endpoints[0] == ends[0] && l.endpoints[1] == ends[1])
            || (l.endpoints[0] == ends[1] && l.endpoints[1] == ends[0]);
        if !same_ends || !w.links[li].active {
            continue;
        }
        let mut v = w.link_availability(li);
        for id in l.path() {
            let i = cfg.node_index(id).expect("validated");
            if !routed(w.nodes[i].mode) {
                v = 0.0;
            }
        }
        for id in &l.via {
            v *= trust(id);
        }
        comm = comm.max(v);
    }
    caps.communication = comm;
    Ok(caps)
}
