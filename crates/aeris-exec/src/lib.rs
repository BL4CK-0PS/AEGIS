//! AERIS execution layer (§6.13): turns an *approved* action into concrete
//! commands, sends them through an [`Adapter`], and verifies the result.
//!
//! Owner: DHARSHAN.
//!
//! * [`gamma`] — Γ(a), the single action → command translation (shared with
//!   the simulator's rollouts so simulated and executed effects agree).
//! * [`Adapter`] — the seam for execution targets. v0.1 ships
//!   [`LocalSimAdapter`] (in-process) and, with feature `nats`,
//!   `NatsSimAdapter` (to `aeris-simd`). No OS or network adapters.
//! * [`authz`] — execution-time authorization; HIGH impact never runs
//!   without a commander's approval of that exact action.
//! * [`policy`] — re-check against the *current* state before sending.
//! * [`verify`] — `‖e‖ ≤ ε` against the expected state.
//! * [`executor`] — lifecycle, deduplication, ordered submission with
//!   deadlines, partial-execution reporting, replan and escalation.
//!
//! The decision logic is not here: this crate cannot choose an action.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod adapter;
pub mod authz;
pub mod executor;
#[cfg(feature = "nats")]
pub mod nats;
pub mod policy;
pub mod verify;

pub use adapter::{Adapter, AdapterError, LocalSimAdapter, SharedWorld, ack_for};
pub use authz::{Approval, Authorization, AuthzError, Role, Verdict, authorize};
pub use executor::{
    CommandRecord, ExecConfig, ExecState, ExecutionReport, ExecutionRequest, Executor, Lifecycle,
    Refusal, ReplanSignal,
};
pub use policy::{PolicyRecheck, PolicyVeto, TopologyGuard};
pub use verify::{ExpectedState, Verification, verify};

/// Γ(a): action → ordered commands. Re-export of
/// [`aeris_wire::commands_for_action`].
pub use aeris_wire::commands_for_action as gamma;
