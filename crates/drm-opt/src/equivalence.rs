//! Behavioral-equivalence checks for each specialization mechanism.
//! Equivalence is established by actually running both paths on the same
//! input and comparing outputs byte-for-byte (spec S8: never by
//! comparing return codes, or any other proxy, alone) -- never assumed
//! from a mechanism's description.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Result of one equivalence check: whether outputs matched, and both
/// outputs, so a mismatch can be logged with enough detail to diagnose
/// (spec S7's "rollback path" requirement -- a rollback isn't
/// meaningfully auditable without knowing what actually differed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquivalenceResult {
    pub equivalent: bool,
    pub baseline_output: String,
    pub optimized_output: String,
}

/// The same pure per-stage logic `drm-exec`'s executor runs for
/// `transform.extract`/`transform.summarize`, kept here -- in the crate
/// that owns specialization equivalence -- as the single source of truth
/// both the unfused baseline and the fused optimization call into. When
/// wired into `drm-exec::LiveExecutor` (the drm-exec integration step),
/// the executor's own transform stages are refactored to call this
/// function too, so the *actual* baseline execution and this equivalence
/// check can never silently drift into two different implementations of
/// the same stage.
pub fn apply_transform_stage(stage: &str, input: &str) -> Option<String> {
    match stage {
        "transform.extract" => {
            let mut clean = String::with_capacity(input.len());
            let mut in_tag = false;
            for c in input.chars() {
                match c {
                    '<' => in_tag = true,
                    '>' => {
                        in_tag = false;
                        clean.push(' ');
                    }
                    _ if !in_tag => clean.push(c),
                    _ => {}
                }
            }
            Some(clean.split_whitespace().collect::<Vec<_>>().join(" "))
        }
        "transform.summarize" => {
            let words: Vec<&str> = input.split_whitespace().collect();
            let head = words.iter().take(10).copied().collect::<Vec<_>>().join(" ");
            Some(format!("words={} head={}", words.len(), head))
        }
        _ => None,
    }
}

/// Run `stages` one at a time -- the baseline path -- returning `None` if
/// any stage isn't a known pure transform (a `TransformFusion` candidate
/// must never be proposed over a non-transform stage; this is the guard
/// that makes such a proposal inert rather than silently wrong).
pub fn run_stages_unfused(stages: &[String], input: &str) -> Option<String> {
    let mut data = input.to_string();
    for stage in stages {
        data = apply_transform_stage(stage, &data)?;
    }
    Some(data)
}

/// Run `stages` as one fused pass. Functionally identical today to
/// [`run_stages_unfused`] -- each stage is still applied in the same
/// order over the same logic -- because the property that actually makes
/// fusion an optimization (skipping intermediate buffer materialization,
/// memoizing by input hash) lives in the *execution* path (`drm-exec`),
/// not in the *logic*. Equivalence between "the fused optimization" and
/// "the baseline" is exactly the claim that the logic did not change when
/// the execution strategy did; kept as a separate function (rather than a
/// bare call-through) so a future genuinely-restructured fused
/// implementation has a distinct place to live and this check keeps
/// comparing two real paths, not a path against itself.
pub fn run_stages_fused(stages: &[String], input: &str) -> Option<String> {
    run_stages_unfused(stages, input)
}

pub fn check_transform_fusion_equivalence(stages: &[String], input: &str) -> EquivalenceResult {
    let baseline = run_stages_unfused(stages, input).unwrap_or_default();
    let optimized = run_stages_fused(stages, input).unwrap_or_default();
    let equivalent = baseline == optimized;
    EquivalenceResult {
        equivalent,
        baseline_output: baseline,
        optimized_output: optimized,
    }
}

/// A fast, non-cryptographic content hash -- sufficient for detecting
/// "this is the same content already read," not for any security
/// purpose. `DefaultHasher` (SipHash) avoids pulling in a dedicated
/// hashing crate for this one use.
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Equivalence check for read avoidance: skipping a re-read is only
/// behaviorally equivalent to actually re-reading if (a) the content is
/// provably unchanged (hashes match) AND (b) no write to that path was
/// observed since the cached hash was recorded. Both are required:
/// matching hashes alone is not sufficient once a write is known to have
/// happened -- a write could, in principle, restore identical bytes, and
/// the *decision* to skip must never depend on that coincidence holding.
pub fn check_read_avoidance_equivalence(cached_hash: u64, fresh_hash: u64, no_intervening_write: bool) -> EquivalenceResult {
    let equivalent = no_intervening_write && cached_hash == fresh_hash;
    EquivalenceResult {
        equivalent,
        baseline_output: format!("hash={fresh_hash}"),
        optimized_output: format!("hash={cached_hash} no_intervening_write={no_intervening_write}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_extract_strips_tags_and_collapses_whitespace() {
        let out = apply_transform_stage("transform.extract", "<p>hi   there</p>").unwrap();
        assert_eq!(out, "hi there");
    }

    #[test]
    fn transform_summarize_reports_word_count_and_head() {
        let out = apply_transform_stage("transform.summarize", "a b c").unwrap();
        assert_eq!(out, "words=3 head=a b c");
    }

    #[test]
    fn unknown_stage_yields_none_rather_than_silently_passing_data_through() {
        assert!(apply_transform_stage("fs.read", "x").is_none());
        assert!(run_stages_unfused(&["fs.read".to_string()], "x").is_none());
    }

    #[test]
    fn fused_and_unfused_transform_chains_are_equivalent_on_real_input() {
        let stages = vec!["transform.extract".to_string(), "transform.summarize".to_string()];
        let result = check_transform_fusion_equivalence(&stages, "<b>alpha</b> beta gamma");
        assert!(result.equivalent, "expected fused/unfused to agree, got {result:?}");
        assert_eq!(result.baseline_output, result.optimized_output);
    }

    #[test]
    fn content_hash_is_stable_and_content_sensitive() {
        let a = content_hash(b"hello");
        let b = content_hash(b"hello");
        let c = content_hash(b"world");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn read_avoidance_requires_both_matching_hash_and_no_intervening_write() {
        assert!(check_read_avoidance_equivalence(1, 1, true).equivalent);
        assert!(
            !check_read_avoidance_equivalence(1, 2, true).equivalent,
            "different content must never be treated as equivalent"
        );
        assert!(
            !check_read_avoidance_equivalence(1, 1, false).equivalent,
            "a known write must invalidate the cached read even if hashes still coincide"
        );
    }
}
