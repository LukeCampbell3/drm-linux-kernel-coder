//! `drmd`: the DRM O/D/C developmental runtime service and CLI.
//!
//! Subcommands:
//! - `selftest`: fast in-memory invariant check (no I/O).
//! - `bench [--out DIR]`: run the frozen 99-episode regression workload.
//! - `serve [--socket P] [--work D] [--consolidate-ms N]`: run the
//!   long-lived episode-submission daemon.
//! - `submit ...`: submit one episode to a running daemon.
//! - `status [--socket P]`: query a running daemon's state.

mod bench;
mod cli;
mod client;
mod fmt;
mod protocol;
mod selftest;
mod serve;
mod workload;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use cli::{ParsedArgs, DEFAULT_BENCH_OUT, DEFAULT_SOCKET, DEFAULT_WORK_DIR};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help() {
    println!(
        "drmd {VERSION} -- DRM O/D/C developmental runtime\n\n\
Usage:\n  \
  drmd selftest\n  \
  drmd bench [--out DIR]\n  \
  drmd serve [--socket PATH] [--work DIR] [--consolidate-ms N]\n  \
  drmd submit --task NAME --ops cap1,cap2,... [--socket PATH] [--source PATH] [--output PATH] [--url PATH] [--ancestral]\n  \
  drmd status [--socket PATH]\n  \
  drmd --version | --help\n\n\
Defaults:\n  \
  socket = {DEFAULT_SOCKET}\n  \
  work   = {DEFAULT_WORK_DIR}\n"
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
        "serve" => cmd_serve(rest),
        "submit" => cmd_submit(rest),
        "status" => cmd_status(rest),
        other => {
            eprintln!("drmd: unknown command `{other}`\n");
            print_help();
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

fn cmd_serve(args: &[String]) -> ExitCode {
    let parsed = ParsedArgs::parse(args);
    let socket_path = parsed.path_or("socket", DEFAULT_SOCKET);
    let work_dir = parsed.path_or("work", DEFAULT_WORK_DIR);
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

    let opts = serve::ServeOptions {
        socket_path,
        work_dir,
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
    let parsed = ParsedArgs::parse(args);
    let socket: PathBuf = parsed.path_or("socket", DEFAULT_SOCKET);
    match client::status(&socket) {
        Ok(response) => {
            println!("{response}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("drmd: status failed: {e}");
            ExitCode::FAILURE
        }
    }
}
