//! `drmd serve`: a long-running Unix-domain-socket daemon. This is the
//! actual product -- the point at which the DRM runtime stops being a
//! benchmark that runs a canned workload and exits, and becomes something
//! external callers can submit real episodes to.
//!
//! Architecture: a single `Mutex<ServerState>` guards the planner and
//! executor; a background thread periodically drains queued vocabulary
//! maintenance via [`HybridPlanner::consolidate_pending`] so that
//! submitting an episode never pays for whole-corpus MDL rescoring inline
//! (see `drm-core`'s `hybrid` module docs for why that matters). Each
//! accepted connection is handled on its own short-lived thread; state
//! access is serialized through the mutex, trading fine-grained
//! concurrency for straightforward correctness, which is the right
//! default until profiling says otherwise.
//!
//! There is no persistence across restarts in v1: the learned vocabulary
//! and history live only in memory. Every commit an executed capability
//! makes to the filesystem is atomic (write-then-rename or append), so an
//! unplanned restart -- including systemd's default `SIGTERM` -- never
//! corrupts on-disk state; it only forgets what the planner had learned.
//! Persisting/restoring vocabulary state is tracked as a v2 roadmap item
//! (see `docs/ARCHITECTURE.md`).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use drm_core::HybridPlanner;
use drm_exec::LiveExecutor;

use crate::fmt::json_string;
use crate::protocol::{parse_request, Request};

struct ServerState {
    planner: HybridPlanner,
    executor: LiveExecutor,
    started_at: Instant,
    episodes_planned: usize,
    next_idx: usize,
}

pub struct ServeOptions {
    pub socket_path: PathBuf,
    pub work_dir: PathBuf,
    pub consolidation_interval: Duration,
}

pub fn run(opts: ServeOptions) -> std::io::Result<()> {
    let executor =
        LiveExecutor::start(opts.work_dir.clone()).map_err(|e| std::io::Error::other(format!("failed to start executor: {e}")))?;
    let state = Arc::new(Mutex::new(ServerState {
        planner: HybridPlanner::default(),
        executor,
        started_at: Instant::now(),
        episodes_planned: 0,
        next_idx: 0,
    }));

    // Background consolidation: applies queued vocabulary maintenance
    // (permanent-word growth, provisional admission/expiry) off the
    // request path.
    {
        let state = state.clone();
        let interval = opts.consolidation_interval;
        std::thread::spawn(move || loop {
            std::thread::sleep(interval);
            if let Ok(mut s) = state.lock() {
                s.planner.consolidate_pending();
            }
        });
    }

    let _ = std::fs::remove_file(&opts.socket_path);
    let listener = UnixListener::bind(&opts.socket_path)?;
    eprintln!(
        "drmd: listening on {} (work dir: {})",
        opts.socket_path.display(),
        opts.work_dir.display()
    );

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("drmd: accept error: {e}");
                continue;
            }
        };
        let state = state.clone();
        std::thread::spawn(move || {
            if let Err(e) = handle_connection(stream, &state) {
                eprintln!("drmd: connection error: {e}");
            }
        });
    }
    Ok(())
}

fn handle_connection(stream: UnixStream, state: &Arc<Mutex<ServerState>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(()); // client disconnected without sending anything
    }
    let mut writer = stream;

    let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
    guard.next_idx += 1;
    let next_idx = guard.next_idx;

    let response = match parse_request(&line, next_idx) {
        Ok(Request::Status) => status_response(&guard),
        Ok(Request::Submit(ep)) => {
            let pm = guard.planner.plan(&ep);
            let exec_result = guard.executor.execute(&ep);
            guard.episodes_planned += 1;
            submit_response(&ep.task, next_idx, &pm, exec_result.err().map(|e| e.to_string()))
        }
        Err(e) => format!("{{\"ok\":false,\"error\":{}}}", json_string(&e.to_string())),
    };
    drop(guard);

    writeln!(writer, "{response}")
}

fn submit_response(task: &str, idx: usize, pm: &drm_core::PlanMetrics, error: Option<String>) -> String {
    let ok = error.is_none();
    let error_field = match error {
        Some(e) => format!(",\"error\":{}", json_string(&e)),
        None => String::new(),
    };
    format!(
        "{{\"ok\":{ok},\"episode\":{idx},\"task\":{},\"semantic\":{},\"recovery\":{},\"local_repair\":{},\"structural_change\":{},\"derived\":{},\"active\":{},\"structure_bytes\":{},\"uniform\":{}{error_field}}}",
        json_string(task),
        pm.semantic,
        pm.recovery,
        pm.local_repair,
        pm.structural_change,
        pm.derived,
        pm.active,
        pm.structure_bytes,
        pm.uniform,
    )
}

fn status_response(s: &ServerState) -> String {
    format!(
        "{{\"ok\":true,\"uptime_secs\":{:.3},\"episodes_planned\":{},\"derived_words\":{},\"provisional_words\":{},\"active_tasks\":{},\"history_tasks\":{},\"uniform_vocabulary\":{},\"commits\":{},\"process_spawns\":{},\"tcp_requests\":{},\"ipc_requests\":{},\"timer_events\":{}}}",
        s.started_at.elapsed().as_secs_f64(),
        s.episodes_planned,
        s.planner.base.vocab.derived.len(),
        s.planner.provisional_words(),
        s.planner.base.active.len(),
        s.planner.base.history.len(),
        s.planner.base.vocab.audit(),
        s.executor.commits,
        s.executor.process_spawns,
        s.executor.tcp_requests,
        s.executor.ipc_requests,
        s.executor.timer_events,
    )
}
