//! `drmd`: the DRM Adaptive Execution Layer service and CLI.
//!
//! Subcommands:
//! - `selftest`: fast in-memory invariant check (no I/O).
//! - `bench [--out DIR]`: run the frozen 99-episode regression workload.
//! - `agent-bench [--out DIR]`: run executable goal-driven program-repair tasks.
//! - `simulate <server|desktop> [--out DIR]`: run the comparative
//!   benchmark suite (five baselines + three DRM configurations) against
//!   a deterministic synthetic workload and write CSVs + a summary.
//! - `serve [--socket P] [--work D] [--state D] [--consolidate-ms N]`:
//!   run the long-lived episode-submission daemon.
//! - `submit ...`: submit one episode to a running daemon.
//! - `status [--socket P]`: query a running daemon's state.
//! - `applications`: list known applications.
//! - `application <id>`: one application's learned-state summary.
//! - `workload <id>`: which applications/words a workload identity uses.
//! - `learned [--app ID]`: every learned word, optionally filtered.
//! - `optimizations`: verified executable specializations.
//! - `metrics`: aggregate execution metrics.
//! - `explain <optimization-id>`: detail on one specialization.
//! - `reset <scope>`: `all` or `application:<id>`.

mod agent_bench;
mod bench;
mod cli;
mod client;
mod fmt;
mod protocol;
mod registry_state;
mod selftest;
mod serve;
mod simulate;
mod workload;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use cli::{positional, ParsedArgs, DEFAULT_BENCH_OUT, DEFAULT_SOCKET, DEFAULT_STATE_DIR, DEFAULT_WORK_DIR};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help() {
    println!(
        "drmd {VERSION} -- DRM Adaptive Execution Layer\n\n\
Usage:\n  \
  drmd selftest\n  \
  drmd bench [--out DIR]\n  \
  drmd agent-bench [--out DIR]\n  \
  drmd simulate <server|desktop> [--out DIR]\n  \
  drmd serve [--socket PATH] [--work DIR] [--state DIR] [--consolidate-ms N]\n  \
  drmd submit --task NAME --ops cap1,cap2,... [--app ID] [--workload ID] [--host ID] [--user ID] [--socket PATH] [--source PATH] [--output PATH] [--url PATH] [--ancestral]\n  \
  drmd status [--socket PATH]\n  \
  drmd applications [--socket PATH]\n  \
  drmd application <id> [--socket PATH]\n  \
  drmd workload <id> [--socket PATH]\n  \
  drmd learned [--app ID] [--socket PATH]\n  \
  drmd optimizations [--socket PATH]\n  \
  drmd metrics [--socket PATH]\n  \
  drmd explain <optimization-id> [--socket PATH]\n  \
  drmd reset <all|application:ID> [--socket PATH]\n  \
  drmd --version | --help\n\n\
Defaults:\n  \
  socket = {DEFAULT_SOCKET}\n  \
  work   = {DEFAULT_WORK_DIR}\n  \
  state  = {DEFAULT_STATE_DIR}\n"
    );
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().collect();
    let Some(command) = raw.get(1).cloned() else {
        print_help();
        return ExitCode::FAILURE;
    };
    let rest = &raw[2..];

    match command.as_str() {
        "-h" | "--help" | "help" => {
            print_help();
            ExitCode::SUCCESS
        }
        "-V" | "--version" => {
            println!("drmd {VERSION}");
            ExitCode::SUCCESS
        }
        "selftest" => {
            if selftest::run() {
                println!("SELF_TEST_PASS");
                ExitCode::SUCCESS
            } else {
                println!("SELF_TEST_FAIL");
                ExitCode::FAILURE
            }
        }
        "bench" => cmd_bench(rest),
        "agent-bench" => cmd_agent_bench(rest),
        "simulate" => cmd_simulate(rest),
        "serve" => cmd_serve(rest),
        "submit" => cmd_submit(rest),
        "status" => cmd_status(rest),
        "applications" => cmd_simple(rest, client::applications),
        "application" => cmd_with_id(rest, "application", client::application),
        "workload" => cmd_with_id(rest, "workload id", client::workload),
        "learned" => cmd_learned(rest),
        "optimizations" => cmd_simple(rest, client::optimizations),
        "metrics" => cmd_simple(rest, client::metrics),
        "explain" => cmd_with_id(rest, "optimization id", client::explain),
        "reset" => cmd_with_id(rest, "scope (all|application:ID)", client::reset),
        other => {
            eprintln!("drmd: unknown command `{other}`\n");
            print_help();
            ExitCode::FAILURE
        }
    }
}

