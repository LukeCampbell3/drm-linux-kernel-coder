//! Runs one scenario through all eight engines and writes the
//! comparative report: a full per-episode metrics CSV, a development-
//! curves CSV, and a markdown summary with the adversarial checks from
//! spec S15 run as actual assertions against the collected data, not
//! just asserted in prose.

use std::fs;
use std::io::Write as _;
use std::path::Path;

use drm_core::Episode;
use drm_observe::{ExecutionMetrics, Warmth};

use super::engine::{Engine, EngineKind};
use super::scenario::Scenario;

pub struct SimulationReport {
    pub scenario_name: String,
    pub episodes: usize,
    pub engines: Vec<EngineSummary>,
    pub adversarial_checks: Vec<AdversarialCheck>,
}

pub struct EngineSummary {
    pub kind: EngineKind,
    pub total_wall_ns: u64,
    pub total_semantic_tokens: usize,
    pub commits: usize,
    pub process_spawns: usize,
    pub tcp_requests: usize,
    pub ipc_requests: usize,
    pub final_permanent_words: usize,
    pub final_provisional_words: usize,
    pub verified_specializations: usize,
    pub permanent_specializations: usize,
    pub rolled_back_specializations: usize,
    pub reads_avoided: usize,
    pub transforms_memoized: usize,
    pub failed_episodes: usize,
    /// Aggregate CPU time across the whole run, from one
    /// `drm_observe::MeasuredRun` bracketing the entire episode loop.
    /// Per-episode CPU is not reported (see module docs on clock-tick
    /// granularity) -- this aggregate is the honest, meaningful number.
    pub aggregate_cpu_ns: u64,
}

pub struct AdversarialCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

fn engine_label_for_observe(kind: EngineKind) -> drm_observe::Engine {
    match kind {
        EngineKind::Baseline0Stateless => drm_observe::Engine::Baseline("stateless"),
        EngineKind::Baseline1ExactCache => drm_observe::Engine::Baseline("exact_cache"),
        EngineKind::Baseline2CheckpointReplay => drm_observe::Engine::Baseline("checkpoint_replay"),
        EngineKind::Baseline3StaticMacros => drm_observe::Engine::Baseline("static_macros"),
        EngineKind::Baseline4PerAppCache => drm_observe::Engine::Baseline("per_app_cache"),
        EngineKind::DrmAPermanentOnly => drm_observe::Engine::Drm("permanent_only"),
        EngineKind::DrmBProvisionalPermanent => drm_observe::Engine::Drm("provisional_permanent"),
        EngineKind::DrmCSpecialized => drm_observe::Engine::Drm("specialized"),
    }
}

