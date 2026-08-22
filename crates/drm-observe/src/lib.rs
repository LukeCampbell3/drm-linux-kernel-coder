//! `drm-observe`: application/workload identity resolution and
//! per-execution measurement for the DRM Adaptive Execution Layer.
//!
//! This crate is intentionally a leaf: it depends on nothing else in the
//! workspace and is depended on by `drmd` (and, for benchmarking, by the
//! `simulate` harness) to answer two questions a caller shouldn't have to
//! answer for itself:
//!
//! - "what host/user is this process running as?" ([`identity`])
//! - "how much did that unit of work actually cost, for real, on this
//!   machine?" ([`metrics`])
//!
//! Neither of these is an OBSERVE/DERIVE/COMMIT capability in the O/D/C
//! sense used by `drm-core`/`drm-exec`: they observe *the runtime itself*
//! (process resource counters, environment identity) rather than
//! performing or deriving structure over one of the frozen root
//! capabilities. That parallels the existing `proc.observe` capability's
//! own status -- a `/proc` read used as evidence, not a new root.

pub mod identity;
pub mod metrics;

pub use identity::{resolve_host_id, resolve_user_scope};
pub use metrics::{Engine, ExecutionMetrics, MeasuredRun, ResourceCost, Warmth};
