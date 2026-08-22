//! Lifecycle-gated admission and rollback for specialization candidates.
//!
//! A candidate only ever reaches `Verified` after a run of consecutive
//! successful shadow validations with non-negative measured gain -- one
//! matching sample is not admission evidence (spec S7/S8: "cannot admit
//! if equivalence unknown," which a single sample effectively still is).
//! Any mismatch, at any stage from `Validating` onward -- including for an
//! already-`Verified`/`Permanent` specialization -- rolls it back
//! immediately and permanently under that id (spec S7's "rollback path");
//! drift never gets a second chance under the same id.

use std::collections::HashMap;

use drm_core::lifecycle::{transition, IllegalTransition, LifecycleStage};
use drm_core::vocabulary::Seq;

use crate::equivalence::EquivalenceResult;
use crate::specialization::{SpecializationCandidate, SpecializationKind};

/// How many consecutive matching shadow validations, each with
/// non-negative measured gain, a candidate needs before being promoted
/// from `Validating` to `Verified`. A smallest-rigorous-implementation
/// choice, not a tuned constant: large enough that a single lucky sample
/// can't admit a specialization, small enough that a benchmark run's
/// episode counts can actually exercise promotion.
pub const VALIDATION_THRESHOLD: usize = 3;

#[derive(Debug)]
pub enum AdmissionError {
    Illegal(IllegalTransition),
    UnknownCandidate(String),
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdmissionError::Illegal(e) => write!(f, "{e}"),
            AdmissionError::UnknownCandidate(id) => write!(f, "no specialization candidate with id `{id}`"),
        }
    }
}

impl std::error::Error for AdmissionError {}

impl From<IllegalTransition> for AdmissionError {
    fn from(e: IllegalTransition) -> Self {
        AdmissionError::Illegal(e)
    }
}

/// Holds every specialization candidate ever proposed for one registry
/// (spanning all applications; each candidate carries its own
/// `application_id` so per-application isolation/reset, spec S18, can
/// filter or drop by it without touching any other application's
/// specializations).
#[derive(Default)]
pub struct AdmissionLedger {
    candidates: HashMap<String, SpecializationCandidate>,
}