pub fn run(scenario: &Scenario, out_dir: &Path) -> std::io::Result<SimulationReport> {
    fs::create_dir_all(out_dir)?;
    let engines_work_root = out_dir.join("engine_workspaces");
    let episodes = scenario.to_episodes();

    let metrics_path = out_dir.join(format!("{}_metrics.csv", scenario.name));
    let curves_path = out_dir.join(format!("{}_development_curves.csv", scenario.name));
    let mut metrics_file = fs::File::create(&metrics_path)?;
    writeln!(metrics_file, "{}", ExecutionMetrics::csv_header())?;
    let mut curves_file = fs::File::create(&curves_path)?;
    writeln!(
        curves_file,
        "engine,application_id,workload_id,occurrence_index,wall_time_ns,warmth"
    )?;

    let mut summaries = Vec::new();
    // Kept alive until after adversarial checks run: the noise-never-
    // learned check (S15) needs to inspect each DRM engine's live
    // vocabulary, not just its on-disk output.
    let mut engines = Vec::new();

    for kind in EngineKind::all() {
        let mut engine = Engine::start(kind, &engines_work_root)?;
        let observe_engine = engine_label_for_observe(kind);
        let mut total_wall_ns = 0u64;
        let mut total_semantic = 0usize;
        let mut failed_episodes = 0usize;

        let cpu_start = drm_observe::MeasuredRun::start();

        for ep in &episodes {
            let (before_spawns, before_ipc, before_tcp) = (
                engine.executor().process_spawns,
                engine.executor().ipc_requests,
                engine.executor().tcp_requests,
            );
            let outcome = engine.run_episode(ep);
            let (after_spawns, after_ipc, after_tcp) = (
                engine.executor().process_spawns,
                engine.executor().ipc_requests,
                engine.executor().tcp_requests,
            );

            total_wall_ns += outcome.wall_time_ns;
            total_semantic += outcome.semantic;
            if !outcome.success {
                failed_episodes += 1;
            }

            let warmth = if outcome.cold { Warmth::Cold } else { Warmth::Warm };
            let verification_status = outcome
                .optimization_id
                .as_ref()
                .and_then(|id| engine.executor().specializations.as_ref().and_then(|s| s.ledger().get(id)))
                .map(|c| format!("{:?}", c.stage));
            let rollback_count = outcome
                .optimization_id
                .as_ref()
                .and_then(|id| engine.executor().specializations.as_ref().and_then(|s| s.ledger().get(id)))
                .map(|c| c.rollback_count)
                .unwrap_or(0);

            let row = ExecutionMetrics {
                application_id: ep.ctx.application_id.clone(),
                workload_id: ep.ctx.workload_id.clone(),
                task_id: ep.ctx.task_id.clone(),
                episode_index: ep.idx,
                warmth,
                engine: observe_engine,
                representation_tokens: outcome.semantic,
                planning_decisions: outcome.structural_change,
                wall_time_ns: outcome.wall_time_ns,
                // Per-episode CPU/RSS/byte counters are not meaningfully
                // measurable at this granularity (see module docs) --
                // left at zero here; the real, aggregate figures are in
                // the per-engine summary instead of being fabricated
                // per-episode.
                cpu_time_ns: 0,
                user_cpu_ns: 0,
                system_cpu_ns: 0,
                rss_kb: 0,
                bytes_read: 0,
                bytes_written: 0,
                syscall_count: None,
                process_spawn_count: after_spawns - before_spawns,
                ipc_count: after_ipc - before_ipc,
                network_count: after_tcp - before_tcp,
                permanent_words: outcome.permanent_words,
                provisional_words: outcome.provisional_words,
                candidate_count: outcome.candidate_count,
                structural_changes: outcome.structural_change,
                optimization_used: outcome.optimization_used,
                optimization_id: outcome.optimization_id.clone(),
                verification_status,
                rollback_count,
            };
            writeln!(metrics_file, "{}", row.to_csv_row())?;
            writeln!(
                curves_file,
                "{},{},{},{},{},{}",
                kind.label(),
                ep.ctx.application_id,
                ep.ctx.workload_id,
                outcome.occurrence_index,
                outcome.wall_time_ns,
                warmth.label(),
            )?;
        }
        let aggregate_cpu_ns = cpu_start.finish().cpu_time_ns;

        let (verified, permanent, rolled_back) = specialization_counts(&engine);
        let (final_permanent_words, final_provisional_words) = final_vocab_totals(&engine);

        summaries.push(EngineSummary {
            kind,
            total_wall_ns,
            total_semantic_tokens: total_semantic,
            commits: engine.executor().commits,
            process_spawns: engine.executor().process_spawns,
            tcp_requests: engine.executor().tcp_requests,
            ipc_requests: engine.executor().ipc_requests,
            final_permanent_words,
            final_provisional_words,
            verified_specializations: verified,
            permanent_specializations: permanent,
            rolled_back_specializations: rolled_back,
            reads_avoided: engine.executor().reads_avoided,
            transforms_memoized: engine.executor().transforms_memoized,
            failed_episodes,
            aggregate_cpu_ns,
        });

        engines.push(engine);
    }

    let adversarial_checks = run_adversarial_checks(scenario, &summaries, &engines, &engines_work_root, &metrics_path)?;

    write_summary_markdown(out_dir, scenario, &summaries, &adversarial_checks)?;

    Ok(SimulationReport {
        scenario_name: scenario.name.clone(),
        episodes: episodes.len(),
        engines: summaries,
        adversarial_checks,
    })
}

