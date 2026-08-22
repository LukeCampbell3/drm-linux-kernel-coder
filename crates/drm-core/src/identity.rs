//! Hierarchical execution identity: which host, user, application,
//! workload, and task produced a given episode.
//!
//! Phase 1 tracked only a flat `task` string. That's enough to recognize
//! "this exact task recurred," but not enough to answer the questions the
//! adaptive execution layer needs: which application produced a pattern,
//! which workloads reuse it, and whether it ever transferred beyond the
//! context it was born in. [`ExecutionContext`] is that missing
//! attribution; [`TransferScope`] classifies one usage against a word's
//! birth context along the same axes the spec calls out (same task /
//! different task same workload / different workload same application /
//! different application). "Global" transfer -- beyond any one
//! application -- is not a pairwise classification; see
//! `registry::Registry`'s cross-application pattern index for how that's
//! actually established (independent emergence in multiple applications,
//! not mere reuse of one application's word).

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExecutionContext {
    /// The machine this execution ran on. Distinguishes a desktop's own
    /// history from a server fleet's, and multiple hosts in a fleet from
    /// each other, without requiring a central identity service.
    pub host_id: String,
    /// The user or service account this execution ran as. On a desktop
    /// this is the logged-in user; on a server it is typically the
    /// service/tenant identity (or "system" for a single-tenant service).
    pub user_scope: String,
    /// The application that produced this episode, e.g. "nginx",
    /// "report-worker", "vscode". Vocabulary is scoped per application
    /// (see `registry::AppState`) -- an application's learned structure
    /// never silently leaks into another's.
    pub application_id: String,
    /// The recurring task *family* this episode belongs to, e.g.
    /// "daily_report" or "api_get_user". Distinct from `task_id`: many
    /// task instances (different report dates, different request ids)
    /// share one workload identity.
    pub workload_id: String,
    /// The specific task instance identity -- what Phase 1 called `task`.
    pub task_id: String,
}

impl ExecutionContext {
    pub fn new(
        host_id: impl Into<String>,
        user_scope: impl Into<String>,
        application_id: impl Into<String>,
        workload_id: impl Into<String>,
        task_id: impl Into<String>,
    ) -> Self {
        Self {
            host_id: host_id.into(),
            user_scope: user_scope.into(),
            application_id: application_id.into(),
            workload_id: workload_id.into(),
            task_id: task_id.into(),
        }
    }

    /// Convenience constructor for callers that don't yet distinguish a
    /// task from its workload family (workload_id defaults to task_id) --
    /// e.g. ported Phase 1 call sites where every task was its own
    /// workload of one.
    pub fn simple(application_id: impl Into<String>, task_id: impl Into<String>) -> Self {
        let task_id = task_id.into();
        Self {
            host_id: "local".to_string(),
            user_scope: "default".to_string(),
            application_id: application_id.into(),
            workload_id: task_id.clone(),
            task_id,
        }
    }
}

/// How a usage context relates to a word's birth context, from most to
/// least local. Used to answer "did this transfer beyond its birth
/// workload? beyond its birth application?" per learned word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TransferScope {
    /// Reused by the exact task it was born from.
    SameTask,
    /// Reused by a different task, but the same workload family.
    DifferentTaskSameWorkload,
    /// Reused by a different workload, but the same application.
    DifferentWorkloadSameApplication,
    /// Observed in a different application entirely. Per spec, this is
    /// evidence toward global promotion, not proof of it -- promotion
    /// requires independent emergence in multiple applications, tracked
    /// by `registry::Registry`'s cross-application index.
    DifferentApplication,
}

impl TransferScope {
    pub fn classify(birth: &ExecutionContext, usage: &ExecutionContext) -> Self {
        if birth.application_id != usage.application_id {
            TransferScope::DifferentApplication
        } else if birth.workload_id != usage.workload_id {
            TransferScope::DifferentWorkloadSameApplication
        } else if birth.task_id != usage.task_id {
            TransferScope::DifferentTaskSameWorkload
        } else {
            TransferScope::SameTask
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_same_task() {
        let ctx = ExecutionContext::new("h", "u", "app", "wl", "t1");
        assert_eq!(TransferScope::classify(&ctx, &ctx), TransferScope::SameTask);
    }

    #[test]
    fn classify_orders_local_to_global() {
        let birth = ExecutionContext::new("h", "u", "app", "wl", "t1");
        let same_workload = ExecutionContext::new("h", "u", "app", "wl", "t2");
        let diff_workload = ExecutionContext::new("h", "u", "app", "wl2", "t3");
        let diff_app = ExecutionContext::new("h", "u", "app2", "wl9", "t9");

        assert_eq!(
            TransferScope::classify(&birth, &same_workload),
            TransferScope::DifferentTaskSameWorkload
        );
        assert_eq!(
            TransferScope::classify(&birth, &diff_workload),
            TransferScope::DifferentWorkloadSameApplication
        );
        assert_eq!(TransferScope::classify(&birth, &diff_app), TransferScope::DifferentApplication);

        // Ordering matches "most local to most general" so callers can e.g.
        // require at least DifferentWorkloadSameApplication as promotion evidence.
        assert!(TransferScope::SameTask < TransferScope::DifferentTaskSameWorkload);
        assert!(TransferScope::DifferentTaskSameWorkload < TransferScope::DifferentWorkloadSameApplication);
        assert!(TransferScope::DifferentWorkloadSameApplication < TransferScope::DifferentApplication);
    }
}
