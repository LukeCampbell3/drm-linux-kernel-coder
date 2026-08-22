//! The base episodic planner: an LRU-bounded "active" working set backed by
//! unbounded "history", growing a permanent [`Vocabulary`] whenever a
//! recurring subsequence provably shrinks the corpus's encoded size (a
//! minimum-description-length admission rule).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::capability::ROOT;
use crate::episode::{Episode, PlanMetrics};
use crate::vocabulary::{Seq, Vocabulary};

#[derive(Debug)]
pub struct DrmPlanner {
    pub vocab: Vocabulary,
    pub active_cap: usize,
    pub mdl_threshold: isize,
    pub active: HashMap<String, Seq>,
    pub lru: VecDeque<String>,
    pub history: HashMap<String, Seq>,
    pub history_version: HashMap<String, usize>,
    pub subseq_users: HashMap<Seq, HashSet<String>>,
    pub version: usize,
}

impl Default for DrmPlanner {
    fn default() -> Self {
        Self::new(8, 3)
    }
}

impl DrmPlanner {
    pub fn new(active_cap: usize, mdl_threshold: isize) -> Self {
        Self {
            vocab: Vocabulary::new(),
            active_cap,
            mdl_threshold,
            active: HashMap::new(),
            lru: VecDeque::new(),
            history: HashMap::new(),
            history_version: HashMap::new(),
            subseq_users: HashMap::new(),
            version: 0,
        }
    }

    pub fn touch(&mut self, task: &str, seq: &[String]) {
        self.active.insert(task.to_string(), seq.to_vec());
        self.lru.retain(|x| x != task);
        self.lru.push_back(task.to_string());
        while self.lru.len() > self.active_cap {
            if let Some(old) = self.lru.pop_front() {
                self.active.remove(&old);
            }
        }
    }

    /// Record every contiguous subsequence of length 2..=5 in `seq` as a
    /// growth candidate, returning the set touched by this episode.
    pub fn note_subseqs(&mut self, task: &str, seq: &[String]) -> HashSet<Seq> {
        let mut touched = HashSet::new();
        let max_len = 5usize.min(seq.len());
        for len in 2..=max_len {
            for start in 0..=(seq.len() - len) {
                let cand = seq[start..start + len].to_vec();
                self.subseq_users.entry(cand.clone()).or_default().insert(task.to_string());
                touched.insert(cand);
            }
        }
        touched
    }

    pub fn corpus_cost(&self, extra: Vec<(String, Seq)>) -> usize {
        self.history
            .values()
            .map(|s| self.vocab.compress_with(s, extra.clone()).len())
            .sum()
    }

    /// The MDL admission rule: among all candidate subsequences used by two
    /// or more tasks, promote the single best-scoring one to a new
    /// permanent vocabulary word if -- and only if -- doing so provably
    /// reduces the total encoded size of the history corpus by at least
    /// `mdl_threshold` tokens net of the new definition's own cost.
    pub fn maybe_grow(&mut self, candidates: &HashSet<Seq>) -> usize {
        if candidates.is_empty() {
            return 0;
        }
        let baseline = self.corpus_cost(Vec::new());
        let existing: HashSet<Seq> = self.vocab.derived.keys().filter_map(|k| self.vocab.expand_symbol(k).ok()).collect();
        let mut best: Option<((isize, usize, usize), Seq)> = None;
        for cand in candidates {
            let Some(userset) = self.subseq_users.get(cand) else { continue };
            if userset.len() < 2 || existing.contains(cand) {
                continue;
            }
            let definition = self.vocab.compress(cand);
            if definition.len() <= 1 {
                continue;
            }
            let new_cost = self.corpus_cost(vec![("__new__".to_string(), cand.clone())]);
            let gain = baseline as isize - new_cost as isize - (definition.len() as isize + 1);
            if gain < self.mdl_threshold {
                continue;
            }
            let key = (gain, userset.len(), cand.len());
            if best.as_ref().map(|x| key > x.0).unwrap_or(true) {
                best = Some((key, definition));
            }
        }
        if let Some((_, definition)) = best {
            self.vocab.counter += 1;
            self.vocab.derived.insert(format!("d{:03}", self.vocab.counter), definition);
            1
        } else {
            0
        }
    }

