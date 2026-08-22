//! The eight comparative execution engines (spec S14): five naive
//! baselines and three DRM configurations. Every engine executes the
//! exact same episode stream through its own `LiveExecutor` (a private
//! scratch work directory per engine, so none can see another's
//! filesystem state), so any measured difference is attributable to the
//! engine's own mechanism, not to a shared cache or shared I/O.
//!
//! DRM_A and DRM_B never skip real per-capability execution -- neither
//! `DrmPlanner` nor `Registry` has any notion of "don't actually run
//! this," only "name it more compactly." That is deliberate: it is what
//! keeps "representation compression" (`semantic`, `structure_bytes`)
//! and "actual runtime cost" (`wall_time_ns`) honestly independent, per
//! spec S2/S11. Only DRM_C, via the `SpecializationSet` attached to its
//! `LiveExecutor`, can ever skip real work -- and only after earning that
//! through validated equivalence, not by assumption.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use drm_core::registry::Registry;
use drm_core::{Baseline, BaselineKind, DrmPlanner, Episode, PlanMetrics, Seq};
use drm_exec::{make_fixtures, LiveExecutor, SpecializationSet};

use super::scenario::Motifs;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EngineKind {
    Baseline0Stateless,
    Baseline1ExactCache,
    Baseline2CheckpointReplay,
    Baseline3StaticMacros,
    Baseline4PerAppCache,
    DrmAPermanentOnly,
    DrmBProvisionalPermanent,
    DrmCSpecialized,
}

impl EngineKind {
    pub fn all() -> [EngineKind; 8] {
        [
            EngineKind::Baseline0Stateless,
            EngineKind::Baseline1ExactCache,
            EngineKind::Baseline2CheckpointReplay,
            EngineKind::Baseline3StaticMacros,
            EngineKind::Baseline4PerAppCache,
            EngineKind::DrmAPermanentOnly,
            EngineKind::DrmBProvisionalPermanent,
            EngineKind::DrmCSpecialized,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            EngineKind::Baseline0Stateless => "BASELINE_0_stateless",
            EngineKind::Baseline1ExactCache => "BASELINE_1_exact_cache",
            EngineKind::Baseline2CheckpointReplay => "BASELINE_2_checkpoint_replay",
            EngineKind::Baseline3StaticMacros => "BASELINE_3_static_macros",
            EngineKind::Baseline4PerAppCache => "BASELINE_4_per_app_cache",
            EngineKind::DrmAPermanentOnly => "DRM_A_permanent_only",
            EngineKind::DrmBProvisionalPermanent => "DRM_B_provisional_permanent",
            EngineKind::DrmCSpecialized => "DRM_C_specialized",
        }
    }
}

/// The fixed table of known-safe capability sequences BASELINE_3 treats
/// as pre-vetted, hand-authored shortcuts -- representing a competent
/// engineer who inspected these specific shapes once, decided they are
/// always safe to skip, and hardcoded that. Applies unconditionally from
/// episode 1 (no learning curve at all -- that is the point of a static
/// macro) and does *not* adapt when a shape drifts even slightly: drift
/// just falls through to full real execution, silently losing the
/// shortcut rather than risking wrong output on an unanticipated shape.
fn static_macro_table() -> Vec<Seq> {
    vec![
        Motifs::read_transform_write(),
        Motifs::hash_check(),
        Motifs::api_call(),
        Motifs::state_update(),
        Motifs::ipc_notify(),
    ]
}

/// What one engine actually did for one episode -- enough for the report
/// generator to build a `drm_observe::ExecutionMetrics` row without
/// reaching back into engine internals.
pub struct EpisodeOutcome {
    pub wall_time_ns: u64,
    pub semantic: usize,
    pub structural_change: usize,
    pub permanent_words: usize,
    pub provisional_words: usize,
    pub candidate_count: usize,
    pub optimization_used: bool,
    pub optimization_id: Option<String>,
    pub success: bool,
    pub cold: bool,
    pub occurrence_index: usize,
}

pub struct Engine {
    pub kind: EngineKind,
    executor: LiveExecutor,
    baseline: Option<Baseline>,
    drm_planner: Option<DrmPlanner>,
    registry: Option<Registry>,
    exact_cache_global: HashMap<String, Seq>,
    exact_cache_per_app: HashMap<(String, String), Seq>,
    static_macros: Vec<Seq>,
    /// workload_id -> how many times seen so far by this engine --
    /// drives both `cold`/`warm` and the development-curve occurrence
    /// index (spec S11's `C_W(1..n)`).
    occurrences: HashMap<String, usize>,
    /// Consolidation (DRM_B/DRM_C only) is deferred off the foreground
    /// path in the real daemon (a background timer); a benchmark harness
    /// has no such timer, so it is driven here on a fixed episode
    /// cadence instead -- still charged to `wall_time_ns` on the episode
    /// that triggers it, exactly like a real synchronous consolidation
    /// tick would be, never hidden from the measurement.
    episodes_since_consolidate: usize,
}

const CONSOLIDATE_EVERY: usize = 20;

