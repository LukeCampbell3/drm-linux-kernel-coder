//! `Registry`: one [`HybridPlanner`] per `application_id`, plus a
//! [`GlobalVocabulary`] that only admits structure independently proven
//! useful across multiple applications.
//!
//! # Why not four independently-scored vocabularies?
//!
//! The spec's conceptual model is
//! `V_i = V_global ∪ V_application_i ∪ V_workload_i ∪ V_provisional_i`.
//! A literal reading would run four separate MDL-scoring engines per
//! application. That is a much larger and more failure-prone machine than
//! this crate's existing, tested `HybridPlanner` -- and workload identity
//! turns out not to need its own scoring engine to be useful: within one
//! application, every workload already shares the *same* permanent and
//! provisional vocabulary (one `HybridPlanner` instance per application),
//! so "did this transfer beyond its birth workload?" is answerable by
//! recording which `workload_id`s actually used a word, not by giving
//! each workload its own vocabulary. Workload is a **transfer-evidence
//! dimension** on [`WordMeta`], not a fifth scoring tier. This still
//! answers every question the spec's §3 requires, with materially less
//! new state and fewer new failure modes.
//!
//! # Why cross-application transfer can't be "word reuse"
//!
//! Each application's vocabulary lives in its own `HybridPlanner`, in its
//! own name space (`d001` in application A and `d001` in application B
//! are unrelated words). A word born in A literally cannot be *used* by
//! B's compressor -- B doesn't have that definition. So "did this
//! transfer beyond its birth application?" cannot be observed as reuse;
//! it has to be observed as **independent emergence of the same
//! underlying capability pattern in more than one application**. That is
//! exactly what `consolidate()`'s cross-application index does: it
//! watches every application's newly-admitted *permanent* words, and any
//! raw capability pattern that becomes permanent in
//! `>= promotion_threshold_apps` distinct applications is promoted to
//! [`GlobalVocabulary`] and backfilled into every application (present
//! and future) -- which is what actually gives it the ability to reduce
//! planning cost everywhere, not just where it was born.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::episode::{Episode, PlanMetrics};
use crate::hybrid::HybridPlanner;
use crate::identity::ExecutionContext;
use crate::lifecycle::LifecycleStage;
use crate::vocabulary::Seq;

/// Everything the runtime knows about one learned word -- symbolic
/// vocabulary word today, potentially also an executable specialization
/// once `drm-opt` attaches one. Answers "which application produced
/// this, which workloads/tasks use it, is it temporary or permanent."
#[derive(Clone, Debug)]
pub struct WordMeta {
    /// The execution context of the episode that caused this word's
    /// admission. For provisional words this is exact (the triggering
    /// episode is known at admission time); for permanent words grown
    /// via the deferred MDL pass it is the most recent context observed
    /// for this application at the time growth ran -- a reasonable, but
    /// approximate, birth attribution, documented as such rather than
    /// silently treated as exact.
    pub birth: ExecutionContext,
    pub stage: LifecycleStage,
    pub created_step: u64,
    pub last_use_step: u64,
    pub usage_count: u64,
    pub used_by_tasks: HashSet<String>,
    pub used_by_workloads: HashSet<String>,
    pub admission_evidence: String,
    pub expiry_evidence: Option<String>,
}

impl WordMeta {
    fn new(birth: ExecutionContext, stage: LifecycleStage, step: u64, evidence: impl Into<String>) -> Self {
        let mut used_by_tasks = HashSet::new();
        let mut used_by_workloads = HashSet::new();
        used_by_tasks.insert(birth.task_id.clone());
        used_by_workloads.insert(birth.workload_id.clone());
        Self {
            birth,
            stage,
            created_step: step,
            last_use_step: step,
            usage_count: 0,
            used_by_tasks,
            used_by_workloads,
            admission_evidence: evidence.into(),
            expiry_evidence: None,
        }
    }

    fn record_use(&mut self, ctx: &ExecutionContext, step: u64) {
        self.last_use_step = step;
        self.usage_count += 1;
        self.used_by_tasks.insert(ctx.task_id.clone());
        self.used_by_workloads.insert(ctx.workload_id.clone());
    }

