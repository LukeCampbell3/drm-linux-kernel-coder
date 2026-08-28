use std::fs;
use std::path::{Path, PathBuf};

use drm_exec::{evolve_task, MutationReport};

struct TaskFixture {
    name: &'static str,
    source: &'static str,
    cases: &'static [(&'static str, &'static str, &'static str)],
}

const TASKS: &[TaskFixture] = &[
    TaskFixture {
        name: "boundary_repair",
        source: "value = int(input())\nprint('high' if value > 10 else 'low')\n",
        cases: &[("below", "9\n", "low"), ("boundary", "10\n", "high"), ("above", "11\n", "high")],
    },
    TaskFixture {
        name: "retry_policy",
        source: "attempts = int(input())\nprint(min(attempts, 2))\n",
        cases: &[("one", "1\n", "1"), ("target", "3\n", "3"), ("cap", "5\n", "3")],
    },
    TaskFixture {
        name: "weighted_scoring",
        source: "wins, draws = map(int, input().split())\nprint(wins * 1 + draws)\n",
        cases: &[("none", "0 0\n", "0"), ("mixed", "2 1\n", "5"), ("wins", "3 0\n", "6"), ("draws", "0 4\n", "4")],
    },
];

pub struct AgentBenchReport {
    pub tasks: usize,
    pub initial_passed: usize,
    pub final_passed: usize,
    pub total_cases: usize,
    pub candidates: usize,
    pub committed: usize,
}

pub fn run(out: &Path) -> Result<AgentBenchReport, Box<dyn std::error::Error>> {
    fs::create_dir_all(out)?;
    let mut rows: Vec<(&str, MutationReport)> = Vec::new();
    for fixture in TASKS {
        let task_dir = out.join(fixture.name);
        fs::create_dir_all(&task_dir)?;
        fs::write(task_dir.join("program.py"), fixture.source)?;
        fs::write(task_dir.join("goal.drm"), manifest(fixture))?;
        rows.push((fixture.name, evolve_task(out, &PathBuf::from(fixture.name).join("goal.drm"))?));
    }

    let report = AgentBenchReport {
        tasks: rows.len(),
        initial_passed: rows.iter().map(|(_, report)| report.initial_passed).sum(),
        final_passed: rows.iter().map(|(_, report)| report.final_passed).sum(),
        total_cases: rows.iter().map(|(_, report)| report.total_cases).sum(),
        candidates: rows.iter().map(|(_, report)| report.candidates_evaluated).sum(),
        committed: rows.iter().map(|(_, report)| report.mutations_committed).sum(),
    };
    let mut csv = String::from("task,initial_passed,final_passed,total_cases,candidates_evaluated,mutations_committed,elapsed_ms\n");
    for (name, row) in &rows {
        csv.push_str(&format!("{name},{},{},{},{},{},{}\n", row.initial_passed, row.final_passed, row.total_cases, row.candidates_evaluated, row.mutations_committed, row.elapsed_ms));
    }
    fs::write(out.join("agentic_metrics.csv"), csv)?;
    fs::write(out.join("agentic_summary.md"), summary(&report, &rows))?;
    Ok(report)
}

fn manifest(fixture: &TaskFixture) -> String {
    let mut value = format!("source={}/program.py\nmax_candidates=256\ntimeout_ms=1000\n", fixture.name);
    for (name, input, expected) in fixture.cases {
        value.push_str(&format!("case={name}|{}|{}\n", input.replace('\n', "\\n"), expected.replace('\n', "\\n")));
    }
    value
}

fn summary(report: &AgentBenchReport, rows: &[(&str, MutationReport)]) -> String {
    let mut value = format!(
        "# Agentic mutation benchmark\n\nStatic baseline: {}/{} cases ({:.1}%).\n\nDevelopmental runtime: {}/{} cases ({:.1}%).\n\nCandidates evaluated: {}. Mutations committed: {}.\n\n| Task | Before | After | Candidates | Commits | ms |\n|---|---:|---:|---:|---:|---:|\n",
        report.initial_passed, report.total_cases, percentage(report.initial_passed, report.total_cases),
        report.final_passed, report.total_cases, percentage(report.final_passed, report.total_cases),
        report.candidates, report.committed
    );
    for (name, row) in rows {
        value.push_str(&format!("| {name} | {}/{} | {}/{} | {} | {} | {} |\n", row.initial_passed, row.total_cases, row.final_passed, row.total_cases, row.candidates_evaluated, row.mutations_committed, row.elapsed_ms));
    }
    value
}

fn percentage(value: usize, total: usize) -> f64 { 100.0 * value as f64 / total as f64 }