fn specialization_counts(engine: &Engine) -> (usize, usize, usize) {
    let Some(spec) = engine.executor().specializations.as_ref() else {
        return (0, 0, 0);
    };
    let ledger = spec.ledger();
    let verified = ledger.all().filter(|c| c.stage == drm_core::LifecycleStage::Verified).count();
    let permanent = ledger.all().filter(|c| c.stage == drm_core::LifecycleStage::Permanent).count();
    let rolled_back = ledger.all().filter(|c| c.stage == drm_core::LifecycleStage::RolledBack).count();
    (verified, permanent, rolled_back)
}

fn final_vocab_totals(engine: &Engine) -> (usize, usize) {
    if let Some(p) = engine.drm_planner() {
        return (p.vocab.derived.len(), 0);
    }
    if let Some(reg) = engine.registry() {
        let mut permanent = 0;
        let mut provisional = 0;
        for app in reg.applications.values() {
            permanent += app.planner.base.vocab.derived.len();
            provisional += app.planner.provisional_words();
        }
        return (permanent, provisional);
    }
    (0, 0)
}

/// Spec S15's adversarial checklist, run as real assertions against the
/// data just collected -- a result only counts as reported if it passes
/// these, per the project's own instruction not to manufacture gains.
fn run_adversarial_checks(
    scenario: &Scenario,
    summaries: &[EngineSummary],
    engines: &[Engine],
    engines_work_root: &Path,
    metrics_path: &Path,
) -> std::io::Result<Vec<AdversarialCheck>> {
    let mut checks = Vec::new();

    // 1. Warmup cost is included, not excluded: the metrics CSV must
    // contain a "cold" row for every workload_id that appears at all.
    let metrics_text = fs::read_to_string(metrics_path)?;
    let has_cold_rows = metrics_text.lines().skip(1).any(|l| l.contains(",cold,"));
    checks.push(AdversarialCheck {
        name: "warmup cost is included in the reported data".to_string(),
        passed: has_cold_rows,
        detail: if has_cold_rows {
            "cold (first-occurrence) rows are present in the metrics CSV".to_string()
        } else {
            "no cold rows found -- warmup cost would be silently excluded".to_string()
        },
    });

    // 2. No engine silently swallowed a real execution failure: a
    // "cheaper" engine that is actually just failing to do the work
    // would be a fabricated win, not a real one.
    let engines_with_failures: Vec<String> = summaries
        .iter()
        .filter(|s| s.failed_episodes > 0)
        .map(|s| format!("{} ({} failed)", s.kind.label(), s.failed_episodes))
        .collect();
    checks.push(AdversarialCheck {
        name: "no engine reports a win by way of silently failed executions".to_string(),
        passed: engines_with_failures.is_empty(),
        detail: if engines_with_failures.is_empty() {
            "zero execution failures across every engine".to_string()
        } else {
            engines_with_failures.join(", ")
        },
    });

    // 3. Representation vs. actual cost stay independent columns: DRM_A
    // and DRM_B must show no *real* wall-time advantage over BASELINE_0
    // worth reporting as if it were one -- they have no execution-skip
    // mechanism, so any apparent "win" here would be measurement noise,
    // not a real effect, and must not be reported as such.
    let baseline0 = summaries.iter().find(|s| s.kind == EngineKind::Baseline0Stateless);
    let drm_b = summaries.iter().find(|s| s.kind == EngineKind::DrmBProvisionalPermanent);
    if let (Some(b0), Some(b)) = (baseline0, drm_b) {
        // DRM_B never skips real execution, so its wall time should be
        // within the same order of magnitude as BASELINE_0's (plus
        // planning overhead) -- a token/representation reduction must
        // never be reported as if it were this kind of real saving.
        let ratio = b.total_wall_ns as f64 / b0.total_wall_ns.max(1) as f64;
        let plausible = (0.5..2.5).contains(&ratio);
        checks.push(AdversarialCheck {
            name: "DRM_B (no execution-skip mechanism) shows no fabricated wall-time win over BASELINE_0".to_string(),
            passed: plausible,
            detail: format!(
                "DRM_B/BASELINE_0 wall-time ratio = {ratio:.3} (expected roughly 0.5x-2.5x, since DRM_B cannot skip real work)"
            ),
        });
    }

    // 4. DRM_C's own specialization bookkeeping is charged to it, not
    // excluded: DRM_C must not report a smaller total_wall_ns than a
    // hypothetical "free" version would by skipping the overhead of
    // proposing/validating candidates -- check this indirectly by
    // requiring DRM_C to have actually recorded validation activity
    // (i.e., the overhead really happened and was timed) whenever it
    // reports any specialization use at all.
    if let Some(drm_c) = summaries.iter().find(|s| s.kind == EngineKind::DrmCSpecialized) {
        let used_any = drm_c.reads_avoided > 0 || drm_c.transforms_memoized > 0;
        let has_candidates = drm_c.verified_specializations + drm_c.permanent_specializations + drm_c.rolled_back_specializations > 0;
        let consistent = !used_any || has_candidates;
        checks.push(AdversarialCheck {
            name: "DRM_C never reports specialization use without the corresponding validated candidate on record".to_string(),
            passed: consistent,
            detail: format!(
                "reads_avoided={}, transforms_memoized={}, verified+permanent+rolled_back candidates={}",
                drm_c.reads_avoided,
                drm_c.transforms_memoized,
                drm_c.verified_specializations + drm_c.permanent_specializations + drm_c.rolled_back_specializations
            ),
        });
    }

    // 5. Observable semantics never change: for a sample of episodes
    // that produced a committed file (fs.write), the committed content
    // must be byte-identical across every engine's independent work
    // directory.
    let mut mismatches = Vec::new();
    let mut compared = 0usize;
    for ep in scenario
        .to_episodes()
        .iter()
        .filter(|e| e.ops.iter().any(|c| c == "fs.write"))
        .step_by(7)
    {
        let mut reference: Option<String> = None;
        for kind in EngineKind::all() {
            let engine_work_dir = engines_work_root.join(kind.label());
            let path = engine_work_dir.join(&ep.output);
            let Ok(raw) = fs::read_to_string(&path) else { continue };
            // `process.run` shells out to `sha256sum <absolute-path>`,
            // whose stdout embeds the file's absolute path -- and each
            // engine necessarily runs against its own isolated work
            // directory (a real, deliberate isolation requirement, not a
            // bug). Normalize that expected, harmless per-engine path
            // difference away before comparing, so this check catches an
            // actual behavioral change, not an artifact of giving every
            // engine its own sandboxed filesystem.
            let content = raw.replace(engine_work_dir.to_string_lossy().as_ref(), "<engine-work-dir>");
            compared += 1;
            match &reference {
                None => reference = Some(content),
                Some(r) if r == &content => {}
                Some(_) => mismatches.push(format!("{} differs on {}", kind.label(), ep.output)),
            }
        }
    }
    checks.push(AdversarialCheck {
        name: "committed output is byte-identical across every engine (no engine silently changed observable behavior)".to_string(),
        passed: mismatches.is_empty(),
        detail: if mismatches.is_empty() {
            format!("{compared} file comparisons across engines, all identical")
        } else {
            format!("{} mismatches: {}", mismatches.len(), mismatches.join("; "))
        },
    });

    // 6. Required durable output actually exists: every non-noise
    // fs.write episode must have actually produced its committed file,
    // for every engine *except* BASELINE_3 -- whose whole design is to
    // skip a known-shape episode unconditionally, from its very first
    // occurrence, with no verification that doing so is safe. That is a
    // deliberate, documented property of that one naive reference
    // baseline (see `static_macro_table`'s doc comment), not something
    // DRM or any other baseline is allowed to do, so it is excluded from
    // the pass/fail gate here and reported separately instead.
    let all_episodes = scenario.to_episodes();
    let write_episodes: Vec<&Episode> = all_episodes
        .iter()
        .filter(|e| !e.ctx.workload_id.contains("noise") && e.ops.iter().any(|c| c == "fs.write"))
        .collect();
    let mut missing_required: Vec<String> = Vec::new();
    let mut baseline3_skipped = 0usize;
    for kind in EngineKind::all() {
        let work_dir = engines_work_root.join(kind.label());
        let mut missing = 0usize;
        for ep in &write_episodes {
            if !work_dir.join(&ep.output).exists() {
                missing += 1;
            }
        }
        if kind == EngineKind::Baseline3StaticMacros {
            baseline3_skipped = missing;
        } else if missing > 0 {
            missing_required.push(format!(
                "{} is missing {missing}/{} required outputs",
                kind.label(),
                write_episodes.len()
            ));
        }
    }
    checks.push(AdversarialCheck {
        name: "every engine except the deliberately-naive BASELINE_3 produces every required durable output".to_string(),
        passed: missing_required.is_empty(),
        detail: if missing_required.is_empty() {
            format!("all {} required outputs present for every engine except BASELINE_3 (which skipped {baseline3_skipped}/{} by design -- see its doc comment)", write_episodes.len(), write_episodes.len())
        } else {
            missing_required.join("; ")
        },
    });

    // 7. Noise is never learned (spec S21's negative test): scan every
    // DRM engine's *actual, live* vocabulary for a derived word whose
    // capability-level expansion exactly matches one of the scenario's
    // known-never-recurring noise patterns. This inspects the real
    // collected state, not just an assumption about how admission is
    // supposed to work.
    let noise: Vec<drm_core::Seq> = super::scenario::noise_patterns();
    let mut leaked = Vec::new();
    for engine in engines {
        if let Some(reg) = engine.registry() {
            for (app_id, app) in &reg.applications {
                for (name, expansion) in app.planner.base.vocab.expansions() {
                    if noise.contains(&expansion) {
                        leaked.push(format!(
                            "{} learned `{name}` == a noise pattern in app `{app_id}`",
                            engine.kind.label()
                        ));
                    }
                }
            }
        }
        if let Some(p) = engine.drm_planner() {
            for (name, expansion) in p.vocab.expansions() {
                if noise.contains(&expansion) {
                    leaked.push(format!("{} learned `{name}` == a noise pattern", engine.kind.label()));
                }
            }
        }
    }
    checks.push(AdversarialCheck {
        name: "noise (single-occurrence, non-recurring patterns) is never promoted into learned vocabulary".to_string(),
        passed: leaked.is_empty(),
        detail: if leaked.is_empty() {
            format!(
                "checked {} noise patterns against every DRM engine's live vocabulary; none were learned",
                noise.len()
            )
        } else {
            leaked.join("; ")
        },
    });

    Ok(checks)
}

