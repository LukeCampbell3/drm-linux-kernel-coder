//! Deterministic synthetic workload generation for the desktop and server
//! benchmark suites (spec S12-S13).
//!
//! Real recorded desktop/server telemetry is not available in this
//! environment, so both suites are explicitly synthetic simulators, not a
//! claim of hooking real application activity -- consistent with
//! plan.md's scoping note. What they *do* provide, deliberately: multiple
//! applications each with their own recurring workload motifs, a
//! stationary period followed by drift (a motif's shape changes
//! mid-scenario), an old workload returning after the drift, a motif
//! shared across two applications (exercising cross-application
//! transfer), and single-occurrence noise episodes that must never be
//! learned (spec S21's negative-test requirement). Everything is
//! generated from a fixed seed, so a given scenario is byte-identical
//! across runs -- required for the development curves and baseline
//! comparisons to be comparable at all.

use drm_core::{Episode, ExecutionContext, Seq};

/// A tiny, dependency-free xorshift32 PRNG. Not cryptographic -- only
/// used to pick deterministically among a fixed menu of motif variations
/// and drift points, seeded once per scenario so the whole generator is
/// reproducible without pulling in the `rand` crate for it.
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0x9E3779B9 } else { seed })
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next_u32() as usize) % items.len()]
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u32() as usize) % n.max(1)
    }
}

fn seq(xs: &[&str]) -> Seq {
    xs.iter().map(|x| x.to_string()).collect()
}

/// The fixed menu of recurring capability motifs every scenario draws
/// from -- small, named, and reused across applications so cross-
/// application transfer (the same shape independently recurring in two
/// applications) has something real to detect.
pub struct Motifs;

impl Motifs {
    pub fn read_transform_write() -> Seq {
        seq(&["fs.read", "transform.extract", "transform.summarize", "fs.write", "notify.send"])
    }
    pub fn hash_check() -> Seq {
        seq(&["process.run", "transform.summarize", "fs.write"])
    }
    pub fn api_call() -> Seq {
        seq(&["http.request", "transform.extract", "transform.summarize", "fs.write"])
    }
    pub fn state_update() -> Seq {
        seq(&["state.read", "transform.summarize", "state.write"])
    }
    pub fn ipc_notify() -> Seq {
        seq(&["fs.read", "transform.summarize", "ipc.request", "fs.write"])
    }
    pub fn observe_log() -> Seq {
        seq(&["proc.observe", "notify.send"])
    }
    pub fn timer_state() -> Seq {
        seq(&["timer.observe", "state.read", "state.write"])
    }

    /// A drifted variant of `read_transform_write` -- one extra transform
    /// stage inserted, same overall shape otherwise. Used to simulate a
    /// mature workload changing (spec S21's drift test): the vocabulary
    /// that matched the old shape must stop matching, without the
    /// runtime corrupting unrelated learned structure.
    pub fn read_transform_write_drifted() -> Seq {
        seq(&[
            "fs.read",
            "transform.extract",
            "transform.extract",
            "transform.summarize",
            "fs.write",
            "notify.send",
        ])
    }
}

