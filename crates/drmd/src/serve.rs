//! `drmd serve`: a long-running Unix-domain-socket daemon. This is the
//! actual product -- the point at which the DRM runtime stops being a
//! benchmark that runs a canned workload and exits, and becomes something
//! external callers can submit real episodes to.
//!
//! Architecture: a single `Mutex<Registry>` guards every application's
//! planner and metadata; a background thread periodically drains queued
//! vocabulary maintenance via [`drm_core::registry::Registry::consolidate`]
//! (which itself calls each application's deferred
//! `HybridPlanner::consolidate_pending`) so that submitting an episode
//! never pays for whole-corpus MDL rescoring, or cross-application
//! promotion scanning, inline. Each accepted connection is handled on its
//! own short-lived thread; state access is serialized through the mutex,
//! trading fine-grained concurrency for straightforward correctness,
//! which is the right default until profiling says otherwise.
//!
//! Persistence: the registry is loaded from `$STATE_DIR/registry.json` on
//! startup (if present and schema-compatible; see `registry_state`), and
//! snapshotted on the same background timer that drives consolidation,
//! plus best-effort on SIGTERM/SIGINT. Every commit an executed
//! capability makes to the filesystem is independently atomic
//! (write-then-rename or append), so even a `kill -9` between snapshots
//! never corrupts on-disk *capability* state -- it only loses vocabulary
//! learned since the last snapshot, which the next snapshot recovers.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use drm_core::identity::ExecutionContext;
use drm_core::registry::Registry;
use drm_exec::LiveExecutor;

use crate::fmt::{json_string, json_string_array};
use crate::protocol::{parse_request, Request};
use crate::registry_state;

struct ServerState {
    registry: Registry,
    executor: LiveExecutor,
    started_at: Instant,
    episodes_planned: usize,
    next_idx: usize,
}

pub struct ServeOptions {
    pub socket_path: PathBuf,
    pub work_dir: PathBuf,
    pub state_dir: PathBuf,
    pub consolidation_interval: Duration,
}

pub fn run(opts: ServeOptions) -> std::io::Result<()> {
    registry_state::install_shutdown_handler();

    let executor =
        LiveExecutor::start(opts.work_dir.clone()).map_err(|e| std::io::Error::other(format!("failed to start executor: {e}")))?;
    let registry = registry_state::load(&opts.state_dir).unwrap_or_else(|| {
        eprintln!(
            "drmd: no prior state at {} -- starting with an empty registry",
            opts.state_dir.display()
        );
        Registry::new()
    });
    let restored_apps = registry.application_ids().len();
    let state = Arc::new(Mutex::new(ServerState {
        registry,
        executor,
        started_at: Instant::now(),
        episodes_planned: 0,
        next_idx: 0,
    }));

    // Background consolidation + snapshot: applies queued vocabulary
    // maintenance and cross-application promotion off the request path,
    // then persists. Also the thread that notices a SIGTERM/SIGINT flag
    // and takes one final snapshot before the process exits.
    {
        let state = state.clone();
        let interval = opts.consolidation_interval;
        let state_dir = opts.state_dir.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(interval);
            if let Ok(mut s) = state.lock() {
                s.registry.consolidate();
                if let Err(e) = registry_state::save(&s.registry, &state_dir) {
                    eprintln!("drmd: failed to snapshot state: {e}");
                }
            }
            if registry_state::SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
                if let Ok(s) = state.lock() {
                    let _ = registry_state::save(&s.registry, &state_dir);
                }
                eprintln!("drmd: shutting down (signal received), final snapshot written");
                std::process::exit(0);
            }
        });
    }

    let _ = std::fs::remove_file(&opts.socket_path);
    let listener = UnixListener::bind(&opts.socket_path)?;
    eprintln!(
        "drmd: listening on {} (work dir: {}, state dir: {}, {} application(s) restored)",
        opts.socket_path.display(),
        opts.work_dir.display(),
        opts.state_dir.display(),
        restored_apps
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
        Ok(Request::Applications) => applications_response(&guard),
        Ok(Request::Application(id)) => application_response(&guard, &id),
        Ok(Request::Workload(id)) => workload_response(&guard, &id),
        Ok(Request::Learned(app_filter)) => learned_response(&guard, app_filter.as_deref()),
        Ok(Request::Optimizations) => "{\"ok\":true,\"optimizations\":[]}".to_string(),
        Ok(Request::Metrics) => metrics_response(&guard),
        Ok(Request::Explain(id)) => format!(
            "{{\"ok\":false,\"error\":{}}}",
            json_string(&format!(
                "no optimization with id `{id}` (drm-opt specialization tracking not yet populated in this build)"
            ))
        ),
        Ok(Request::Reset(scope)) => reset_response(&mut guard, &scope),
        Ok(Request::Submit(ep)) => {
            let ctx = ep.ctx.clone();
            let pm = guard.registry.plan(&ctx, ep.ops.clone(), ep.phase.clone(), ep.ancestral);
            let exec_result = guard.executor.execute(&ep);
            guard.episodes_planned += 1;
            submit_response(&ctx, next_idx, &pm, exec_result.err().map(|e| e.to_string()))
        }
        Err(e) => format!("{{\"ok\":false,\"error\":{}}}", json_string(&e.to_string())),
    };
    drop(guard);

    writeln!(writer, "{response}")
}

