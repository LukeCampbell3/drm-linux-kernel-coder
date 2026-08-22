//! The staged developmental lifecycle every learned structure -- a
//! symbolic vocabulary word or an executable specialization -- passes
//! through. A learned motif is never automatically an executable
//! optimization: the runtime first establishes that the structure
//! *exists* (Observed/Candidate/Provisional), then independently
//! establishes whether optimizing it is safe and useful
//! (Validating/Verified), before it is trusted long-term (Permanent).
//!
//! Symbolic vocabulary words (drm-core's own concern) normally only ever
//! reach `Provisional` or `Permanent` -- they never have an executable
//! specialization attached, so `Validating`/`Verified` don't apply to
//! them. Executable specializations (drm-opt's concern) are the objects
//! that actually drive `Validating -> Verified`. Both share this one
//! state machine and its transition legality so "was this ever
//! rejected/rolled back" means the same thing everywhere in the system,
//! per the requirement that primitive capability, derived vocabulary
//! word, and verified executable specialization stay distinct objects
//! that nonetheless share one notion of developmental status.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LifecycleStage {
    /// A candidate structure has been seen at least once. Not yet
    /// scored for admission.
    Observed,
    /// Seen enough / by enough distinct users to be scored for
    /// admission (drm-core's existing `pscore`/MDL-gain thresholds).
    Candidate,
    /// Admitted as a fast-forming, capped, grace-period-expiring
    /// symbolic abstraction (today's `HybridPlanner` provisional tier),
    /// or promoted to the conservative permanent tier -- both symbolic,
    /// no executable specialization implied.
    Provisional,
    /// An executable specialization has been proposed for this
    /// structure and is being shadow-run against the baseline path to
    /// establish behavioral equivalence (drm-opt).
    Validating,
    /// Equivalence established; the specialization is live and still
    /// periodically shadow-sampled.
    Verified,
    /// Required strong, sustained evidence (symbolic: passed permanent
    /// MDL admission; executable: verified and stable over enough
    /// samples). The end state for something the runtime trusts
    /// long-term -- but even a Permanent specialization can still be
    /// rolled back if drift is caught later.
    Permanent,
    /// Failed admission scoring. Terminal.
    Rejected,
    /// A provisional word aged out unused/non-transferable. Terminal.
    Expired,
    /// An equivalence check failed, at any point after Validating
    /// began -- including for a previously Verified/Permanent
    /// specialization. Always logged with the failing episode. Terminal
    /// for that specialization; the underlying symbolic word (if any)
    /// is unaffected and simply stops being specialized.
    RolledBack,
}

impl LifecycleStage {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            LifecycleStage::Rejected | LifecycleStage::Expired | LifecycleStage::RolledBack
        )
    }

    /// True for a stage where the structure is currently trusted for use
    /// (symbolically, executably, or both).
    pub fn is_active(self) -> bool {
        matches!(
            self,
            LifecycleStage::Provisional | LifecycleStage::Validating | LifecycleStage::Verified | LifecycleStage::Permanent
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IllegalTransition {
    pub from: LifecycleStage,
    pub to: LifecycleStage,
}

impl std::fmt::Display for IllegalTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "illegal lifecycle transition: {:?} -> {:?}", self.from, self.to)
    }
}

impl std::error::Error for IllegalTransition {}

/// Validate (and, on success, return) a lifecycle transition. Guards
/// against invalid jumps (e.g. `Rejected -> Permanent`) creeping in as
/// the state machine is driven from multiple places (drm-core admission,
/// drm-opt validation, drmd's `reset` command).
pub fn transition(from: LifecycleStage, to: LifecycleStage) -> Result<LifecycleStage, IllegalTransition> {
    use LifecycleStage::*;
    let legal = matches!(
        (from, to),
        (Observed, Candidate)
            | (Observed, Rejected)
            | (Candidate, Provisional)
            | (Candidate, Rejected)
            | (Provisional, Validating)
            | (Provisional, Permanent)
            | (Provisional, Expired)
            | (Validating, Verified)
            | (Validating, RolledBack)
            | (Verified, Permanent)
            | (Verified, RolledBack)
            | (Permanent, RolledBack)
    );
    if legal {
        Ok(to)
    } else {
        Err(IllegalTransition { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LifecycleStage::*;

    #[test]
    fn full_symbolic_to_specialization_chain_is_legal() {
        let mut stage = Observed;
        for next in [Candidate, Provisional, Validating, Verified, Permanent] {
            stage = transition(stage, next).unwrap();
        }
        assert_eq!(stage, Permanent);
    }

    #[test]
    fn permanent_can_still_roll_back_on_drift() {
        assert_eq!(transition(Permanent, RolledBack), Ok(RolledBack));
    }

    #[test]
    fn illegal_jumps_are_rejected() {
        assert!(transition(Rejected, Permanent).is_err());
        assert!(transition(Observed, Permanent).is_err());
        assert!(transition(Expired, Provisional).is_err());
    }

    #[test]
    fn terminal_and_active_classification() {
        for s in [Rejected, Expired, RolledBack] {
            assert!(s.is_terminal());
            assert!(!s.is_active());
        }
        for s in [Provisional, Validating, Verified, Permanent] {
            assert!(!s.is_terminal());
            assert!(s.is_active());
        }
        for s in [Observed, Candidate] {
            assert!(!s.is_terminal());
            assert!(!s.is_active());
        }
    }
}