    /// The changed middle region between `old` and `new`, trimmed of
    /// matching prefix and suffix.
    pub fn diff_middle(old: &[String], new: &[String]) -> Seq {
        let mut prefix = 0usize;
        while prefix < old.len().min(new.len()) && old[prefix] == new[prefix] {
            prefix += 1;
        }
        let mut suffix = 0usize;
        while suffix < (old.len() - prefix).min(new.len() - prefix) && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix] {
            suffix += 1;
        }
        let end = if suffix == 0 { new.len() } else { new.len() - suffix };
        new[prefix..end].to_vec()
    }

    pub fn structure_bytes(&self) -> usize {
        let mut n: usize = ROOT.iter().map(|s| s.len()).sum();
        for (k, v) in &self.vocab.derived {
            n += k.len() + 1 + v.iter().map(|s| s.len() + 1).sum::<usize>();
        }
        for (k, v) in &self.active {
            n += k.len() + 1 + v.iter().map(|s| s.len() + 1).sum::<usize>();
        }
        for (k, v) in &self.history_version {
            n += k.len() + v.to_string().len() + 2;
        }
        n
    }

    fn finish_metrics(&self, mut m: PlanMetrics) -> PlanMetrics {
        m.derived = self.vocab.derived.len();
        m.active = self.active.len();
        m.uniform = self.vocab.audit();
        let depths: Vec<usize> = self.vocab.derived.keys().filter_map(|k| self.vocab.depth(k).ok()).collect();
        if !depths.is_empty() {
            m.avg_depth = depths.iter().sum::<usize>() as f64 / depths.len() as f64;
            m.max_depth = *depths.iter().max().unwrap();
        }
        m.structure_bytes = self.structure_bytes();
        m
    }

    /// Advance the planner by one episode, returning what it cost.
    pub fn plan(&mut self, ep: &Episode) -> PlanMetrics {
        let task = ep.task();
        let mut m = PlanMetrics::default();
        if let Some(old) = self.active.get(task).cloned() {
            if old == ep.ops {
                m.semantic = 1;
            } else {
                let delta = Self::diff_middle(&old, &ep.ops);
                m.semantic = 1usize.max(self.vocab.compress(&delta).len());
                m.local_repair = 1;
                m.structural_change += 1;
            }
        } else if let Some(old) = self.history.get(task).cloned() {
            if ep.ancestral {
                m.recovery = 1;
                m.semantic = 1usize.max(self.vocab.compress(&ep.ops).len());
                m.structural_change += 1;
            } else if old != ep.ops {
                let delta = Self::diff_middle(&old, &ep.ops);
                m.semantic = 1usize.max(self.vocab.compress(&delta).len());
                m.local_repair = 1;
                m.structural_change += 1;
            } else {
                m.semantic = 1;
            }
        } else {
            m.semantic = 1usize.max(self.vocab.compress(&ep.ops).len());
            m.structural_change += 1;
        }

        let new_structural_evidence = self.history.get(task).map(|old| old != &ep.ops).unwrap_or(true);
        self.version += 1;
        self.history.insert(task.to_string(), ep.ops.clone());
        self.history_version.insert(task.to_string(), self.version);
        if new_structural_evidence {
            let touched = self.note_subseqs(task, &ep.ops);
            m.structural_change += self.maybe_grow(&touched);
        }
        self.touch(task, &ep.ops);
        self.finish_metrics(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(xs: &[&str]) -> Seq {
        xs.iter().map(|x| x.to_string()).collect()
    }

    fn ep(idx: usize, task: &str, ops: Seq, ancestral: bool) -> Episode {
        Episode {
            idx,
            ctx: crate::ExecutionContext::simple("test-app", task),
            phase: "x".into(),
            ops,
            source: "x".into(),
            output: "y".into(),
            url_path: "/".into(),
            ancestral,
        }
    }

    #[test]
    fn repeat_of_active_task_costs_one_decision() {
        let mut p = DrmPlanner::new(8, 3);
        let e = ep(1, "t", s(&["fs.read", "transform.summarize", "fs.write"]), false);
        p.plan(&e);
        let m = p.plan(&e);
        assert_eq!(m.semantic, 1);
        assert_eq!(m.local_repair, 0);
    }

    #[test]
    fn ancestral_recovery_is_one_shot_after_forward_integration() {
        let mut p = DrmPlanner::new(1, 3);
        let e = ep(1, "old", s(&["fs.read", "transform.summarize", "fs.write"]), false);
        p.plan(&e);
        p.plan(&ep(2, "other", s(&["fs.read", "transform.summarize", "fs.write"]), false));
        let recovered = ep(3, "old", e.ops.clone(), true);
        let m1 = p.plan(&recovered);
        assert_eq!(m1.recovery, 1);
        let m2 = p.plan(&ep(4, "old", e.ops.clone(), false));
        assert_eq!(m2.recovery, 0);
        assert_eq!(m2.semantic, 1);
    }

    #[test]
    fn shared_motif_across_tasks_grows_permanent_vocabulary() {
        let mut p = DrmPlanner::new(8, 1);
        let motif = s(&["fs.read", "transform.extract", "transform.summarize", "fs.write", "notify.send"]);
        for i in 0..4 {
            p.plan(&ep(i, &format!("task_{i}"), motif.clone(), false));
        }
        assert!(
            !p.vocab.derived.is_empty(),
            "expected at least one derived word to form from a shared motif"
        );
        assert!(p.vocab.audit());
    }

    #[test]
    fn active_cache_evicts_by_lru_capacity() {
        let mut p = DrmPlanner::new(2, 3);
        p.plan(&ep(1, "a", s(&["fs.read", "fs.write"]), false));
        p.plan(&ep(2, "b", s(&["fs.read", "fs.write"]), false));
        p.plan(&ep(3, "c", s(&["fs.read", "fs.write"]), false));
        assert_eq!(p.active.len(), 2);
        assert!(!p.active.contains_key("a"));
        assert!(p.history.contains_key("a"), "evicted task must still be recoverable from history");
    }
}
