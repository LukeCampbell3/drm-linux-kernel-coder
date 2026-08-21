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
//! This crate is pure planning logic over `Vec<String>` sequences -- it has
//! no I/O and no OS dependency. Real execution of planned capabilities
//! against the host lives in the `drm-exec` crate.

pub mod baseline;
pub mod capability;
pub mod episode;
pub mod hybrid;
pub mod planner;
pub mod vocabulary;

pub use baseline::{Baseline, BaselineKind};
pub use capability::{is_known_capability, is_root, root_expansion, CAPABILITIES, ROOT};
pub use episode::{Episode, PlanMetrics};
pub use hybrid::HybridPlanner;
pub use planner::DrmPlanner;
pub use vocabulary::{Seq, VocabError, Vocabulary};
