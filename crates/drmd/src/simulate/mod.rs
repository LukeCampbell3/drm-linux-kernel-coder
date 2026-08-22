//! `drmd simulate {desktop|server}`: the comparative benchmark suites
//! (spec S12-S16). Generates a deterministic synthetic workload, runs it
//! through all eight comparative engines (five naive baselines, three
//! DRM configurations), and writes a per-episode metrics CSV, a
//! development-curves CSV, and a markdown summary with the adversarial
//! checks from spec S15 run as real assertions against the collected
//! data.
//!
//! This is explicitly a synthetic simulator, not a claim of hooking real
//! desktop/server application activity (see `scenario` module docs) --
//! the experimental question it answers is narrower and more honest:
//! *given* a recurring-structure workload with drift, noise, and
//! cross-application motifs, does DRM's learned/verified specialization
//! layer produce real, measured wall-clock savings beyond what naive
//! caching already gets for free, without ever changing observable
//! output? Spec S22's primary experimental question.

mod engine;
mod report;
mod scenario;

use std::path::Path;

pub use report::SimulationReport;

pub fn run_server(out_dir: &Path) -> std::io::Result<SimulationReport> {
    report::run(&scenario::server_scenario(), out_dir)
}

pub fn run_desktop(out_dir: &Path) -> std::io::Result<SimulationReport> {
    report::run(&scenario::desktop_scenario(), out_dir)
}