    /// The transfer breadth this word has actually demonstrated within
    /// its own application: same task only, or spread across multiple
    /// tasks/workloads. (Cross-application breadth is tracked
    /// separately -- see module docs -- since it can't be observed as
    /// reuse of this same word.)
    pub fn transferred_within_application(&self) -> bool {
        self.used_by_workloads.len() > 1 || self.used_by_tasks.len() > 1
    }
}

/// One application's learned state: its own permanent+provisional
/// vocabulary (via the existing, unmodified [`HybridPlanner`]) plus
/// per-word metadata the base engine doesn't itself track.
pub struct AppState {
    pub planner: HybridPlanner,
    /// Keyed by vocabulary word name (`d###` permanent, `p###`
    /// provisional) -- unambiguous within one application's own
    /// `HybridPlanner` name space.
    pub word_meta: HashMap<String, WordMeta>,
    pub created_step: u64,
}

impl AppState {
    fn new(step: u64) -> Self {
        Self {
            planner: HybridPlanner::default(),
            word_meta: HashMap::new(),
            created_step: step,
        }
    }

    /// Empty-shelled `AppState` for `persistence::from_snapshot` to
    /// repopulate via `HybridPlanner`'s `restore_*` methods.
    pub(crate) fn restored(created_step: u64) -> Self {
        Self::new(created_step)
    }
}

/// Structure proven useful across multiple applications. Only
/// `Registry::consolidate` populates this -- never a single
/// application's own admission path.
pub struct GlobalVocabulary {
    pub vocab: crate::vocabulary::Vocabulary,
    pub word_meta: HashMap<String, WordMeta>,
    /// Minimum number of distinct applications a pattern must have
    /// independently promoted to *permanent* before it is promoted to
    /// global. Spec §2: "require evidence of cross-application usefulness
    /// before global promotion."
    pub promotion_threshold_apps: usize,
    counter: usize,
}

impl Default for GlobalVocabulary {
    fn default() -> Self {
        Self {
            vocab: crate::vocabulary::Vocabulary::new(),
            word_meta: HashMap::new(),
            promotion_threshold_apps: 2,
            counter: 0,
        }
    }
}

impl GlobalVocabulary {
    pub fn counter(&self) -> usize {
        self.counter
    }

    /// Rebuild from a persisted snapshot. `word_meta` is populated
    /// separately by `persistence::from_snapshot` (it needs the
    /// snapshot's own conversion helpers, which live in that module).
    pub(crate) fn restore(permanent: BTreeMap<String, Seq>, counter: usize, promotion_threshold_apps: usize) -> Self {
        let mut vocab = crate::vocabulary::Vocabulary::new();
        vocab.derived = permanent;
        vocab.counter = counter;
        Self {
            vocab,
            word_meta: HashMap::new(),
            promotion_threshold_apps,
            counter,
        }
    }
}

/// What `Registry::consolidate` did, for logging/CLI/testing.
#[derive(Debug, Default)]
pub struct ConsolidationReport {
    pub applications_consolidated: usize,
    pub words_admitted: usize,
    pub words_expired: usize,
    pub words_promoted_to_global: Vec<String>,
}

#[derive(Default)]
pub struct Registry {
    pub applications: BTreeMap<String, AppState>,
    pub global: GlobalVocabulary,
    /// Pattern -> set of application_ids that currently hold it as a
    /// *permanent* word. The cross-application locality index spec §9
    /// asks for; maintained incrementally by `consolidate`, never
    /// rescored from scratch on the foreground path.
    cross_app_index: HashMap<Seq, HashSet<String>>,
    pub step: u64,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_create_app(&mut self, application_id: &str) -> &mut AppState {
        if !self.applications.contains_key(application_id) {
            self.step += 1;
            let mut app = AppState::new(self.step);
            // A new application starts with every already-proven global
            // word -- this is what makes global promotion actually
            // useful, not just bookkeeping: it lowers planning cost for
            // applications that have never seen the pattern before.
            for (name, def) in self.global.vocab.derived.iter() {
                app.planner.base.vocab.derived.insert(name.clone(), def.clone());
            }
            self.applications.insert(application_id.to_string(), app);
        }
        self.applications.get_mut(application_id).unwrap()
    }

