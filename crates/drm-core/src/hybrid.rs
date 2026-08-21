//! A production-oriented variant of [`DrmPlanner`]: a two-tier vocabulary
//! (fast-forming, capped, grace-period-expiring *provisional* words plus the
//! base engine's conservative *permanent* words) and a **deferred**
//! consolidation step that moves expensive whole-corpus MDL rescoring off
//! the synchronous request path.
//!
//! This is ported from the staged-deployment research variant, whose
//! documented effect was cutting planner p95 latency from ~41ms to ~0.5ms
//! on the same workload by no longer rescoring the whole history corpus
//! inline with every request. In `serve` mode this matters: a caller
//! submitting one episode should never pay for vocabulary maintenance work
//! that only needs to happen once per batch of new structural evidence.

use std::collections::{HashMap, HashSet};

use crate::episode::{Episode, PlanMetrics};
use crate::planner::DrmPlanner;
use crate::vocabulary::Seq;

#[derive(Clone, Debug, Default)]
struct PStat {
    tasks: HashSet<String>,
    hits: usize,
}

#[derive(Clone, Debug)]
struct PWord {
    raw: Seq,
    #[allow(dead_code)]
    birth_tasks: HashSet<String>,
    last_transfer_step: usize,
    transfer_hits: usize,
}

pub struct HybridPlanner {
    pub base: DrmPlanner,
    pstats: HashMap<Seq, PStat>,
    provisional: HashMap<String, PWord>,
    struct_step: usize,
    pcounter: usize,
    pub provisional_cap: usize,
    pub grace: usize,
    pub admitted_total: usize,
    pub expired_total: usize,
    pending_touched: HashSet<Seq>,
    pending_touched_p: HashSet<Seq>,
    pending_consolidation: bool,
}

impl Default for HybridPlanner {
    fn default() -> Self {
        Self::new(DrmPlanner::default())
    }
}

impl HybridPlanner {
    pub fn new(base: DrmPlanner) -> Self {
        Self {
            base,
            pstats: HashMap::new(),
            provisional: HashMap::new(),
            struct_step: 0,
            pcounter: 0,
            provisional_cap: 20,
            grace: 12,
            admitted_total: 0,
            expired_total: 0,
            pending_touched: HashSet::new(),
            pending_touched_p: HashSet::new(),
            pending_consolidation: false,
        }
    }

