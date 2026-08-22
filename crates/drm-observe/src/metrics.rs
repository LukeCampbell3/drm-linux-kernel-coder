//! Per-execution measurement: real process-level counters (wall time, CPU
//! time, RSS, I/O byte counts) sampled via `/proc/self/*`, joined with
//! executor/planner-reported counts (process spawns, IPC round-trips,
//! permanent/provisional vocabulary size, ...) into one
//! [`ExecutionMetrics`] record per episode.
//!
//! Deliberately not measured: per-syscall counts (`syscall_count`) and
//! energy consumption. Getting real syscall counts means `ptrace`/`strace`
//! or eBPF instrumentation around every execution -- exactly the kind of
//! heavyweight, platform-specific machinery the spec's own instruction to
//! "avoid brittle platform-specific metrics" argues against. A *fabricated*
//! syscall count would be worse than an honestly absent one: it would look
//! like real evidence in the final report while being none. `syscall_count`
//! is therefore always `None`, and every report that renders this record
//! must say "not measured" rather than silently treating the column as
//! zero -- enforced by making it `Option<u64>`, not `u64`.

use std::time::Instant;

/// Whether this execution is the first time this engine has ever executed
/// this exact workload, or a repeat -- the axis the development curves
/// (spec S11, `C_W(1..n)`) are plotted against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warmth {
    Cold,
    Warm,
}

impl Warmth {
    pub fn label(self) -> &'static str {
        match self {
            Warmth::Cold => "cold",
            Warmth::Warm => "warm",
        }
    }
}

/// Which execution engine produced this record -- lets one CSV hold both
/// baseline and DRM rows for direct, apples-to-apples comparison (spec
/// S14's `BASELINE_0..4` vs. `DRM_A/B/C`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Baseline(&'static str),
    Drm(&'static str),
}

impl Engine {
    pub fn label(self) -> String {
        match self {
            Engine::Baseline(name) => format!("baseline:{name}"),
            Engine::Drm(name) => format!("drm:{name}"),
        }
    }
}

/// A snapshot of this process's own resource counters, taken via
/// `/proc/self/stat` (utime/stime, in clock ticks) and `/proc/self/status`
/// (`VmRSS`). Two samples bracketing a unit of work, subtracted, give real
/// CPU time spent on that work without any external profiling tool.
#[derive(Clone, Copy, Debug, Default)]
struct ProcessSample {
    wall: Option<Instant>,
    utime_ticks: u64,
    stime_ticks: u64,
    rss_kb: u64,
}

impl ProcessSample {
    /// Take a sample of the current process's resource counters. Never
    /// fails: a `/proc` read that doesn't parse just leaves that field at
    /// zero -- this is measurement, not a load-bearing correctness path,
    /// and a missing sample must never abort the execution it is timing.
    fn now() -> Self {
        let (utime_ticks, stime_ticks) = read_proc_self_stat_times().unwrap_or((0, 0));
        let rss_kb = read_proc_self_status_rss_kb().unwrap_or(0);
        Self {
            wall: Some(Instant::now()),
            utime_ticks,
            stime_ticks,
            rss_kb,
        }
    }
}

/// `sysconf(_SC_CLK_TCK)`, resolved once per process via `getconf` rather
/// than an `libc` binding for this one constant. Falls back to 100, the
/// value on effectively every Linux configuration this runtime targets,
/// if `getconf` isn't on `PATH` (a minimal container, a sandboxed test
/// runner).
fn clock_ticks_per_sec() -> u64 {
    static TICKS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *TICKS.get_or_init(|| {
        std::process::Command::new("getconf")
            .arg("CLK_TCK")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(100)
    })
}

fn read_proc_self_stat_times() -> Option<(u64, u64)> {
    let s = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Field 2 (`comm`) is parenthesized and may itself contain spaces or
    // parentheses, so split on the *last* ')' to skip past it reliably;
    // fields after that point are then whitespace-separated and stably
    // indexed. utime is field 14 overall (rest[11] here, since rest[0] is
    // field 3); stime is field 15 (rest[12]). See `man 5 proc`.
    let after_comm = s.rsplit_once(')')?.1;
    let rest: Vec<&str> = after_comm.split_whitespace().collect();
    let utime: u64 = rest.get(11)?.parse().ok()?;
    let stime: u64 = rest.get(12)?.parse().ok()?;
    Some((utime, stime))
}