    /// Plan one episode within its application's own vocabulary. Cheap,
    /// foreground-safe: no cross-application scanning happens here (see
    /// `consolidate`).
    pub fn plan(&mut self, ctx: &ExecutionContext, ops: Seq, phase: impl Into<String>, ancestral: bool) -> PlanMetrics {
        self.step += 1;
        let step = self.step;
        let app = self.get_or_create_app(&ctx.application_id);
        let idx = app.planner.base.version + 1;
        let mut episode = Episode::with_ctx(idx, ctx.clone(), phase, ops);
        episode.ancestral = ancestral;
        let pm = app.planner.plan(&episode);

        // Usage/transfer bookkeeping: which words did the compressor
        // actually use to represent this episode?
        let compressed = app.planner.base.vocab.compress(&episode.ops);
        for tok in &compressed {
            if let Some(meta) = app.word_meta.get_mut(tok) {
                meta.record_use(ctx, step);
            }
        }
        pm
    }

    /// Off the foreground path (spec §9): drain each application's
    /// deferred vocabulary maintenance, diff its vocabulary before/after
    /// to detect newly-admitted or newly-expired words, update the
    /// cross-application index, and promote+backfill any pattern that
    /// has independently become permanent in enough applications.
    pub fn consolidate(&mut self) -> ConsolidationReport {
        let mut report = ConsolidationReport::default();
        let step = self.step;

        for (app_id, app) in self.applications.iter_mut() {
            let permanent_before: HashSet<String> = app.planner.base.vocab.derived.keys().cloned().collect();
            let provisional_before: HashSet<String> = provisional_names(&app.planner);

            app.planner.consolidate_pending();
            report.applications_consolidated += 1;

            let permanent_after: HashSet<String> = app.planner.base.vocab.derived.keys().cloned().collect();
            let provisional_after: HashSet<String> = provisional_names(&app.planner);

            let last_ctx = app
                .word_meta
                .values()
                .max_by_key(|m| m.last_use_step)
                .map(|m| m.birth.clone())
                .unwrap_or_else(|| ExecutionContext::simple(app_id.clone(), "unknown"));

            for name in permanent_after.difference(&permanent_before) {
                app.word_meta
                    .entry(name.clone())
                    .or_insert_with(|| WordMeta::new(last_ctx.clone(), LifecycleStage::Permanent, step, "mdl_growth_admitted"))
                    .stage = LifecycleStage::Permanent;
                report.words_admitted += 1;
                if let Some(def) = app.planner.base.vocab.derived.get(name) {
                    self.cross_app_index.entry(def.clone()).or_default().insert(app_id.clone());
                }
            }
            for name in provisional_after.difference(&provisional_before) {
                app.word_meta
                    .entry(name.clone())
                    .or_insert_with(|| WordMeta::new(last_ctx.clone(), LifecycleStage::Provisional, step, "provisional_admitted"));
                report.words_admitted += 1;
            }
            for name in provisional_before.difference(&provisional_after) {
                if let Some(meta) = app.word_meta.get_mut(name) {
                    if meta.stage != LifecycleStage::Permanent {
                        meta.stage = LifecycleStage::Expired;
                        meta.expiry_evidence = Some("provisional grace period elapsed unused".to_string());
                        report.words_expired += 1;
                    }
                }
            }
        }

        // Global promotion sweep: any pattern permanent in enough
        // distinct applications, and not already global, gets promoted
        // and backfilled everywhere.
        let threshold = self.global.promotion_threshold_apps;
        let candidates: Vec<(Seq, HashSet<String>)> = self
            .cross_app_index
            .iter()
            .filter(|(pattern, apps)| apps.len() >= threshold && !self.global.vocab.derived.values().any(|d| d == *pattern))
            .map(|(p, a)| (p.clone(), a.clone()))
            .collect();

        for (pattern, apps) in candidates {
            self.global.counter += 1;
            let name = format!("g{:03}", self.global.counter);
            self.global.vocab.derived.insert(name.clone(), pattern.clone());
            let birth_app = apps.iter().min().cloned().unwrap_or_default();
            let mut meta = WordMeta::new(
                ExecutionContext::simple(birth_app, "cross-application"),
                LifecycleStage::Permanent,
                step,
                format!("promoted: permanent in {} applications", apps.len()),
            );
            meta.used_by_tasks = apps.clone();
            self.global.word_meta.insert(name.clone(), meta);
            report.words_promoted_to_global.push(name.clone());

            // Backfill into every current application so the promotion
            // actually lowers planning cost everywhere, not just where
            // it was discovered.
            for app in self.applications.values_mut() {
                app.planner
                    .base
                    .vocab
                    .derived
                    .entry(name.clone())
                    .or_insert_with(|| pattern.clone());
            }
        }

        report
    }

