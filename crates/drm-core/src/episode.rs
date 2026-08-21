use crate::vocabulary::Seq;

/// One unit of planning/execution work: a task identity paired with the
/// capability sequence it wants to run, plus enough addressing information
/// for an executor to act on it.
#[derive(Clone, Debug, Default)]
pub struct Episode {
    pub idx: usize,
    pub task: String,
    pub phase: String,
    pub ops: Seq,
    pub source: String,
    pub output: String,
    pub url_path: String,
    /// True when this episode is a deliberate replay of a task that has
    /// fallen out of the active working set and back into history --
    /// exercises the planner's ancestral-recovery path.
    pub ancestral: bool,
}

impl Episode {
    pub fn new(idx: usize, task: impl Into<String>, phase: impl Into<String>, ops: Seq) -> Self {
        Self {
            idx,
            task: task.into(),
            phase: phase.into(),
            ops,
            source: String::new(),
            output: String::new(),
            url_path: String::new(),
            ancestral: false,
        }
    }
}

/// Per-episode planning outcome.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanMetrics {
    /// Number of compressed decision tokens this episode cost the planner.
    pub semantic: usize,
    /// 1 if this episode was a one-shot recovery of a task that had aged
    /// out of the active set into history.
    pub recovery: usize,
    /// 1 if only a diffed middle region needed replanning.
    pub local_repair: usize,
    /// Count of structural events (new task, drift, vocabulary growth).
    pub structural_change: usize,
    pub derived: usize,
    pub active: usize,
    pub structure_bytes: usize,
    pub avg_depth: f64,
    pub max_depth: usize,
    pub uniform: bool,
}