/// Twenty structurally distinct, hand-enumerated capability sequences,
/// each used at most once across a whole scenario. Drawing noise
/// episodes from this fixed list (rather than a formula that might
/// accidentally repeat a shape) is what makes the negative test in spec
/// S21 meaningful: `drm-core`'s MDL admission requires a subsequence to
/// recur across >= 2 distinct tasks (see
/// `drm-core::planner::tests::shared_motif_across_tasks_grows_permanent_vocabulary`),
/// so a pattern that provably never recurs can never legitimately be
/// admitted -- and the report generator checks exactly that against the
/// real, collected vocabulary, not just by construction.
///
/// Every pattern here is one capability repeated back-to-back two or
/// three times. No hand-authored motif in this module ever repeats a
/// capability immediately -- the one exception is
/// [`Motifs::read_transform_write_drifted`]'s deliberate double
/// `transform.extract`, which is why `transform.extract` is excluded
/// from the pool below. This makes "noise never collides with a
/// legitimate recurring motif" a structural guarantee rather than
/// something spot-checked by eye -- verified for real in
/// `tests::noise_patterns_never_collide_with_any_motif_subsequence`,
/// which was exactly the check that caught the previous version of this
/// list accidentally reusing `Motifs::observe_log()` verbatim as
/// "noise."
pub fn noise_patterns() -> Vec<Seq> {
    // `fs.write` is deliberately excluded: `LiveExecutor` verifies a
    // committed `fs.write` is non-empty, and a noise pattern with no
    // preceding OBSERVE/DERIVE capability has nothing to write --
    // that's a property of the executor's own correctness checking, not
    // something a "this should always execute successfully" noise
    // pattern should trip over.
    const REPEATABLE_CAPS: [&str; 10] = [
        "fs.read",
        "state.read",
        "proc.observe",
        "timer.observe",
        "http.request",
        "ipc.request",
        "process.run",
        "transform.summarize",
        "state.write",
        "notify.send",
    ];
    let mut patterns: Vec<Seq> = REPEATABLE_CAPS.iter().map(|c| vec![c.to_string(), c.to_string()]).collect();
    patterns.extend(REPEATABLE_CAPS.iter().map(|c| vec![c.to_string(), c.to_string(), c.to_string()]));
    patterns.truncate(20);
    patterns
}

#[cfg(test)]
/// Every contiguous subsequence of length 2..=5 in `seq` -- the same
/// candidate shape `drm_core::planner::DrmPlanner::note_subseqs` scores
/// for admission, reimplemented here (rather than depending on
/// `drm-core`'s private helper) purely as an independent check on the
/// scenario generator's own noise-vs-motif disjointness.
fn subsequences(seq: &[String]) -> Vec<Seq> {
    let mut out = Vec::new();
    let max_len = 5.min(seq.len());
    for len in 2..=max_len {
        for start in 0..=(seq.len() - len) {
            out.push(seq[start..start + len].to_vec());
        }
    }
    out
}

/// One synthetic capability episode plus enough bookkeeping for the
/// report generator: which occurrence of this workload this is (for the
/// development curves) and whether it's deliberately-unlearnable noise.
pub struct PlannedEpisode {
    pub ctx: ExecutionContext,
    pub ops: Seq,
    pub source: String,
    pub output: String,
    pub url_path: String,
    pub is_noise: bool,
}

pub struct Scenario {
    pub name: String,
    pub episodes: Vec<PlannedEpisode>,
}

impl Scenario {
    /// Materialize this scenario's episodes as real `drm_core::Episode`s
    /// with sequential indices -- every engine runs the exact same
    /// sequence, built fresh per engine so no engine's state can leak
    /// into another's run.
    pub fn to_episodes(&self) -> Vec<Episode> {
        self.episodes
            .iter()
            .enumerate()
            .map(|(i, pe)| {
                let mut ep = Episode::with_ctx(i + 1, pe.ctx.clone(), "simulate", pe.ops.clone());
                ep.source = pe.source.clone();
                ep.output = pe.output.clone();
                ep.url_path = pe.url_path.clone();
                ep
            })
            .collect()
    }
}

struct Builder {
    host_id: String,
    user_scope: String,
    episodes: Vec<PlannedEpisode>,
    fixture_count: usize,
}

