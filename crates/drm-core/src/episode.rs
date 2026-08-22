use crate::identity::ExecutionContext;
use crate::vocabulary::Seq;

/// One unit of planning/execution work: a hierarchical execution identity
/// paired with the capability sequence it wants to run, plus enough
/// addressing information for an executor to act on it.
#[derive(Clone, Debug, Default)]
pub struct Episode {
    pub idx: usize,
    /// Which host/user/application/workload/task produced this episode.
    /// Planning identity (what Phase 1 called `task`) is `ctx.task_id`;
    /// see [`Episode::task`] for the common-case accessor.
    pub ctx: ExecutionContext,
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
    /// The task identity string -- `ctx.task_id`. Most planning logic
    /// only ever needs this, not the rest of the hierarchy.
    pub fn task(&self) -> &str {
        &self.ctx.task_id
    }

    /// Single-application, single-workload construction: the common case
    /// for tests, benchmarks, and callers that don't yet distinguish a
    /// task from its workload family.
    pub fn new(idx: usize, task: impl Into<String>, phase: impl Into<String>, ops: Seq) -> Self {
        let task = task.into();
        Self {
            idx,
            ctx: ExecutionContext::simple("default", task),
            phase: phase.into(),
            ops,
            source: String::new(),
            output: String::new(),
            url_path: String::new(),
            ancestral: false,
        }
    }

    /// Full construction with an explicit hierarchical identity.
    pub fn with_ctx(idx: usize, ctx: ExecutionContext, phase: impl Into<String>, ops: Seq) -> Self {
        Self {
            idx,
            ctx,
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