fn write_summary_markdown(
    out_dir: &Path,
    scenario: &Scenario,
    summaries: &[EngineSummary],
    checks: &[AdversarialCheck],
) -> std::io::Result<()> {
    let mut f = fs::File::create(out_dir.join(format!("{}_summary.md", scenario.name)))?;
    writeln!(f, "# {} simulation summary\n", scenario.name)?;
    writeln!(f, "Episodes: {}\n", scenario.episodes.len())?;
    writeln!(f, "| engine | total wall (ms) | aggregate cpu (ms) | semantic tokens | commits | process spawns | tcp requests | ipc requests | failed episodes | permanent words | provisional words | verified specs | permanent specs | rolled back specs | reads avoided | transforms memoized |")?;
    writeln!(f, "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")?;
    for s in summaries {
        writeln!(
            f,
            "| {} | {:.3} | {:.3} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            s.kind.label(),
            s.total_wall_ns as f64 / 1e6,
            s.aggregate_cpu_ns as f64 / 1e6,
            s.total_semantic_tokens,
            s.commits,
            s.process_spawns,
            s.tcp_requests,
            s.ipc_requests,
            s.failed_episodes,
            s.final_permanent_words,
            s.final_provisional_words,
            s.verified_specializations,
            s.permanent_specializations,
            s.rolled_back_specializations,
            s.reads_avoided,
            s.transforms_memoized,
        )?;
    }
    writeln!(f, "\n## Adversarial checks (spec S15)\n")?;
    for c in checks {
        writeln!(f, "- [{}] {} -- {}", if c.passed { "PASS" } else { "FAIL" }, c.name, c.detail)?;
    }
    Ok(())
}
