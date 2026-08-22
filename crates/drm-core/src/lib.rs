//! `drm-core`: the DRM O/D/C developmental runtime engine.
//!
//! A [`planner::DrmPlanner`] learns a compressed vocabulary of recurring
//! workflow motifs from a stream of [`episode::Episode`]s, each a task
//! identity paired with a sequence of [`capability`] tokens. Every
//! capability -- and every symbol later derived from capabilities --
//! reduces recursively to nothing but the frozen root vocabulary
//! `OBSERVE` / `DERIVE` / `COMMIT` (see [`capability::ROOT`]); this is
//! audited by [`vocabulary::Vocabulary::audit`] and is the one invariant
//! that must never break.
//!
//! [`hybrid::HybridPlanner`] extends the base planner with a two-tier
//! provisional/permanent vocabulary and a deferred consolidation pass
//! suitable for a latency-sensitive service (see its module docs for why).
//!
//! [`registry::Registry`] is the adaptive-execution-layer layer on top:
//! one [`hybrid::HybridPlanner`] per `application_id` (never one shared
//! flat vocabulary -- an application's learned structure never leaks into
//! another's without evidence), a [`registry::GlobalVocabulary`] that
//! only admits structure independently proven useful across multiple
//! applications, and [`identity::ExecutionContext`]-based attribution
//! answering "which application produced this, and did it transfer
//! beyond its birth workload/application?" for every learned word. See
//! `registry`'s module docs for why cross-application transfer is
//! detected by structural pattern comparison rather than by word reuse.
//!
//! This crate is pure planning logic over `Vec<String>` sequences -- it has
//! no I/O and no OS dependency. Real execution of planned capabilities
//! against the host lives in the `drm-exec` crate; deciding whether a
//! learned word is also safe to execute *differently* (not just name
//! more cheaply) lives in the `drm-opt` crate.

pub mod baseline;
pub mod capability;
pub mod episode;
pub mod hybrid;
pub mod identity;
pub mod lifecycle;
pub mod persistence;
pub mod planner;
pub mod registry;
pub mod vocabulary;

pub use baseline::{Baseline, BaselineKind};
pub use capability::{is_known_capability, is_root, root_expansion, CAPABILITIES, ROOT};
pub use episode::{Episode, PlanMetrics};
pub use hybrid::HybridPlanner;
pub use identity::{ExecutionContext, TransferScope};
pub use lifecycle::{transition, IllegalTransition, LifecycleStage};
pub use planner::DrmPlanner;
pub use registry::{AppState, ConsolidationReport, GlobalVocabulary, Registry, WordMeta};
pub use vocabulary::{Seq, VocabError, Vocabulary};