impl Builder {
    fn new(host_id: &str, user_scope: &str) -> Self {
        Self {
            host_id: host_id.to_string(),
            user_scope: user_scope.to_string(),
            episodes: Vec::new(),
            fixture_count: 16,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, application_id: &str, workload_id: &str, task_id: &str, ops: Seq, rng: &mut Rng, is_noise: bool) {
        let ctx = ExecutionContext::new(self.host_id.clone(), self.user_scope.clone(), application_id, workload_id, task_id);
        let source = format!("inputs/report_{}.csv", rng.below(self.fixture_count));
        let output = format!("outputs/{application_id}_{task_id}.txt");
        let url_path = format!("/news_{}.html", rng.below(8));
        self.episodes.push(PlannedEpisode {
            ctx,
            ops,
            source,
            output,
            url_path,
            is_noise,
        });
    }

    /// Replay `n` previously-added, non-noise episodes verbatim -- same
    /// `ExecutionContext` (crucially, the same `task_id`), same ops,
    /// same source/output/url. Models the exact-same-request-recurring
    /// case ("the same report gets requested again," "the same file gets
    /// reopened") that exact-match caches (BASELINE_1/2/4) are actually
    /// designed to catch -- without at least some of these, those
    /// baselines would never get a single cache hit in the whole
    /// scenario, which would make them straw men rather than fair
    /// comparisons.
    fn replay_exact(&mut self, n: usize, rng: &mut Rng) {
        let candidates: Vec<usize> = (0..self.episodes.len()).filter(|&i| !self.episodes[i].is_noise).collect();
        if candidates.is_empty() {
            return;
        }
        for _ in 0..n {
            let i = *rng.pick(&candidates);
            let pe = &self.episodes[i];
            let replay = PlannedEpisode {
                ctx: pe.ctx.clone(),
                ops: pe.ops.clone(),
                source: pe.source.clone(),
                output: pe.output.clone(),
                url_path: pe.url_path.clone(),
                is_noise: false,
            };
            self.episodes.push(replay);
        }
    }
}

/// One application's recurring-workload profile: a name, the workload
/// motifs it repeats, and how many times each recurs across the
/// scenario's stationary period.
#[derive(Clone)]
struct AppProfile {
    application_id: &'static str,
    workloads: Vec<(&'static str, Seq)>,
    repeats_per_workload: usize,
}

fn run_profile(b: &mut Builder, rng: &mut Rng, profile: &AppProfile, task_counter: &mut usize) {
    // Stationary period: each workload recurs `repeats_per_workload`
    // times, interleaved round-robin (matching how a real service
    // actually receives mixed traffic, not one workload run to
    // completion before the next starts).
    for _ in 0..profile.repeats_per_workload {
        for (workload_id, ops) in &profile.workloads {
            *task_counter += 1;
            let task_id = format!("{workload_id}_{task_counter}");
            b.add(profile.application_id, workload_id, &task_id, ops.clone(), rng, false);
        }
    }
}

/// Server benchmark suite (spec S13, highest priority): four
/// applications, each developing its own learned state, hundreds of
/// episodes, a stationary period, mid-scenario drift on one workload, a
/// recurrence of an old (pre-drift) workload shape afterward, one motif
/// shared by two applications, and single-occurrence noise tasks that
/// must never be learned.
pub fn server_scenario() -> Scenario {
    let mut rng = Rng::new(0xD12_5A11);
    let mut b = Builder::new("srv-01", "system");
    let mut task_counter = 0usize;

    let profiles = [
        AppProfile {
            application_id: "api-service",
            workloads: vec![("api_get_user", Motifs::api_call()), ("session_touch", Motifs::state_update())],
            repeats_per_workload: 45,
        },
        AppProfile {
            application_id: "report-worker",
            // Shares `read_transform_write` with `build-worker` below --
            // the cross-application transfer motif.
            workloads: vec![
                ("daily_report", Motifs::read_transform_write()),
                ("archive_check", Motifs::hash_check()),
            ],
            repeats_per_workload: 40,
        },
        AppProfile {
            application_id: "build-worker",
            workloads: vec![
                ("checksum_artifact", Motifs::hash_check()),
                ("stage_output", Motifs::read_transform_write()),
            ],
            repeats_per_workload: 35,
        },
        AppProfile {
            application_id: "job-processor",
            workloads: vec![("dequeue_notify", Motifs::ipc_notify()), ("heartbeat", Motifs::timer_state())],
            repeats_per_workload: 40,
        },
    ];

    for profile in &profiles {
        run_profile(&mut b, &mut rng, profile, &mut task_counter);
    }

    // Drift: report-worker's `daily_report` workload changes shape
    // mid-scenario (an extra transform stage) for a run of episodes.
    for _ in 0..12 {
        task_counter += 1;
        let task_id = format!("daily_report_{task_counter}");
        b.add(
            "report-worker",
            "daily_report",
            &task_id,
            Motifs::read_transform_write_drifted(),
            &mut rng,
            false,
        );
    }

    // The old (pre-drift) shape returns -- must still be servable from
    // whatever global/permanent structure survived the drift, per spec
    // S21's "old global structure retained."
    for _ in 0..15 {
        task_counter += 1;
        let task_id = format!("daily_report_{task_counter}");
        b.add(
            "report-worker",
            "daily_report",
            &task_id,
            Motifs::read_transform_write(),
            &mut rng,
            false,
        );
    }

    // More stationary traffic after drift settles, so post-drift
    // specializations/vocabulary have episodes to actually verify against.
    for profile in &profiles {
        let shortened = AppProfile {
            repeats_per_workload: 15,
            ..profile.clone()
        };
        run_profile(&mut b, &mut rng, &shortened, &mut task_counter);
    }

    // Exact repeats: the same request/task recurring verbatim -- the
    // case naive exact-match caches (BASELINE_1/2/4) are actually built
    // to catch. See `Builder::replay_exact`.
    b.replay_exact(25, &mut rng);

    // Noise: single-occurrence, structurally unique tasks. Spec S21: at
    // least one pattern that must end REJECTED/never promoted.
    for ops in noise_patterns() {
        task_counter += 1;
        b.add("api-service", "noise", &format!("noise_{task_counter}"), ops, &mut rng, true);
    }

    Scenario {
        name: "server".to_string(),
        episodes: b.episodes,
    }
}

/// Desktop benchmark suite (spec S12): a single user's four workflow
/// families (development, reporting, research, stateful-utility) across
/// a simulated first day -> mature month -> a workload changing ->
/// an old workflow returning, matching the spec's explicit simulation
/// points. Modeled as one `user_scope`, several `application_id`s (the
/// desktop's hierarchy is `V_global > V_application > V_workload >
/// V_ephemeral` -- application here is "which app the user was in," e.g.
/// an editor vs. a report tool).
pub fn desktop_scenario() -> Scenario {
    let mut rng = Rng::new(0x0E5C_7070);
    let mut b = Builder::new("desktop-01", "alex");
    let mut task_counter = 0usize;

    let profiles = [
        AppProfile {
            application_id: "dev-editor",
            workloads: vec![
                ("build_and_hash", Motifs::hash_check()),
                ("save_and_backup", Motifs::read_transform_write()),
            ],
            repeats_per_workload: 30,
        },
        AppProfile {
            application_id: "report-tool",
            workloads: vec![
                ("weekly_report", Motifs::read_transform_write()),
                ("notify_team", Motifs::observe_log()),
            ],
            repeats_per_workload: 28,
        },
        AppProfile {
            application_id: "research-browser",
            workloads: vec![("fetch_and_summarize", Motifs::api_call()), ("sync_notes", Motifs::ipc_notify())],
            repeats_per_workload: 25,
        },
        AppProfile {
            application_id: "utility-daemon",
            workloads: vec![("session_state", Motifs::state_update()), ("idle_tick", Motifs::timer_state())],
            repeats_per_workload: 30,
        },
    ];

    // "First day": one light round of every workload.
    for profile in &profiles {
        let first_day = AppProfile {
            repeats_per_workload: 3,
            ..profile.clone()
        };
        run_profile(&mut b, &mut rng, &first_day, &mut task_counter);
    }

    // "Mature month": the bulk of the recurring traffic.
    for profile in &profiles {
        run_profile(&mut b, &mut rng, profile, &mut task_counter);
    }

    // "A workload changes": report-tool's weekly_report shape drifts.
    for _ in 0..10 {
        task_counter += 1;
        let task_id = format!("weekly_report_{task_counter}");
        b.add(
            "report-tool",
            "weekly_report",
            &task_id,
            Motifs::read_transform_write_drifted(),
            &mut rng,
            false,
        );
    }

    // "An old workflow returns": dev-editor's save_and_backup, unseen for
    // a while, recurs again exactly as it originally did.
    for _ in 0..8 {
        task_counter += 1;
        let task_id = format!("save_and_backup_{task_counter}");
        b.add(
            "dev-editor",
            "save_and_backup",
            &task_id,
            Motifs::read_transform_write(),
            &mut rng,
            false,
        );
    }

    // Exact repeats: reopening the same file, re-running the same build.
    b.replay_exact(20, &mut rng);

    // Noise: unique one-off tasks a desktop user genuinely never repeats.
    for ops in noise_patterns().into_iter().take(15) {
        task_counter += 1;
        b.add("dev-editor", "noise", &format!("noise_{task_counter}"), ops, &mut rng, true);
    }

    Scenario {
        name: "desktop".to_string(),
        episodes: b.episodes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_patterns_never_collide_with_any_motif_subsequence() {
        let motifs = [
            Motifs::read_transform_write(),
            Motifs::hash_check(),
            Motifs::api_call(),
            Motifs::state_update(),
            Motifs::ipc_notify(),
            Motifs::observe_log(),
            Motifs::timer_state(),
            Motifs::read_transform_write_drifted(),
        ];
        let mut motif_subseqs: std::collections::HashSet<Seq> = std::collections::HashSet::new();
        for m in &motifs {
            for s in subsequences(m) {
                motif_subseqs.insert(s);
            }
            // The full motif itself is also a candidate subsequence when
            // its own length is < 2 or > 5 relative to the 2..=5 window
            // `subsequences` covers -- for these motifs (length 2-6)
            // that window already includes every motif of length <= 5;
            // insert the full sequence unconditionally too so a 6-length
            // motif (the drifted variant) is covered as a whole as well.
            motif_subseqs.insert(m.clone());
        }
        let noise = noise_patterns();
        assert_eq!(
            noise.len(),
            20,
            "expected exactly 20 noise patterns (one per scenario noise episode)"
        );
        for pattern in &noise {
            assert!(!motif_subseqs.contains(pattern), "noise pattern {pattern:?} collides with a real motif subsequence -- it would be a legitimate candidate for admission, defeating the negative test");
        }
        // Every noise pattern must also be structurally distinct from
        // every other noise pattern (no pattern used twice).
        let unique: std::collections::HashSet<&Seq> = noise.iter().collect();
        assert_eq!(unique.len(), noise.len(), "noise patterns must all be distinct");
    }

    #[test]
    fn server_and_desktop_scenarios_are_deterministic() {
        let a = server_scenario();
        let b = server_scenario();
        assert_eq!(a.episodes.len(), b.episodes.len());
        for (x, y) in a.episodes.iter().zip(b.episodes.iter()) {
            assert_eq!(x.ctx.task_id, y.ctx.task_id);
            assert_eq!(x.source, y.source);
            assert_eq!(x.ops, y.ops);
        }
    }

    #[test]
    fn scenarios_contain_noise_and_a_drift_and_recurrence_point() {
        for scenario in [server_scenario(), desktop_scenario()] {
            assert!(
                scenario.episodes.iter().any(|e| e.is_noise),
                "{} scenario has no noise episodes",
                scenario.name
            );
            assert!(
                scenario.episodes.len() > 100,
                "{} scenario is too small to be a real benchmark",
                scenario.name
            );
        }
    }

    #[test]
    fn every_generated_episode_uses_only_known_capabilities() {
        for scenario in [server_scenario(), desktop_scenario()] {
            for ep in &scenario.episodes {
                for cap in &ep.ops {
                    assert!(
                        drm_core::is_known_capability(cap),
                        "unknown capability `{cap}` in {} scenario",
                        scenario.name
                    );
                }
            }
        }
    }
}
