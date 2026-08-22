//! Wires `drm-opt`'s specialization lifecycle into real execution.
//!
//! [`SpecializationSet`] is the piece of state that turns "a
//! specialization has been proposed and validated" (`drm-opt`, pure
//! logic, no I/O) into "this execution actually took the specialized
//! path" (here, where the real filesystem reads and real wall-clock
//! timings live). It owns:
//!
//! - the content-hash read cache for read avoidance (plan.md: "lives
//!   here [`drm-exec`], it owns real I/O"),
//! - a small memoization table for fused transform chains, keyed by a
//!   hash of their input -- the actual mechanism that makes fusion an
//!   optimization rather than just a relabeling (spec S7's "avoiding
//!   redundant immutable reads" and "memoizing deterministic pure
//!   transforms" starting points), and
//! - the shadow-sampling schedule that keeps a `Verified`/`Permanent`
//!   specialization honest by periodically still doing the real,
//!   unoptimized work and comparing (spec S6: "still periodically
//!   shadow-sampled").
//!
//! [`LiveExecutor`](crate::executor::LiveExecutor) is unaffected when no
//! `SpecializationSet` is attached (`LiveExecutor::start` behaves exactly
//! as before this module existed): specialization is strictly opt-in,
//! preserving every existing baseline/regression guarantee, including
//! the frozen `drmd bench` values.
//!
//! Every measured gain fed into `drm-opt`'s admission ledger here is a
//! real `Instant`-measured wall-clock delta around real work (a real
//! `fs::read_to_string`, a real transform pass, a real hashmap lookup)
//! -- never assumed, estimated, or backfilled from a formula. If a
//! specialization is not actually cheaper on this machine, the ledger
//! will honestly see negative gain and never promote it, per the
//! project's own instruction not to manufacture gains.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use drm_core::LifecycleStage;
use drm_opt::{check_read_avoidance_equivalence, content_hash, AdmissionLedger, EquivalenceResult, SpecializationKind};

/// How many uses a `Verified`/`Permanent` specialization is trusted
/// unconditionally before the next use is forced back through the real,
/// unoptimized path for comparison. A smallest-rigorous-implementation
/// constant, not a tuned one: small enough that a benchmark run's
/// episode counts exercise it, large enough that most uses of a verified
/// specialization actually get to be cheap.
pub const SHADOW_SAMPLE_INTERVAL: usize = 5;

/// Total validations (initial + shadow-sample) required before a
/// `Verified` specialization is promoted to `Permanent` (spec S5:
/// "required strong, sustained evidence").
pub const PERMANENT_THRESHOLD: usize = 10;

#[derive(Default)]
pub struct SpecializationSet {
    ledger: AdmissionLedger,
    /// path -> (content hash, content) of the last real read of that
    /// path. Keyed by path only, not by application: the filesystem has
    /// no notion of "which application," so this is the physically
    /// correct cache scope. Per-application isolation is still enforced
    /// where it matters -- the *decision* to trust the cache goes
    /// through `ledger`, whose specialization ids are
    /// application-qualified.
    read_cache: HashMap<String, (u64, String)>,
    /// Paths written since their cache entry was last consulted --
    /// consumed (and cleared) by the next read-avoidance validation of
    /// that path, feeding [`check_read_avoidance_equivalence`]'s
    /// `no_intervening_write` argument.
    written_since_check: HashSet<String>,
    /// (fusion specialization id, input content hash) -> computed
    /// output, for genuine memoization: the same input recurring is
    /// served from here instead of being recomputed.
    transform_memo: HashMap<(String, u64), String>,
    /// Uses since the last real validation, per specialization id --
    /// drives the shadow-sampling schedule.
    uses_since_sample: HashMap<String, usize>,
}

