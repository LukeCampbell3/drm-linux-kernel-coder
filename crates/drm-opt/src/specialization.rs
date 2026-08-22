//! The two conservative executable-specialization mechanisms this phase
//! admits (see plan.md scoping decision 2) and the record kept for each
//! proposed one, admitted or not.
//!
//! Both mechanisms change *how* work already described by the O/D/C
//! capability vocabulary is scheduled/cached -- never *what* root effects
//! occur, which is the acceptance criterion checked before either is ever
//! admitted (see [`crate::equivalence`] for the mechanism-specific
//! checks). Nothing more elaborate is in scope for this phase: no JIT, no
//! self-modifying dispatch, no speculative execution.

use drm_core::lifecycle::LifecycleStage;
use drm_core::vocabulary::Seq;

/// Which of the two pre-approved mechanisms a candidate specializes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpecializationKind {
    /// A derived word's expansion is a pure `transform.*` chain (no I/O
    /// capability appears inside it): fuse the stages into one function
    /// call, memoized by a hash of its input, instead of running each
    /// stage and materializing the intermediate output between them.
    TransformFusion { stages: Vec<String> },
    /// Skip re-reading `path` via `fs.read` when its content hash from a
    /// prior read in this process is still known good -- no write to
    /// that path has been observed since the hash was recorded.
    ReadAvoidance { path: String },
}

impl SpecializationKind {
    pub fn describe(&self) -> String {
        match self {
            SpecializationKind::TransformFusion { stages } => format!("fuse[{}]", stages.join("->")),
            SpecializationKind::ReadAvoidance { path } => format!("avoid_read[{path}]"),
        }
    }
}

/// One specialization's full developmental record: baseline path,
/// lifecycle stage, and the evidence that moved it (or failed to move
/// it) along that lifecycle. Spec S7 requires exactly this shape --
/// baseline path, optimized path (`kind`), equivalence check history,
/// admission reason, rollback path, measured gain -- to exist for every
/// specialization, admitted or not, not just the ones that succeed.
#[derive(Clone, Debug)]
pub struct SpecializationCandidate {
    pub id: String,
    pub application_id: String,
    pub kind: SpecializationKind,
    /// The ops sequence this specialization stands in for, run
    /// unmodified through the ordinary capability path. Kept verbatim so
    /// a rollback always has a known-good path to fall back to (spec
    /// S18) without having to reconstruct it.
    pub baseline_path: Seq,
    pub stage: LifecycleStage,
    pub admission_reason: Option<String>,
    pub rollback_reason: Option<String>,
    /// Count of consecutive successful shadow validations since the last
    /// reset (a mismatch, or an equivalent-but-not-cheaper sample,
    /// zeroes this).
    pub consecutive_matches: usize,
    pub total_validations: usize,
    pub total_mismatches: usize,
    /// Running-average measured gain in nanoseconds, signed: positive
    /// means cheaper than baseline, negative means the "optimization"
    /// was actually slower. Never hidden or floored at zero -- a real
    /// regression must be visible in this field, not laundered away.
    pub measured_gain_ns: i64,
    pub rollback_count: usize,
}

impl SpecializationCandidate {
    pub fn propose(id: impl Into<String>, application_id: impl Into<String>, kind: SpecializationKind, baseline_path: Seq) -> Self {
        Self {
            id: id.into(),
            application_id: application_id.into(),
            kind,
            baseline_path,
            stage: LifecycleStage::Observed,
            admission_reason: None,
            rollback_reason: None,
            consecutive_matches: 0,
            total_validations: 0,
            total_mismatches: 0,
            measured_gain_ns: 0,
            rollback_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_freshly_proposed_candidate_starts_observed_with_no_evidence() {
        let c = SpecializationCandidate::propose(
            "spec-1",
            "nginx",
            SpecializationKind::ReadAvoidance {
                path: "cfg.json".to_string(),
            },
            vec!["fs.read".to_string()],
        );
        assert_eq!(c.stage, LifecycleStage::Observed);
        assert!(c.admission_reason.is_none());
        assert!(c.rollback_reason.is_none());
        assert_eq!(c.total_validations, 0);
        assert_eq!(c.measured_gain_ns, 0);
    }

    #[test]
    fn describe_names_each_mechanism_distinctly() {
        let fusion = SpecializationKind::TransformFusion {
            stages: vec!["transform.extract".to_string(), "transform.summarize".to_string()],
        };
        let avoidance = SpecializationKind::ReadAvoidance {
            path: "in.csv".to_string(),
        };
        assert_eq!(fusion.describe(), "fuse[transform.extract->transform.summarize]");
        assert_eq!(avoidance.describe(), "avoid_read[in.csv]");
        assert_ne!(fusion.describe(), avoidance.describe());
    }
}