impl Engine {
    pub fn start(kind: EngineKind, work_root: &Path) -> std::io::Result<Self> {
        let work = work_root.join(kind.label());
        let _ = std::fs::remove_dir_all(&work);
        make_fixtures(&work, 16)?;
        let mut executor = LiveExecutor::start(work).map_err(|e| std::io::Error::other(e.to_string()))?;
        if kind == EngineKind::DrmCSpecialized {
            executor = executor.with_specialization(SpecializationSet::new());
        }

        let baseline = match kind {
            EngineKind::Baseline0Stateless => Some(Baseline::new(BaselineKind::Stateless)),
            EngineKind::Baseline1ExactCache | EngineKind::Baseline3StaticMacros | EngineKind::Baseline4PerAppCache => {
                Some(Baseline::new(BaselineKind::TemplateCache))
            }
            EngineKind::Baseline2CheckpointReplay => Some(Baseline::new(BaselineKind::CheckpointReplay)),
            _ => None,
        };
        let drm_planner = matches!(kind, EngineKind::DrmAPermanentOnly).then(|| DrmPlanner::new(64, 3));
        let registry = matches!(kind, EngineKind::DrmBProvisionalPermanent | EngineKind::DrmCSpecialized).then(Registry::new);

        Ok(Self {
            kind,
            executor,
            baseline,
            drm_planner,
            registry,
            exact_cache_global: HashMap::new(),
            exact_cache_per_app: HashMap::new(),
            static_macros: static_macro_table(),
            occurrences: HashMap::new(),
            episodes_since_consolidate: 0,
        })
    }

    pub fn executor(&self) -> &LiveExecutor {
        &self.executor
    }

    pub fn registry(&self) -> Option<&Registry> {
        self.registry.as_ref()
    }

    pub fn drm_planner(&self) -> Option<&DrmPlanner> {
        self.drm_planner.as_ref()
    }

    /// Run one episode through this engine, returning what actually
    /// happened. `wall_time_ns` covers planning + the execute-or-skip
    /// decision + (on the DRM engines) any deferred consolidation
    /// triggered this episode -- the full real cost of handling this
    /// episode under this engine, not a cherry-picked sub-step.
    pub fn run_episode(&mut self, ep: &Episode) -> EpisodeOutcome {
        let occurrence_index = {
            let c = self.occurrences.entry(ep.ctx.workload_id.clone()).or_insert(0);
            *c += 1;
            *c
        };
        let cold = occurrence_index == 1;

        let t0 = Instant::now();
        let pm = self.plan_representation(ep);
        let success = self.execute_with_engine_policy(ep);
        if let Some(reg) = self.registry.as_mut() {
            self.episodes_since_consolidate += 1;
            if self.episodes_since_consolidate >= CONSOLIDATE_EVERY {
                reg.consolidate();
                self.episodes_since_consolidate = 0;
            }
        }
        let wall_time_ns = t0.elapsed().as_nanos() as u64;

        let (permanent_words, provisional_words) = self.vocab_counts(&ep.ctx.application_id);
        let candidate_count = self
            .executor
            .specializations
            .as_ref()
            .map(|s| s.ledger().all().count())
            .unwrap_or(0);
        let optimization_id = if self.executor.optimizations_used.is_empty() {
            None
        } else {
            Some(self.executor.optimizations_used.join("+"))
        };
        let optimization_used = optimization_id.is_some();

        EpisodeOutcome {
            wall_time_ns,
            semantic: pm.semantic,
            structural_change: pm.structural_change,
            permanent_words,
            provisional_words,
            candidate_count,
            optimization_used,
            optimization_id,
            success,
            cold,
            occurrence_index,
        }
    }

    fn plan_representation(&mut self, ep: &Episode) -> PlanMetrics {
        if let Some(b) = self.baseline.as_mut() {
            if self.kind == EngineKind::Baseline4PerAppCache {
                // BASELINE_4 differs from BASELINE_1 only in cache key
                // scope (per-application vs. global); reuse Baseline's
                // exact-match logic by namespacing the task id it keys
                // on, without touching the real episode identity used
                // for execution or reporting.
                let mut namespaced = ep.clone();
                namespaced.ctx.task_id = format!("{}::{}", ep.ctx.application_id, ep.task());
                return b.plan(&namespaced);
            }
            return b.plan(ep);
        }
        if let Some(p) = self.drm_planner.as_mut() {
            return p.plan(ep);
        }
        if let Some(reg) = self.registry.as_mut() {
            return reg.plan(&ep.ctx, ep.ops.clone(), ep.phase.clone(), ep.ancestral);
        }
        PlanMetrics::default()
    }

    /// Decide whether this episode's capabilities need to actually run,
    /// per this engine's own mechanism, and do so. Returns whether
    /// execution (real or skipped) succeeded.
    fn execute_with_engine_policy(&mut self, ep: &Episode) -> bool {
        match self.kind {
            EngineKind::Baseline0Stateless
            | EngineKind::Baseline2CheckpointReplay
            | EngineKind::DrmAPermanentOnly
            | EngineKind::DrmBProvisionalPermanent
            | EngineKind::DrmCSpecialized => self.executor.execute(ep).is_ok(),
            EngineKind::Baseline1ExactCache => {
                if self.exact_cache_global.get(ep.task()) == Some(&ep.ops) {
                    return true;
                }
                self.exact_cache_global.insert(ep.task().to_string(), ep.ops.clone());
                self.executor.execute(ep).is_ok()
            }
            EngineKind::Baseline4PerAppCache => {
                let key = (ep.ctx.application_id.clone(), ep.task().to_string());
                if self.exact_cache_per_app.get(&key) == Some(&ep.ops) {
                    return true;
                }
                self.exact_cache_per_app.insert(key, ep.ops.clone());
                self.executor.execute(ep).is_ok()
            }
            EngineKind::Baseline3StaticMacros => {
                if self.static_macros.iter().any(|m| m == &ep.ops) {
                    return true;
                }
                self.executor.execute(ep).is_ok()
            }
        }
    }

    fn vocab_counts(&self, application_id: &str) -> (usize, usize) {
        if let Some(p) = &self.drm_planner {
            return (p.vocab.derived.len(), 0);
        }
        if let Some(reg) = &self.registry {
            if let Some(app) = reg.applications.get(application_id) {
                return (app.planner.base.vocab.derived.len(), app.planner.provisional_words());
            }
        }
        (0, 0)
    }
}