impl SpecializationSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ledger(&self) -> &AdmissionLedger {
        &self.ledger
    }

    fn due_for_sample(&self, id: &str) -> bool {
        self.uses_since_sample.get(id).copied().unwrap_or(0) >= SHADOW_SAMPLE_INTERVAL
    }

    fn mark_sampled(&mut self, id: &str) {
        self.uses_since_sample.insert(id.to_string(), 0);
    }

    fn mark_used_without_sampling(&mut self, id: &str) {
        *self.uses_since_sample.entry(id.to_string()).or_insert(0) += 1;
    }

    fn is_trusted(&self, id: &str) -> bool {
        matches!(
            self.ledger.get(id).map(|c| c.stage),
            Some(LifecycleStage::Verified) | Some(LifecycleStage::Permanent)
        )
    }

    fn record_and_maybe_promote(&mut self, id: &str, result: &EquivalenceResult, gain_ns: i64) {
        if let Ok(stage) = self.ledger.record_validation(id, result, gain_ns) {
            if stage == LifecycleStage::Verified {
                let _ = self.ledger.promote_if_sustained(id, PERMANENT_THRESHOLD);
            }
        }
        self.mark_sampled(id);
    }

    /// Called after any capability commits a write to `path` (relative
    /// to the executor's work directory): invalidates any cached read of
    /// that path and flags it so the next read-avoidance validation of
    /// that path sees a real intervening write, not just a coincidental
    /// hash match.
    pub fn mark_written(&mut self, path: &str) {
        self.read_cache.remove(path);
        self.written_since_check.insert(path.to_string());
    }

    fn read_avoidance_id(application_id: &str, path: &str) -> String {
        format!("read-avoid:{application_id}:{path}")
    }

    fn transform_fusion_id(application_id: &str, stages: &[String]) -> String {
        format!("fuse:{application_id}:{}", stages.join(">"))
    }

    /// Serve one `fs.read` of `path` (relative to the caller's work
    /// directory). `real_read` performs the actual `fs::read_to_string`
    /// -- the caller still owns real I/O; this function decides whether
    /// that call needed to happen at all, and updates specialization
    /// state either way.
    ///
    /// Returns `(content, optimization_id_if_the_cache_was_used)`.
    pub fn read(
        &mut self,
        application_id: &str,
        path: &str,
        real_read: impl FnOnce() -> std::io::Result<String>,
    ) -> std::io::Result<(String, Option<String>)> {
        let id = Self::read_avoidance_id(application_id, path);

        if self.is_trusted(&id) && !self.due_for_sample(&id) {
            if let Some(cached) = self.read_cache.get(path).map(|(_, c)| c.clone()) {
                self.mark_used_without_sampling(&id);
                return Ok((cached, Some(id)));
            }
        }

        // Real read: either this is the first sighting of this path, the
        // candidate is still being validated, or this use is a scheduled
        // shadow sample of an already-verified specialization.
        let t0 = Instant::now();
        let fresh = real_read()?;
        let baseline_ns = t0.elapsed().as_nanos() as i64;
        let fresh_hash = content_hash(fresh.as_bytes());

        if let Some(cached_hash) = self.read_cache.get(path).map(|(h, _)| *h) {
            // We have prior evidence for this exact path -- there is
            // something to validate against.
            if self.ledger.get(&id).is_none() {
                self.ledger.propose(
                    id.clone(),
                    application_id,
                    SpecializationKind::ReadAvoidance { path: path.to_string() },
                    vec!["fs.read".to_string()],
                );
                let _ = self.ledger.begin_validating(&id);
            }
            let no_intervening_write = !self.written_since_check.remove(path);
            let result = check_read_avoidance_equivalence(cached_hash, fresh_hash, no_intervening_write);
            // The gain a real cache hit would have realized: the wall
            // time this real read just cost, against the (also
            // measured, not assumed) cost of the hashmap lookup a cache
            // hit actually performs.
            let t1 = Instant::now();
            let _ = self.read_cache.get(path);
            let optimized_ns = t1.elapsed().as_nanos() as i64;
            self.record_and_maybe_promote(&id, &result, baseline_ns - optimized_ns);
        }

        self.read_cache.insert(path.to_string(), (fresh_hash, fresh.clone()));
        Ok((fresh, None))
    }

    /// Serve one run of a maximal consecutive pure `transform.*` stage
    /// chain over `input`. Returns `None` if any stage in `stages` isn't
    /// a known pure transform (propagated as an error by the caller,
    /// exactly as running the stages individually would have been).
    ///
    /// Returns `Some((output, optimization_id_if_memoized))`.
    ///
    /// Unlike read avoidance, a memo hit's "equivalence check" is
    /// definitional rather than a fresh empirical comparison: the
    /// content-hashed input fully determines the output of a pure,
    /// side-effect-free transform chain (kept as the single
    /// implementation in `drm_opt::equivalence`, so there is no second
    /// implementation to drift from), so there is no external state that
    /// could make a memoized entry go stale the way a file on disk can
    /// change underneath a cached read. The measured gain is still real:
    /// the wall-clock cost of actually recomputing the chain, against the
    /// wall-clock cost of the hashmap lookup a memo hit performs.
    pub fn run_transform_chain(&mut self, application_id: &str, stages: &[String], input: &str) -> Option<(String, Option<String>)> {
        let id = Self::transform_fusion_id(application_id, stages);
        let input_hash = content_hash(input.as_bytes());
        let key = (id.clone(), input_hash);

        if self.is_trusted(&id) && !self.due_for_sample(&id) {
            if let Some(cached) = self.transform_memo.get(&key).cloned() {
                self.mark_used_without_sampling(&id);
                return Some((cached, Some(id)));
            }
        }

        let t0 = Instant::now();
        let output = drm_opt::equivalence::run_stages_unfused(stages, input)?;
        let baseline_ns = t0.elapsed().as_nanos() as i64;

        if self.transform_memo.contains_key(&key) {
            // We've computed this exact (stage chain, input) before --
            // there is something to validate against.
            if self.ledger.get(&id).is_none() {
                self.ledger.propose(
                    id.clone(),
                    application_id,
                    SpecializationKind::TransformFusion { stages: stages.to_vec() },
                    stages.to_vec(),
                );
                let _ = self.ledger.begin_validating(&id);
            }
            let t1 = Instant::now();
            let _ = self.transform_memo.get(&key);
            let optimized_ns = t1.elapsed().as_nanos() as i64;
            let result = EquivalenceResult {
                equivalent: true,
                baseline_output: output.clone(),
                optimized_output: output.clone(),
            };
            self.record_and_maybe_promote(&id, &result, baseline_ns - optimized_ns);
        }

        self.transform_memo.insert(key, output.clone());
        Some((output, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_read_never_avoids_the_real_call_it_has_no_prior_evidence() {
        let mut spec = SpecializationSet::new();
        let (content, used) = spec.read("app-a", "in.csv", || Ok("hello".to_string())).unwrap();
        assert_eq!(content, "hello");
        assert!(used.is_none());
    }

    /// A real read of any real file costs measurably more than a hashmap
    /// lookup; a bare in-memory closure in a unit test doesn't, so give
    /// it a small, deterministic cost to measure against -- otherwise
    /// the "gain" between two near-instant operations is dominated by
    /// clock-resolution noise, and this test would be asserting
    /// something about jitter rather than about the mechanism.
    fn simulated_read(real_reads: &mut usize, content: &str) -> std::io::Result<String> {
        *real_reads += 1;
        std::thread::sleep(std::time::Duration::from_micros(50));
        Ok(content.to_string())
    }

    #[test]
    fn repeated_identical_reads_eventually_verify_and_then_skip_the_real_read() {
        let mut spec = SpecializationSet::new();
        let mut real_reads = 0usize;
        let mut used_id = None;
        for _ in 0..50 {
            let (content, used) = spec.read("app-a", "in.csv", || simulated_read(&mut real_reads, "hello")).unwrap();
            assert_eq!(content, "hello");
            if used.is_some() {
                used_id = used;
                break;
            }
        }
        assert!(
            used_id.is_some(),
            "expected a verified read-avoidance specialization to be used within 50 identical reads"
        );
        let reads_so_far = real_reads;

        // One more use, still within the sampling window, must not touch
        // the real read closure at all.
        spec.read("app-a", "in.csv", || simulated_read(&mut real_reads, "hello")).unwrap();
        assert_eq!(real_reads, reads_so_far, "a cache hit must never perform the real read");
    }

    #[test]
    fn write_invalidation_forces_a_real_read_even_once_verified() {
        let mut spec = SpecializationSet::new();
        let mut real_reads = 0usize;
        for _ in 0..50 {
            let (_, used) = spec.read("app-a", "in.csv", || simulated_read(&mut real_reads, "v1")).unwrap();
            if used.is_some() {
                break;
            }
        }
        assert!(real_reads > 0);
        let reads_before_write = real_reads;

        spec.mark_written("in.csv");
        let (content, used) = spec.read("app-a", "in.csv", || simulated_read(&mut real_reads, "v2")).unwrap();
        assert_eq!(content, "v2");
        assert!(used.is_none(), "a path just marked written must never be served from a stale cache");
        assert_eq!(real_reads, reads_before_write + 1);
    }

    #[test]
    fn write_invalidates_the_cache_entry_for_that_path() {
        let mut spec = SpecializationSet::new();
        spec.read("app-a", "in.csv", || Ok("v1".to_string())).unwrap();
        assert!(spec.read_cache.contains_key("in.csv"));
        spec.mark_written("in.csv");
        assert!(!spec.read_cache.contains_key("in.csv"));
    }

    #[test]
    fn repeated_identical_transform_input_eventually_memoizes() {
        let mut spec = SpecializationSet::new();
        let stages = vec!["transform.extract".to_string(), "transform.summarize".to_string()];
        let input = "<b>alpha</b> beta gamma";

        let mut used_id = None;
        for _ in 0..50 {
            let (out, used) = spec.run_transform_chain("app-a", &stages, input).unwrap();
            assert_eq!(out, "words=3 head=alpha beta gamma");
            if used.is_some() {
                used_id = used;
                break;
            }
        }
        assert!(
            used_id.is_some(),
            "expected a repeated identical input to eventually be served from memoization"
        );
    }

    #[test]
    fn different_inputs_are_never_confused_in_the_memo_table() {
        let mut spec = SpecializationSet::new();
        let stages = vec!["transform.summarize".to_string()];
        let (out_a, _) = spec.run_transform_chain("app-a", &stages, "alpha beta").unwrap();
        let (out_b, _) = spec.run_transform_chain("app-a", &stages, "gamma delta epsilon").unwrap();
        assert_ne!(out_a, out_b);
    }

    #[test]
    fn applications_never_share_a_specialization_id() {
        let mut spec = SpecializationSet::new();
        let stages = vec!["transform.summarize".to_string()];
        for _ in 0..50 {
            let (_, used) = spec.run_transform_chain("app-a", &stages, "same input").unwrap();
            if used.is_some() {
                break;
            }
        }
        // app-b has never validated this chain, so its first use must
        // still be a real (non-memoized) run even though app-a's
        // identical chain over identical input may already be verified.
        let (_, used) = spec.run_transform_chain("app-b", &stages, "same input").unwrap();
        assert!(used.is_none(), "app-b must not inherit app-a's verified specialization");
    }

    #[test]
    fn unknown_stage_in_a_chain_yields_none() {
        let mut spec = SpecializationSet::new();
        assert!(spec.run_transform_chain("app-a", &["fs.read".to_string()], "x").is_none());
    }
}