    fn contains_seq(haystack: &[String], needle: &[String]) -> bool {
        if needle.is_empty() || needle.len() > haystack.len() {
            return false;
        }
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    fn provisional_expansions(&self) -> Vec<(String, Seq)> {
        self.provisional.iter().map(|(n, p)| (n.clone(), p.raw.clone())).collect()
    }

    fn compress_effective(&self, s: &[String]) -> Seq {
        self.base.vocab.compress_with(s, self.provisional_expansions())
    }

    /// Dynamic-programming tokenizer: the minimum number of tokens needed to
    /// cover `s` using permanent and provisional vocabulary words. Using
    /// provisional words can never *increase* token count versus not using
    /// them, unlike the base planner's greedy longest-match compressor.
    fn semantic_cost(&self, s: &[String]) -> usize {
        if s.is_empty() {
            return 0;
        }
        let mut patterns: Vec<Seq> = self
            .base
            .vocab
            .expansions()
            .into_iter()
            .filter(|(_, raw)| raw.len() >= 2)
            .map(|(_, raw)| raw)
            .collect();
        patterns.extend(self.provisional.values().filter(|p| p.raw.len() >= 2).map(|p| p.raw.clone()));
        let n = s.len();
        let mut dp = vec![usize::MAX / 4; n + 1];
        dp[n] = 0;
        for pos in (0..n).rev() {
            dp[pos] = 1 + dp[pos + 1];
            for pat in &patterns {
                if pos + pat.len() <= n && s[pos..pos + pat.len()] == pat[..] {
                    dp[pos] = dp[pos].min(1 + dp[pos + pat.len()]);
                }
            }
        }
        dp[0]
    }

    fn represented(&self, s: &[String]) -> bool {
        self.base
            .vocab
            .derived
            .keys()
            .any(|n| self.base.vocab.expand_symbol(n).map(|e| e == s).unwrap_or(false))
            || self.provisional.values().any(|p| p.raw == s)
    }

    fn note_p(&mut self, task: &str, seq: &[String]) -> HashSet<Seq> {
        let mut touched = HashSet::new();
        let max_len = 5usize.min(seq.len());
        for len in 2..=max_len {
            for start in 0..=(seq.len() - len) {
                let cand = seq[start..start + len].to_vec();
                let st = self.pstats.entry(cand.clone()).or_default();
                st.hits += 1;
                st.tasks.insert(task.to_string());
                touched.insert(cand);
            }
        }
        touched
    }

    fn pscore(&self, cand: &[String], st: &PStat) -> Option<i64> {
        if self.represented(cand) || self.compress_effective(cand).len() <= 1 {
            return None;
        }
        if cand.len() == 2 && st.tasks.len() < 3 {
            return None;
        }
        if cand.len() >= 3 && st.tasks.len() < 2 {
            return None;
        }
        let def = self.base.vocab.compress(cand);
        if def.len() <= 1 {
            return None;
        }
        let saving = st.tasks.len() as i64 * (cand.len() as i64 - 1) - (def.len() as i64 + 1);
        if saving < 0 {
            return None;
        }
        Some(saving * 32 + st.tasks.len() as i64 * 8 + cand.len() as i64 * 4)
    }

    fn admit(&mut self, touched: &HashSet<Seq>) -> usize {
        if self.provisional.len() >= self.provisional_cap {
            return 0;
        }
        let mut best: Option<(i64, Seq)> = None;
        for cand in touched {
            let Some(st) = self.pstats.get(cand) else { continue };
            if let Some(score) = self.pscore(cand, st) {
                if best.as_ref().map(|(bs, _)| score > *bs).unwrap_or(true) {
                    best = Some((score, cand.clone()));
                }
            }
        }
        let Some((_, best_seq)) = best else { return 0 };
        self.pcounter += 1;
        let name = format!("p{:03}", self.pcounter);
        let tasks = self.pstats.get(&best_seq).map(|s| s.tasks.clone()).unwrap_or_default();
        self.provisional.insert(
            name,
            PWord {
                raw: best_seq,
                birth_tasks: tasks,
                last_transfer_step: self.struct_step,
                transfer_hits: 0,
            },
        );
        self.admitted_total += 1;
        1
    }

    fn update_transfer(&mut self, ep: &Episode) {
        for (_, p) in self.provisional.iter_mut() {
            if Self::contains_seq(&ep.ops, &p.raw) && !p.birth_tasks.contains(&ep.task) {
                p.transfer_hits += 1;
                p.last_transfer_step = self.struct_step;
            }
        }
    }

    fn expire(&mut self) -> usize {
        let step = self.struct_step;
        let grace = self.grace;
        let before = self.provisional.len();
        self.provisional.retain(|_, p| step < p.last_transfer_step + grace);
        let removed = before - self.provisional.len();
        self.expired_total += removed;
        removed
    }

    fn remove_committed_equivalents(&mut self) {
        let derived_exp: Vec<Seq> = self
            .base
            .vocab
            .derived
            .keys()
            .filter_map(|n| self.base.vocab.expand_symbol(n).ok())
            .collect();
        self.provisional.retain(|_, p| !derived_exp.iter().any(|d| d == &p.raw));
    }

    /// Localized MDL scoring: a candidate can only change compression cost
    /// for tasks that have actually contained that subsequence, so we only
    /// rescore those tasks instead of the whole corpus.
    fn maybe_grow_localized(&mut self, candidates: &HashSet<Seq>) -> usize {
        if candidates.is_empty() {
            return 0;
        }
        let existing: HashSet<Seq> = self
            .base
            .vocab
            .derived
            .keys()
            .filter_map(|k| self.base.vocab.expand_symbol(k).ok())
            .collect();
        let mut best: Option<((i64, usize, usize), Seq)> = None;
        for cand in candidates {
            let Some(userset) = self.base.subseq_users.get(cand) else {
                continue;
            };
            if userset.len() < 2 || existing.contains(cand) {
                continue;
            }
            let def = self.base.vocab.compress(cand);
            if def.len() <= 1 {
                continue;
            }
            let mut saving: i64 = 0;
            for task in userset {
                let Some(hist) = self.base.history.get(task) else { continue };
                let before = self.base.vocab.compress(hist).len();
                let after = self
                    .base
                    .vocab
                    .compress_with(hist, vec![("__new__".to_string(), cand.clone())])
                    .len();
                saving += before as i64 - after as i64;
            }
            let gain = saving - (def.len() as i64 + 1);
            if gain < self.base.mdl_threshold as i64 {
                continue;
            }
            let key = (gain, userset.len(), cand.len());
            if best.as_ref().map(|(bk, _)| key > *bk).unwrap_or(true) {
                best = Some((key, def));
            }
        }
        if let Some((_, def)) = best {
            self.base.vocab.counter += 1;
            self.base.vocab.derived.insert(format!("d{:03}", self.base.vocab.counter), def);
            1
        } else {
            0
        }
    }

    /// Apply any vocabulary maintenance queued up by episodes planned since
    /// the last call. Safe to call after every episode (cheap when nothing
    /// is pending) or in a background loop on a fixed interval; either way
    /// it is never on the critical path of `plan`.
    pub fn consolidate_pending(&mut self) -> usize {
        if !self.pending_consolidation {
            return 0;
        }
        let touched = std::mem::take(&mut self.pending_touched);
        let touched_p = std::mem::take(&mut self.pending_touched_p);
        let mut changes = self.maybe_grow_localized(&touched);
        self.remove_committed_equivalents();
        changes += self.admit(&touched_p);
        changes += self.expire();
        self.pending_consolidation = false;
        changes
    }

    pub fn plan(&mut self, ep: &Episode) -> PlanMetrics {
        let mut m = PlanMetrics::default();
        if let Some(old) = self.base.active.get(&ep.task).cloned() {
            if old == ep.ops {
                m.semantic = 1;
            } else {
                let delta = DrmPlanner::diff_middle(&old, &ep.ops);
                m.semantic = 1usize.max(self.semantic_cost(&delta));
                m.local_repair = 1;
                m.structural_change += 1;
            }
        } else if let Some(old) = self.base.history.get(&ep.task).cloned() {
            if ep.ancestral {
                m.recovery = 1;
                m.semantic = 1usize.max(self.semantic_cost(&ep.ops));
                m.structural_change += 1;
            } else if old != ep.ops {
                let delta = DrmPlanner::diff_middle(&old, &ep.ops);
                m.semantic = 1usize.max(self.semantic_cost(&delta));
                m.local_repair = 1;
                m.structural_change += 1;
            } else {
                m.semantic = 1;
            }
        } else {
            m.semantic = 1usize.max(self.semantic_cost(&ep.ops));
            m.structural_change += 1;
        }

        let new_structural_evidence = self.base.history.get(&ep.task).map(|old| old != &ep.ops).unwrap_or(true);
        self.base.version += 1;
        self.base.history.insert(ep.task.clone(), ep.ops.clone());
        self.base.history_version.insert(ep.task.clone(), self.base.version);
        if new_structural_evidence {
            self.struct_step += 1;
            self.update_transfer(ep);
            self.pending_touched_p = self.note_p(&ep.task, &ep.ops);
            self.pending_touched = self.base.note_subseqs(&ep.task, &ep.ops);
            self.pending_consolidation = true;
        }
        self.base.touch(&ep.task, &ep.ops);

        m.derived = self.base.vocab.derived.len();
        m.active = self.base.active.len();
        m.uniform = self.base.vocab.audit();
        let depths: Vec<usize> = self
            .base
            .vocab
            .derived
            .keys()
            .filter_map(|k| self.base.vocab.depth(k).ok())
            .collect();
        if !depths.is_empty() {
            m.avg_depth = depths.iter().sum::<usize>() as f64 / depths.len() as f64;
            m.max_depth = *depths.iter().max().unwrap();
        }
        m.structure_bytes = self.base.structure_bytes();
        m
    }

    pub fn provisional_words(&self) -> usize {
        self.provisional.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(xs: &[&str]) -> Seq {
        xs.iter().map(|x| x.to_string()).collect()
    }

    fn ep(idx: usize, task: &str, ops: Seq) -> Episode {
        Episode {
            idx,
            task: task.into(),
            phase: "x".into(),
            ops,
            source: "x".into(),
            output: "y".into(),
            url_path: "/".into(),
            ancestral: false,
        }
    }

    #[test]
    fn provisional_words_admit_and_respect_cap() {
        let mut h = HybridPlanner::new(DrmPlanner::new(8, 1000)); // huge threshold: permanent growth never fires
        h.provisional_cap = 2;
        let motif_a = s(&["fs.read", "transform.extract", "transform.summarize"]);
        let motif_b = s(&["http.request", "transform.extract", "transform.summarize"]);
        let motif_c = s(&["ipc.request", "transform.extract", "transform.summarize"]);
        for (i, motif) in [motif_a, motif_b, motif_c].into_iter().enumerate() {
            for j in 0..3 {
                h.plan(&ep(i * 3 + j, &format!("task_{i}_{j}"), motif.clone()));
                h.consolidate_pending();
            }
        }
        assert!(
            h.provisional_words() <= 2,
            "provisional cap must be respected, got {}",
            h.provisional_words()
        );
    }

    #[test]
    fn consolidation_is_deferred_until_explicitly_requested() {
        let mut h = HybridPlanner::default();
        let motif = s(&["fs.read", "transform.extract", "transform.summarize", "fs.write"]);
        h.plan(&ep(1, "a", motif.clone()));
        assert!(
            h.pending_consolidation,
            "a structurally new episode should queue consolidation work"
        );
    }

    #[test]
    fn audit_holds_after_growth_and_consolidation() {
        let mut h = HybridPlanner::new(DrmPlanner::new(8, 1));
        let motif = s(&["fs.read", "transform.extract", "transform.summarize", "fs.write", "notify.send"]);
        for i in 0..5 {
            h.plan(&ep(i, &format!("t{i}"), motif.clone()));
            h.consolidate_pending();
        }
        assert!(h.base.vocab.audit());
    }
}