fn cmd_agent_bench(args: &[String]) -> ExitCode {
    let parsed = ParsedArgs::parse(args);
    let out = parsed.path_or("out", "results/agent-bench");
    match agent_bench::run(&out) {
        Ok(report) => {
            println!(
                "tasks={} static={}/{} evolved={}/{} candidates={} committed={}",
                report.tasks,
                report.initial_passed,
                report.total_cases,
                report.final_passed,
                report.total_cases,
                report.candidates,
                report.committed
            );
            println!("report written to {}", out.display());
            if report.final_passed == report.total_cases {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("drmd: agent-bench failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_bench(args: &[String]) -> ExitCode {
    let parsed = ParsedArgs::parse(args);
    let out = parsed.path_or("out", DEFAULT_BENCH_OUT);
    match bench::run(&out) {
        Ok(report) => {
            println!(
                "episodes={} success={} semantic={} derived={} recoveries={} repairs={} struct={} dl_reduction={:.6}",
                report.episodes,
                report.success,
                report.semantic_total,
                report.derived_final,
                report.recoveries,
                report.local_repairs,
                report.structure_bytes_final,
                report.description_length_reduction
            );
            println!(
                "root_counts: OBSERVE={} DERIVE={} COMMIT={}",
                report.root_observe, report.root_derive, report.root_commit
            );
            println!("report written to {}", out.display());
            if report.success == report.episodes && report.uniform {
                ExitCode::SUCCESS
            } else {
                eprintln!(
                    "drmd: bench completed with failures (success={}/{}, uniform={})",
                    report.success, report.episodes, report.uniform
                );
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("drmd: bench failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_simulate(args: &[String]) -> ExitCode {
    let Some(which) = positional(args) else {
        eprintln!("drmd: simulate requires a positional target (server|desktop)");
        return ExitCode::FAILURE;
    };
    let which = which.to_string();
    let parsed = ParsedArgs::parse(&args[1..]);
    let out = parsed.path_or("out", "results/simulate");

    let result = match which.as_str() {
        "server" => simulate::run_server(&out),
        "desktop" => simulate::run_desktop(&out),
        other => {
            eprintln!("drmd: unknown simulate target `{other}` (expected `server` or `desktop`)");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(report) => {
            println!(
                "simulation={} episodes={} engines={}",
                report.scenario_name,
                report.episodes,
                report.engines.len()
            );
            let failed: Vec<&str> = report
                .adversarial_checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| c.name.as_str())
                .collect();
            for c in &report.adversarial_checks {
                println!("[{}] {}", if c.passed { "PASS" } else { "FAIL" }, c.name);
            }
            println!("report written to {}", out.display());
            if failed.is_empty() {
                ExitCode::SUCCESS
            } else {
                eprintln!("drmd: {} adversarial check(s) failed: {}", failed.len(), failed.join(", "));
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("drmd: simulate failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_serve(args: &[String]) -> ExitCode {
    let parsed = ParsedArgs::parse(args);
    let socket_path = parsed.path_or("socket", DEFAULT_SOCKET);
    let work_dir = parsed.path_or("work", DEFAULT_WORK_DIR);
    let state_dir = parsed.path_or("state", DEFAULT_STATE_DIR);
    let consolidate_ms: u64 = parsed.get("consolidate-ms").and_then(|s| s.parse().ok()).unwrap_or(250);

    if let Some(parent) = socket_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("drmd: failed to create socket directory {}: {e}", parent.display());
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = std::fs::create_dir_all(&work_dir) {
        eprintln!("drmd: failed to create work directory {}: {e}", work_dir.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::create_dir_all(&state_dir) {
        eprintln!("drmd: failed to create state directory {}: {e}", state_dir.display());
        return ExitCode::FAILURE;
    }

    let opts = serve::ServeOptions {
        socket_path,
        work_dir,
        state_dir,
        consolidation_interval: Duration::from_millis(consolidate_ms),
    };
    match serve::run(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("drmd: serve failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_submit(args: &[String]) -> ExitCode {
    let parsed = ParsedArgs::parse(args);
    let socket: PathBuf = parsed.path_or("socket", DEFAULT_SOCKET);
    let Some(task) = parsed.get("task") else {
        eprintln!("drmd: submit requires --task NAME");
        return ExitCode::FAILURE;
    };
    let Some(ops) = parsed.get("ops") else {
        eprintln!("drmd: submit requires --ops cap1,cap2,...");
        return ExitCode::FAILURE;
    };
    let submit_args = client::SubmitArgs {
        task: task.to_string(),
        ops: ops.split(',').map(|s| s.to_string()).collect(),
        source: parsed.get_or("source", ""),
        output: parsed.get_or("output", ""),
        url: parsed.get_or("url", ""),
        ancestral: parsed.has("ancestral"),
        application: parsed.get("app").map(|s| s.to_string()),
        workload: parsed.get("workload").map(|s| s.to_string()),
        host: parsed.get("host").map(|s| s.to_string()),
        user: parsed.get("user").map(|s| s.to_string()),
    };
    match client::submit(&socket, &submit_args) {
        Ok(response) => {
            println!("{response}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("drmd: submit failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_status(args: &[String]) -> ExitCode {
    cmd_simple(args, client::status)
}

/// Subcommands that take only `--socket PATH` and no other arguments.
fn cmd_simple(args: &[String], f: fn(&std::path::Path) -> std::io::Result<String>) -> ExitCode {
    let parsed = ParsedArgs::parse(args);
    let socket: PathBuf = parsed.path_or("socket", DEFAULT_SOCKET);
    respond(f(&socket))
}

/// Subcommands of the shape `drmd <command> <id> [--socket PATH]`.
fn cmd_with_id(args: &[String], id_desc: &str, f: fn(&std::path::Path, &str) -> std::io::Result<String>) -> ExitCode {
    let Some(id) = positional(args) else {
        eprintln!("drmd: this command requires a positional {id_desc}");
        return ExitCode::FAILURE;
    };
    let parsed = ParsedArgs::parse(&args[1..]);
    let socket: PathBuf = parsed.path_or("socket", DEFAULT_SOCKET);
    respond(f(&socket, id))
}

fn cmd_learned(args: &[String]) -> ExitCode {
    let parsed = ParsedArgs::parse(args);
    let socket: PathBuf = parsed.path_or("socket", DEFAULT_SOCKET);
    let app_filter = parsed.get("app");
    respond(client::learned(&socket, app_filter))
}

fn respond(result: std::io::Result<String>) -> ExitCode {
    match result {
        Ok(response) => {
            println!("{response}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("drmd: request failed: {e}");
            ExitCode::FAILURE
        }
    }
}