fn read_proc_self_status_rss_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// Brackets a unit of work with two [`ProcessSample`]s and reduces them to
/// the resource-cost fields of an [`ExecutionMetrics`] record. Everything
/// else on the record (identity, vocabulary counts, optimization
/// involvement) is filled in by the caller, which is the only place that
/// actually knows those things.
pub struct MeasuredRun {
    start: ProcessSample,
}

/// Real resource cost measured across one [`MeasuredRun`].
pub struct ResourceCost {
    pub wall_time_ns: u64,
    pub cpu_time_ns: u64,
    pub user_cpu_ns: u64,
    pub system_cpu_ns: u64,
    pub rss_kb: u64,
}

impl MeasuredRun {
    pub fn start() -> Self {
        Self {
            start: ProcessSample::now(),
        }
    }

    pub fn finish(self) -> ResourceCost {
        let end = ProcessSample::now();
        let wall_time_ns = end
            .wall
            .zip(self.start.wall)
            .map(|(e, s)| e.saturating_duration_since(s).as_nanos() as u64)
            .unwrap_or(0);
        let ticks = clock_ticks_per_sec().max(1);
        let user_ticks = end.utime_ticks.saturating_sub(self.start.utime_ticks);
        let sys_ticks = end.stime_ticks.saturating_sub(self.start.stime_ticks);
        let user_cpu_ns = user_ticks * 1_000_000_000 / ticks;
        let system_cpu_ns = sys_ticks * 1_000_000_000 / ticks;
        ResourceCost {
            wall_time_ns,
            cpu_time_ns: user_cpu_ns + system_cpu_ns,
            user_cpu_ns,
            system_cpu_ns,
            rss_kb: end.rss_kb,
        }
    }
}

/// The full per-episode measurement record (spec S10).
#[derive(Clone, Debug)]
pub struct ExecutionMetrics {
    pub application_id: String,
    pub workload_id: String,
    pub task_id: String,
    pub episode_index: usize,
    pub warmth: Warmth,
    pub engine: Engine,

    // Representation vs. actual work -- kept as separate columns
    // deliberately (spec S2/S11): a shrinking token count must never be
    // read as a falling CPU cost, and vice versa.
    pub representation_tokens: usize,
    pub planning_decisions: usize,

    // Real resource cost, sampled from this process's own counters.
    pub wall_time_ns: u64,
    pub cpu_time_ns: u64,
    pub user_cpu_ns: u64,
    pub system_cpu_ns: u64,
    pub rss_kb: u64,

    // Executor-tracked I/O and interaction counts.
    pub bytes_read: u64,
    pub bytes_written: u64,
    /// Always `None` in this build; see module docs.
    pub syscall_count: Option<u64>,
    pub process_spawn_count: usize,
    pub ipc_count: usize,
    pub network_count: usize,

    // Vocabulary/lifecycle state at the moment of this execution.
    pub permanent_words: usize,
    pub provisional_words: usize,
    pub candidate_count: usize,
    pub structural_changes: usize,

    // Specialization involvement, if any.
    pub optimization_used: bool,
    pub optimization_id: Option<String>,
    pub verification_status: Option<String>,
    pub rollback_count: usize,
}

