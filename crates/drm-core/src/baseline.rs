//! Trivial comparison planners used to demonstrate that [`crate::planner::DrmPlanner`]
//! beats naive replanning and naive caching, not just naive replanning.

use std::collections::HashMap;

use crate::episode::{Episode, PlanMetrics};
use crate::planner::DrmPlanner;
use crate::vocabulary::Seq;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineKind {
    /// Replans every episode from scratch; every token is charged.
    Stateless,
    /// Exact-match cache: free on an identical repeat, full cost otherwise.
    TemplateCache,
    /// Exact-match cache with diff-based local repair on drift.
    CheckpointReplay,
}

#[derive(Default)]
pub struct Baseline {
    pub kind: Option<BaselineKind>,
    pub seen: HashMap<String, Seq>,
}

impl Baseline {
    pub fn new(kind: BaselineKind) -> Self {
        Self {
            kind: Some(kind),
            seen: HashMap::new(),
        }
    }

    pub fn structure_bytes(&self) -> usize {
        self.seen
            .values()
            .map(|v| v.iter().map(|s| s.len() + 1).sum::<usize>())
            .sum::<usize>()
            + self.seen.keys().map(|k| k.len() + 1).sum::<usize>()
    }

    pub fn plan(&mut self, ep: &Episode) -> PlanMetrics {
        let mut m = PlanMetrics::default();
        match self.kind.unwrap() {
            BaselineKind::Stateless => {
                m.semantic = ep.ops.len();
            }
            BaselineKind::TemplateCache | BaselineKind::CheckpointReplay => {
                match self.seen.get(&ep.task) {
                    None => {
                        m.semantic = ep.ops.len();
                        m.structural_change = 1;
                    }
                    Some(old) if old == &ep.ops => {
                        m.semantic = 1;
                    }
                    Some(old) => {
                        if self.kind.unwrap() == BaselineKind::CheckpointReplay {
                            let delta = DrmPlanner::diff_middle(old, &ep.ops);
                            m.semantic = 1usize.max(delta.len());
                            m.local_repair = 1;
                        } else {
                            m.semantic = ep.ops.len();
                            m.structural_change = 1;
                        }
                    }
                }
                if ep.ancestral && self.kind.unwrap() == BaselineKind::CheckpointReplay {
                    m.recovery = 1;
                }
            }
        }
        self.seen.insert(ep.task.clone(), ep.ops.clone());
        m.structure_bytes = self.structure_bytes();
        m
    }
}