fn submit_response(ctx: &ExecutionContext, idx: usize, pm: &drm_core::PlanMetrics, error: Option<String>) -> String {
    let ok = error.is_none();
    let error_field = match error {
        Some(e) => format!(",\"error\":{}", json_string(&e)),
        None => String::new(),
    };
    format!(
        "{{\"ok\":{ok},\"episode\":{idx},\"application\":{},\"workload\":{},\"task\":{},\"semantic\":{},\"recovery\":{},\"local_repair\":{},\"structural_change\":{},\"derived\":{},\"active\":{},\"structure_bytes\":{},\"uniform\":{}{error_field}}}",
        json_string(&ctx.application_id),
        json_string(&ctx.workload_id),
        json_string(&ctx.task_id),
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
        "{{\"ok\":true,\"uptime_secs\":{:.3},\"episodes_planned\":{},\"applications\":{},\"global_words\":{},\"commits\":{},\"process_spawns\":{},\"tcp_requests\":{},\"ipc_requests\":{},\"timer_events\":{},\"web_requests\":{},\"mutation_candidates\":{},\"mutations_committed\":{}}}",
        s.started_at.elapsed().as_secs_f64(),
        s.episodes_planned,
        s.registry.application_ids().len(),
        s.registry.global.vocab.derived.len(),
        s.executor.commits,
        s.executor.process_spawns,
        s.executor.tcp_requests,
        s.executor.ipc_requests,
        s.executor.timer_events,
        s.executor.web_requests,
        s.executor.mutation_candidates,
        s.executor.mutations_committed,
    )
}

fn applications_response(s: &ServerState) -> String {
    let mut ids = s.registry.application_ids();
    ids.sort();
    format!("{{\"ok\":true,\"applications\":{}}}", json_string_array(&ids))
}

fn application_response(s: &ServerState, id: &str) -> String {
    let Some(app) = s.registry.applications.get(id) else {
        return format!("{{\"ok\":false,\"error\":{}}}", json_string(&format!("no application `{id}`")));
    };
    let permanent = app.planner.base.vocab.derived.len();
    let provisional = app.planner.provisional_words();
    let verified_specializations = 0; // populated once drm-opt is wired into this response
    format!(
        "{{\"ok\":true,\"application\":{},\"observed_executions\":{},\"permanent_words\":{permanent},\"provisional_words\":{provisional},\"verified_specializations\":{verified_specializations},\"created_step\":{}}}",
        json_string(id),
        app.planner.base.version,
        app.created_step,
    )
}

/// A workload has no vocabulary of its own (see `registry` module docs:
/// workload is a transfer-evidence dimension on each word, not a
/// separate scoring tier), so "info about a workload" means "which
/// words, in which applications, have this workload in their
/// `used_by_workloads` set."
fn workload_response(s: &ServerState, workload_id: &str) -> String {
    let mut words = Vec::new();
    let mut applications = std::collections::BTreeSet::new();
    for (app_id, app) in &s.registry.applications {
        for (name, meta) in &app.word_meta {
            if meta.used_by_workloads.contains(workload_id) {
                applications.insert(app_id.clone());
                words.push(format!(
                    "{{\"application\":{},\"word\":{},\"stage\":{}}}",
                    json_string(app_id),
                    json_string(name),
                    json_string(&format!("{:?}", meta.stage))
                ));
            }
        }
    }
    let apps: Vec<String> = applications.into_iter().collect();
    format!(
        "{{\"ok\":true,\"workload\":{},\"applications\":{},\"words\":[{}]}}",
        json_string(workload_id),
        json_string_array(&apps),
        words.join(",")
    )
}

fn learned_response(s: &ServerState, app_filter: Option<&str>) -> String {
    let mut words = Vec::new();
    for (app_id, app) in &s.registry.applications {
        if let Some(f) = app_filter {
            if f != app_id {
                continue;
            }
        }
        for (name, meta) in &app.word_meta {
            words.push(format!(
                "{{\"application\":{},\"word\":{},\"stage\":{},\"usage_count\":{},\"transferred_within_application\":{},\"birth_task\":{}}}",
                json_string(app_id),
                json_string(name),
                json_string(&format!("{:?}", meta.stage)),
                meta.usage_count,
                meta.transferred_within_application(),
                json_string(&meta.birth.task_id),
            ));
        }
    }
    for (name, meta) in &s.registry.global.word_meta {
        words.push(format!(
            "{{\"application\":\"__global__\",\"word\":{},\"stage\":{},\"usage_count\":{},\"transferred_within_application\":true,\"birth_task\":{}}}",
            json_string(name),
            json_string(&format!("{:?}", meta.stage)),
            meta.usage_count,
            json_string(&meta.admission_evidence),
        ));
    }
    format!("{{\"ok\":true,\"words\":[{}]}}", words.join(","))
}

fn metrics_response(s: &ServerState) -> String {
    format!(
        "{{\"ok\":true,\"episodes_planned\":{},\"applications\":{},\"global_words\":{},\"commits\":{},\"process_spawns\":{},\"tcp_requests\":{},\"ipc_requests\":{},\"web_requests\":{},\"mutation_candidates\":{},\"mutations_committed\":{}}}",
        s.episodes_planned,
        s.registry.application_ids().len(),
        s.registry.global.vocab.derived.len(),
        s.executor.commits,
        s.executor.process_spawns,
        s.executor.tcp_requests,
        s.executor.ipc_requests,
        s.executor.web_requests,
        s.executor.mutation_candidates,
        s.executor.mutations_committed,
    )
}

fn reset_response(guard: &mut ServerState, scope: &str) -> String {
    if scope == "all" {
        guard.registry = Registry::new();
        return "{\"ok\":true,\"reset\":\"all\"}".to_string();
    }
    if let Some(app_id) = scope.strip_prefix("application:") {
        let existed = guard.registry.applications.remove(app_id).is_some();
        return format!("{{\"ok\":{existed},\"reset\":{}}}", json_string(scope));
    }
    format!(
        "{{\"ok\":false,\"error\":{}}}",
        json_string(&format!("unknown reset scope `{scope}` (expected `all` or `application:<id>`)"))
    )
}
