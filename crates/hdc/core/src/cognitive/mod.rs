//! Cognitive architecture — ALMA affect model, behavioral state machine,
//! somatic marker bias, dream cycle, and Mattar-Daw replay.

pub mod affect;
pub mod dream;
pub mod replay;
pub mod somatic;
pub mod state_machine;

pub use affect::{AlmaState, PadState};
pub use dream::{DreamConfig, DreamReport};
pub use replay::{ReplayEntry, mattar_daw_evb, prioritize_replay};
pub use somatic::{apply_somatic_bias, apply_somatic_candidate_filter};
pub use state_machine::{BehavioralState, StateContext, StateModifiers, TickMetrics};