impl ExecutionMetrics {
    pub fn csv_header() -> &'static str {
        "application_id,workload_id,task_id,episode_index,warmth,engine,\
representation_tokens,planning_decisions,wall_time_ns,cpu_time_ns,\
user_cpu_ns,system_cpu_ns,rss_kb,bytes_read,bytes_written,syscall_count,\
process_spawn_count,ipc_count,network_count,permanent_words,\
provisional_words,candidate_count,structural_changes,optimization_used,\
optimization_id,verification_status,rollback_count"
    }

    pub fn to_csv_row(&self) -> String {
        [
            csv_field(&self.application_id),
            csv_field(&self.workload_id),
            csv_field(&self.task_id),
            self.episode_index.to_string(),
            self.warmth.label().to_string(),
            self.engine.label(),
            self.representation_tokens.to_string(),
            self.planning_decisions.to_string(),
            self.wall_time_ns.to_string(),
            self.cpu_time_ns.to_string(),
            self.user_cpu_ns.to_string(),
            self.system_cpu_ns.to_string(),
            self.rss_kb.to_string(),
            self.bytes_read.to_string(),
            self.bytes_written.to_string(),
            self.syscall_count
                .map(|v| v.to_string())
                .unwrap_or_else(|| "not_measured".to_string()),
            self.process_spawn_count.to_string(),
            self.ipc_count.to_string(),
            self.network_count.to_string(),
            self.permanent_words.to_string(),
            self.provisional_words.to_string(),
            self.candidate_count.to_string(),
            self.structural_changes.to_string(),
            self.optimization_used.to_string(),
            csv_field(self.optimization_id.as_deref().unwrap_or("")),
            csv_field(self.verification_status.as_deref().unwrap_or("")),
            self.rollback_count.to_string(),
        ]
        .join(",")
    }
}

fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_measured_run_reports_nonzero_wall_time_across_real_work() {
        let run = MeasuredRun::start();
        let mut acc: u64 = 0;
        for i in 0..200_000u64 {
            acc = acc.wrapping_add(i.wrapping_mul(31));
        }
        std::hint::black_box(acc);
        let cost = run.finish();
        assert!(cost.wall_time_ns > 0);
        // cpu_time_ns is derived from integer clock-tick deltas, which can
        // legitimately read zero for a run shorter than one tick (~10ms);
        // it must never be negative/overflow, which the saturating
        // subtraction guarantees, so just check the type's contract holds.
        assert_eq!(cost.cpu_time_ns, cost.user_cpu_ns + cost.system_cpu_ns);
    }

    #[test]
    fn proc_self_stat_times_parse_on_a_real_linux_process() {
        // This crate targets real Linux processes (matching the rest of
        // the workspace); if /proc/self/stat doesn't parse in the test
        // environment, that's the fact this test exists to catch.
        assert!(read_proc_self_stat_times().is_some());
    }

    #[test]
    fn proc_self_status_rss_parses_on_a_real_linux_process() {
        assert!(read_proc_self_status_rss_kb().unwrap_or(0) > 0);
    }

    fn sample_record() -> ExecutionMetrics {
        ExecutionMetrics {
            application_id: "nginx".to_string(),
            workload_id: "api_get".to_string(),
            task_id: "t1".to_string(),
            episode_index: 7,
            warmth: Warmth::Warm,
            engine: Engine::Drm("provisional+permanent"),
            representation_tokens: 3,
            planning_decisions: 1,
            wall_time_ns: 12_345,
            cpu_time_ns: 10_000,
            user_cpu_ns: 8_000,
            system_cpu_ns: 2_000,
            rss_kb: 4096,
            bytes_read: 512,
            bytes_written: 256,
            syscall_count: None,
            process_spawn_count: 0,
            ipc_count: 1,
            network_count: 0,
            permanent_words: 12,
            provisional_words: 3,
            candidate_count: 1,
            structural_changes: 0,
            optimization_used: true,
            optimization_id: Some("spec-1".to_string()),
            verification_status: Some("VERIFIED".to_string()),
            rollback_count: 0,
        }
    }

    #[test]
    fn csv_row_has_exactly_as_many_fields_as_the_header() {
        let header_fields = ExecutionMetrics::csv_header().split(',').count();
        let row_fields = sample_record().to_csv_row().split(',').count();
        assert_eq!(header_fields, row_fields);
    }

    #[test]
    fn csv_row_renders_missing_syscall_count_as_not_measured_not_zero() {
        let row = sample_record().to_csv_row();
        assert!(row.contains("not_measured"));
        // Never silently render an unmeasured column as if it were "0" --
        // that would look like a real zero-syscall measurement.
    }

    #[test]
    fn csv_field_quotes_values_containing_commas() {
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("plain"), "plain");
    }
}
