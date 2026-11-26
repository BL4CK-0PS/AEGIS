//! AERIS deterministic simulated world.
//!
//! Owner: DHARSHAN. Specification: `docs/v0.1/04-mathematical-model.md`
//! §6.1 (world model), §6.7 (capabilities), §6.10 (rollouts); world in
//! `docs/spec/world.toml`; scenarios in `scenarios/s1..s8.toml`.
//!
//! * [`World`] — nodes A/B/C with GPS/IMU/camera, links L1 (A–B–C) and L2
//!   (A–C standby), mission capabilities; `step()` is
//!   `S(t+1) = F(S(t), a, W(t))` at 10 Hz with seeded ChaCha8 streams.
//! * [`Scenario`] — S1–S8 fault schedules and the attacker model. Ground
//!   truth stays inside the world; detectors see only [`Observation`]s.
//! * [`rollout`] — `fork()` + Monte Carlo rollouts with common random
//!   numbers, returning outcome samples for the decision engine.
//!
//! This crate does no I/O and no wall-clock reads: same seed, same output.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod config;
pub mod rng;
pub mod rollout;
pub mod scenario;
pub mod world;

pub use config::{CapabilityMinima, WorldConfig};
pub use rollout::{Belief, NodeBelief, RolloutBatch, RolloutOutcome, RolloutSpec, run_rollouts};
pub use scenario::{AttackerProfile, Fault, Scenario};
pub use world::{
    ApplyOutcome, Delivery, GroundTruth, Observation, World, capabilities_from_observation,
    estimate_capabilities, expected_topology,
};

use aeris_types::Capability;
use serde::{Deserialize, Serialize};

/// Simulator error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SimError {
    /// Invalid world configuration.
    #[error("world config: {0}")]
    Config(String),
    /// Invalid scenario file.
    #[error("scenario: {0}")]
    Scenario(String),
    /// Command names an entity that does not exist.
    #[error("unknown entity {0}")]
    UnknownEntity(String),
    /// Command is not valid in the current state.
    #[error("invalid transition: {0}")]
    InvalidTransition(String),
    /// An actuator fault refused the command.
    #[error("actuator refused command for {0}")]
    Refused(String),
    /// Invalid rollout request.
    #[error("rollout: {0}")]
    Rollout(String),
}

/// Capability levels `c_nav, c_comm, c_obs`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Navigation.
    pub navigation: f64,
    /// Communication.
    pub communication: f64,
    /// Observation.
    pub observation: f64,
}

impl Capabilities {
    /// Level of one capability.
    pub fn get(&self, c: Capability) -> f64 {
        match c {
            Capability::Navigation => self.navigation,
            Capability::Communication => self.communication,
            Capability::Observation => self.observation,
        }
    }

    /// Sets one capability.
    pub fn set(&mut self, c: Capability, v: f64) {
        match c {
            Capability::Navigation => self.navigation = v,
            Capability::Communication => self.communication = v,
            Capability::Observation => self.observation = v,
        }
    }

    /// `true` when every capability is at or above its minimum.
    pub fn meets(&self, m: &CapabilityMinima) -> bool {
        self.navigation >= m.navigation
            && self.communication >= m.communication
            && self.observation >= m.observation
    }

    /// As the `(Capability, f64)` list used in `aeris_types::CycleOutput`.
    pub fn to_vec(&self) -> Vec<(Capability, f64)> {
        vec![
            (Capability::Navigation, self.navigation),
            (Capability::Communication, self.communication),
            (Capability::Observation, self.observation),
        ]
    }
}

#[cfg(test)]
mod tests;