    pub fn application_ids(&self) -> Vec<String> {
        self.applications.keys().cloned().collect()
    }

    /// Recompute the cross-application pattern index from the current
    /// state of every application's permanent vocabulary. Used after
    /// `persistence::from_snapshot` restores applications directly
    /// (bypassing `consolidate`'s incremental diffing, which only sees
    /// *new* admissions, not a freshly-loaded whole vocabulary).
    pub(crate) fn rebuild_cross_app_index(&mut self) {
        self.cross_app_index.clear();
        for (app_id, app) in &self.applications {
            for def in app.planner.base.vocab.derived.values() {
                self.cross_app_index.entry(def.clone()).or_default().insert(app_id.clone());
            }
        }
    }
}

fn provisional_names(planner: &HybridPlanner) -> HashSet<String> {
    planner.provisional_word_names()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(app: &str, workload: &str, task: &str) -> ExecutionContext {
        ExecutionContext::new("host", "user", app, workload, task)
    }

    fn motif() -> Seq {
        vec![
            "fs.read".into(),
            "transform.extract".into(),
            "transform.summarize".into(),
            "fs.write".into(),
            "notify.send".into(),
        ]
    }

    #[test]
    fn applications_get_independent_vocabularies() {
        let mut reg = Registry::new();
        for i in 0..5 {
            reg.plan(&ctx("app-a", "wl", &format!("t{i}")), motif(), "x", false);
        }
        reg.consolidate();
        assert!(!reg.applications.get("app-a").unwrap().planner.base.vocab.derived.is_empty());
        assert!(!reg.applications.contains_key("app-b"), "app-b was never used, must not exist");
    }

    #[test]
    fn cross_application_promotion_requires_multiple_apps() {
        let mut reg = Registry::new();
        reg.global.promotion_threshold_apps = 2;
        for i in 0..6 {
            reg.plan(&ctx("app-a", "wl", &format!("a{i}")), motif(), "x", false);
        }
        reg.consolidate();
        assert!(
            reg.global.vocab.derived.is_empty(),
            "one application alone must not reach global promotion"
        );

        for i in 0..6 {
            reg.plan(&ctx("app-b", "wl", &format!("b{i}")), motif(), "x", false);
        }
        let report = reg.consolidate();
        assert!(
            !reg.global.vocab.derived.is_empty(),
            "independent emergence in a second application must promote to global"
        );
        assert!(!report.words_promoted_to_global.is_empty());
    }

    #[test]
    fn global_promotion_backfills_into_every_application() {
        let mut reg = Registry::new();
        reg.global.promotion_threshold_apps = 2;
        for i in 0..6 {
            reg.plan(&ctx("app-a", "wl", &format!("a{i}")), motif(), "x", false);
        }
        for i in 0..6 {
            reg.plan(&ctx("app-b", "wl", &format!("b{i}")), motif(), "x", false);
        }
        reg.consolidate();
        let global_name = reg.global.vocab.derived.keys().next().cloned().unwrap();

        // A brand-new application must be seeded with the global word.
        reg.get_or_create_app("app-c");
        assert!(reg.applications["app-c"].planner.base.vocab.derived.contains_key(&global_name));
    }
}
