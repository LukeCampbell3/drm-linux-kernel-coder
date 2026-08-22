//! `drm-opt`: executable specialization for the DRM Adaptive Execution
//! Layer -- construction, behavioral-equivalence validation, and
//! lifecycle-gated admission/rollback of specialization candidates.
//!
//! This is deliberately narrow in scope (see plan.md scoping decision
//! 2): exactly two conservative mechanisms are modeled
//! ([`specialization::SpecializationKind`]), both explicitly pre-approved
//! starting points in the spec -- deterministic pure-transform fusion and
//! memoization, and redundant-immutable-read avoidance. Neither ever
//! changes *what* root O/D/C effects occur, only *how* work already
//! described by the capability vocabulary is scheduled or cached; that is
//! the acceptance criterion checked in [`equivalence`] before anything is
//! ever admitted by [`admission::AdmissionLedger`].
//!
//! A specialization candidate is a distinct object from a symbolic
//! vocabulary word (`drm-core`'s concern) even though both share one
//! [`drm_core::lifecycle::LifecycleStage`] state machine -- per the
//! requirement that primitive capability, derived vocabulary word, and
//! verified executable specialization stay three separate objects in the
//! code model, not three names for the same thing.

pub mod admission;
pub mod equivalence;
pub mod specialization;

pub use admission::{AdmissionError, AdmissionLedger, VALIDATION_THRESHOLD};
pub use equivalence::{check_read_avoidance_equivalence, check_transform_fusion_equivalence, content_hash, EquivalenceResult};
pub use specialization::{SpecializationCandidate, SpecializationKind};