impl AdmissionLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a newly observed candidate structure. A repeat proposal
    /// under the same id is idempotent (returns the existing record
    /// unchanged) rather than resetting progress -- proposing is "this
    /// structure exists," not "start over."
    pub fn propose(
        &mut self,
        id: impl Into<String>,
        application_id: impl Into<String>,
        kind: SpecializationKind,
        baseline_path: Seq,
    ) -> &SpecializationCandidate {
        let id = id.into();
        self.candidates
            .entry(id.clone())
            .or_insert_with(|| SpecializationCandidate::propose(id, application_id, kind, baseline_path))
    }

    /// Advance a proposed candidate from `Observed` through `Candidate`
    /// and `Provisional` into `Validating` -- an executable specialization
    /// is now being shadow-run against the baseline. These are bookkeeping
    /// transitions with no equivalence evidence yet, so none of them touch
    /// `consecutive_matches`. A no-op (returns the current stage
    /// unchanged) if the candidate has already moved past `Observed`.
    pub fn begin_validating(&mut self, id: &str) -> Result<LifecycleStage, AdmissionError> {
        let c = self
            .candidates
            .get_mut(id)
            .ok_or_else(|| AdmissionError::UnknownCandidate(id.to_string()))?;
        if c.stage == LifecycleStage::Observed {
            c.stage = transition(c.stage, LifecycleStage::Candidate)?;
        }
        if c.stage == LifecycleStage::Candidate {
            c.stage = transition(c.stage, LifecycleStage::Provisional)?;
        }
        if c.stage == LifecycleStage::Provisional {
            c.stage = transition(c.stage, LifecycleStage::Validating)?;
        }
        Ok(c.stage)
    }

    /// Record one shadow-validation sample: an equivalence check result
    /// plus the measured gain (baseline cost minus optimized cost, in
    /// nanoseconds; positive is cheaper) for that sample. This is the one
    /// place lifecycle promotion and rollback actually happen.
    pub fn record_validation(&mut self, id: &str, result: &EquivalenceResult, gain_ns: i64) -> Result<LifecycleStage, AdmissionError> {
        let c = self
            .candidates
            .get_mut(id)
            .ok_or_else(|| AdmissionError::UnknownCandidate(id.to_string()))?;
        c.total_validations += 1;

        if !result.equivalent {
            c.total_mismatches += 1;
            c.consecutive_matches = 0;
            c.rollback_reason = Some(format!(
                "equivalence mismatch at validation #{}: baseline={:?} optimized={:?}",
                c.total_validations, result.baseline_output, result.optimized_output
            ));
            c.rollback_count += 1;
            c.stage = transition(c.stage, LifecycleStage::RolledBack)?;
            return Ok(c.stage);
        }

        // Equivalent, but a specialization that is *slower* than baseline
        // is not admissible on cost grounds even though it's behaviorally
        // safe -- it just doesn't advance the streak. Spec S8: minimize
        // cost *subject to* equivalence, not "equivalence alone is enough."
        if gain_ns < 0 {
            c.consecutive_matches = 0;
            return Ok(c.stage);
        }

        c.consecutive_matches += 1;
        // Running mean, recomputed exactly (not a decaying estimate) so it
        // stays auditable by hand against the raw per-episode metrics CSV.
        let n = c.total_validations as i64;
        c.measured_gain_ns += (gain_ns - c.measured_gain_ns) / n;

        if c.stage == LifecycleStage::Validating && c.consecutive_matches >= VALIDATION_THRESHOLD {
            c.stage = transition(c.stage, LifecycleStage::Verified)?;
            c.admission_reason = Some(format!(
                "{} consecutive equivalent, non-negative-gain validations (avg gain {}ns)",
                c.consecutive_matches, c.measured_gain_ns
            ));
        }
        Ok(c.stage)
    }

    /// Promote an already-`Verified` specialization to `Permanent` once it
    /// has accumulated at least `permanent_threshold` total validations --
    /// sustained evidence, not just the initial validation run (spec S5's
    /// "required strong, sustained evidence" for `Permanent`). A no-op if
    /// the candidate isn't currently `Verified` or hasn't yet met the
    /// threshold.
    pub fn promote_if_sustained(&mut self, id: &str, permanent_threshold: usize) -> Result<LifecycleStage, AdmissionError> {
        let c = self
            .candidates
            .get_mut(id)
            .ok_or_else(|| AdmissionError::UnknownCandidate(id.to_string()))?;
        if c.stage == LifecycleStage::Verified && c.total_validations >= permanent_threshold {
            c.stage = transition(c.stage, LifecycleStage::Permanent)?;
        }
        Ok(c.stage)
    }

    /// Force a rollback outside the normal per-sample path -- used when an
    /// operator, or a higher-level drift detector (a mature workload
    /// changing shape, spec S21's drift test), decides a specialization
    /// must stop being used immediately, without waiting for its next
    /// scheduled shadow sample.
    pub fn force_rollback(&mut self, id: &str, reason: impl Into<String>) -> Result<LifecycleStage, AdmissionError> {
        let c = self
            .candidates
            .get_mut(id)
            .ok_or_else(|| AdmissionError::UnknownCandidate(id.to_string()))?;
        c.rollback_reason = Some(reason.into());
        c.rollback_count += 1;
        c.stage = transition(c.stage, LifecycleStage::RolledBack)?;
        Ok(c.stage)
    }

    pub fn get(&self, id: &str) -> Option<&SpecializationCandidate> {
        self.candidates.get(id)
    }

    /// Specializations currently trusted for use (`Verified` or
    /// `Permanent`) -- what `drm-exec` should actually consult when
    /// deciding whether to take the specialized path for a given ops
    /// sequence.
    pub fn verified(&self) -> impl Iterator<Item = &SpecializationCandidate> {
        self.candidates
            .values()
            .filter(|c| matches!(c.stage, LifecycleStage::Verified | LifecycleStage::Permanent))
    }

    pub fn for_application<'a>(&'a self, application_id: &'a str) -> impl Iterator<Item = &'a SpecializationCandidate> {
        self.candidates.values().filter(move |c| c.application_id == application_id)
    }

    pub fn all(&self) -> impl Iterator<Item = &SpecializationCandidate> {
        self.candidates.values()
    }

    /// Remove every candidate belonging to `application_id` -- the
    /// specialization half of `drmd reset application:<id>` (spec S17).
    pub fn reset_application(&mut self, application_id: &str) {
        self.candidates.retain(|_, c| c.application_id != application_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::equivalence::EquivalenceResult;

    fn kind() -> SpecializationKind {
        SpecializationKind::TransformFusion {
            stages: vec!["transform.extract".to_string(), "transform.summarize".to_string()],
        }
    }

    fn matching(baseline: &str) -> EquivalenceResult {
        EquivalenceResult {
            equivalent: true,
            baseline_output: baseline.to_string(),
            optimized_output: baseline.to_string(),
        }
    }

    fn mismatching() -> EquivalenceResult {
        EquivalenceResult {
            equivalent: false,
            baseline_output: "a".to_string(),
            optimized_output: "b".to_string(),
        }
    }

    #[test]
    fn propose_is_idempotent_and_does_not_reset_progress() {
        let mut ledger = AdmissionLedger::new();
        ledger.propose("spec-1", "app-a", kind(), vec!["transform.extract".to_string()]);
        ledger.begin_validating("spec-1").unwrap();
        ledger.record_validation("spec-1", &matching("x"), 100).unwrap();
        assert_eq!(ledger.get("spec-1").unwrap().consecutive_matches, 1);

        // Re-proposing must not clobber the in-flight candidate.
        ledger.propose("spec-1", "app-a", kind(), vec!["transform.extract".to_string()]);
        assert_eq!(ledger.get("spec-1").unwrap().consecutive_matches, 1);
    }

    #[test]
    fn three_consecutive_positive_gain_matches_promote_to_verified() {
        let mut ledger = AdmissionLedger::new();
        ledger.propose("spec-1", "app-a", kind(), vec!["transform.extract".to_string()]);
        ledger.begin_validating("spec-1").unwrap();
        for _ in 0..VALIDATION_THRESHOLD - 1 {
            let stage = ledger.record_validation("spec-1", &matching("x"), 50).unwrap();
            assert_eq!(stage, LifecycleStage::Validating, "must not promote before the threshold is met");
        }
        let stage = ledger.record_validation("spec-1", &matching("x"), 50).unwrap();
        assert_eq!(stage, LifecycleStage::Verified);
        let c = ledger.get("spec-1").unwrap();
        assert!(c.admission_reason.is_some());
        assert_eq!(c.measured_gain_ns, 50);
    }

    #[test]
    fn a_single_mismatch_rolls_back_immediately_even_after_verification() {
        let mut ledger = AdmissionLedger::new();
        ledger.propose("spec-1", "app-a", kind(), vec!["transform.extract".to_string()]);
        ledger.begin_validating("spec-1").unwrap();
        for _ in 0..VALIDATION_THRESHOLD {
            ledger.record_validation("spec-1", &matching("x"), 50).unwrap();
        }
        assert_eq!(ledger.get("spec-1").unwrap().stage, LifecycleStage::Verified);
        ledger.promote_if_sustained("spec-1", VALIDATION_THRESHOLD).unwrap();
        assert_eq!(ledger.get("spec-1").unwrap().stage, LifecycleStage::Permanent);

        // Drift: a later sample stops matching. Even a Permanent
        // specialization must roll back instantly, per spec S7.
        let stage = ledger.record_validation("spec-1", &mismatching(), 50).unwrap();
        assert_eq!(stage, LifecycleStage::RolledBack);
        let c = ledger.get("spec-1").unwrap();
        assert!(c.rollback_reason.is_some());
        assert_eq!(c.rollback_count, 1);
        assert_eq!(c.total_mismatches, 1);
    }

    #[test]
    fn equivalent_but_slower_samples_never_advance_the_streak() {
        let mut ledger = AdmissionLedger::new();
        ledger.propose("spec-1", "app-a", kind(), vec!["transform.extract".to_string()]);
        ledger.begin_validating("spec-1").unwrap();
        for _ in 0..10 {
            let stage = ledger.record_validation("spec-1", &matching("x"), -10).unwrap();
            assert_eq!(
                stage,
                LifecycleStage::Validating,
                "a slower-but-equivalent specialization must never be admitted"
            );
        }
        assert_eq!(ledger.get("spec-1").unwrap().consecutive_matches, 0);
    }

    #[test]
    fn force_rollback_works_from_any_active_stage() {
        let mut ledger = AdmissionLedger::new();
        ledger.propose("spec-1", "app-a", kind(), vec!["transform.extract".to_string()]);
        ledger.begin_validating("spec-1").unwrap();
        let stage = ledger.force_rollback("spec-1", "operator requested disable").unwrap();
        assert_eq!(stage, LifecycleStage::RolledBack);
        assert_eq!(
            ledger.get("spec-1").unwrap().rollback_reason.as_deref(),
            Some("operator requested disable")
        );
    }

    #[test]
    fn unknown_candidate_id_is_reported_not_panicked_on() {
        let mut ledger = AdmissionLedger::new();
        assert!(matches!(
            ledger.begin_validating("missing"),
            Err(AdmissionError::UnknownCandidate(_))
        ));
        assert!(matches!(
            ledger.record_validation("missing", &matching("x"), 1),
            Err(AdmissionError::UnknownCandidate(_))
        ));
        assert!(matches!(
            ledger.force_rollback("missing", "x"),
            Err(AdmissionError::UnknownCandidate(_))
        ));
    }

    #[test]
    fn reset_application_only_drops_that_applications_candidates() {
        let mut ledger = AdmissionLedger::new();
        ledger.propose("spec-a", "app-a", kind(), vec![]);
        ledger.propose("spec-b", "app-b", kind(), vec![]);
        ledger.reset_application("app-a");
        assert!(ledger.get("spec-a").is_none());
        assert!(ledger.get("spec-b").is_some());
    }

    #[test]
    fn verified_iterator_includes_permanent_but_not_validating() {
        let mut ledger = AdmissionLedger::new();
        ledger.propose("spec-1", "app-a", kind(), vec![]);
        ledger.begin_validating("spec-1").unwrap();
        assert_eq!(ledger.verified().count(), 0);
        for _ in 0..VALIDATION_THRESHOLD {
            ledger.record_validation("spec-1", &matching("x"), 1).unwrap();
        }
        assert_eq!(ledger.verified().count(), 1);
    }
}
