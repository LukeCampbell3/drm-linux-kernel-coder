use std::fs;
use std::path::{Path, PathBuf};

use drm_exec::WatchLearner;

struct Family {
    name: &'static str,
    initial_ms: u64,
    initial_interventions: usize,
    slow: &'static [&'static str],
    efficient: &'static [&'static str],
    efficient_ms: [u64; 3],
}

const FAMILIES: &[Family] = &[
    Family {
        name: "web_research_to_notes",
        initial_ms: 1_200,
        initial_interventions: 2,
        slow: &["browser|navigate|search|query", "browser|click|result|", "browser|extract|article|", "notes|open|research|", "notes|write|research|summary"],
        efficient: &["browser|navigate|direct|", "browser|extract|article|", "notes|write|research|summary"],
        efficient_ms: [700, 650, 600],
    },
    Family {
        name: "spreadsheet_to_report",
        initial_ms: 1_100,
        initial_interventions: 2,
        slow: &["files|open|input.csv|", "spreadsheet|import|input.csv|", "spreadsheet|select|data|", "spreadsheet|summarize|data|", "files|save|report.csv|", "notes|write|report|complete"],
        efficient: &["spreadsheet|import|input.csv|", "spreadsheet|summarize|data|", "files|save|report.csv|"],
        efficient_ms: [650, 600, 550],
    },
];

pub struct SuiteReport {
    pub families: usize,
    pub observations: usize,
    pub certified: usize,
    pub initial_actions: usize,
    pub certified_actions: usize,
    pub initial_ms: u64,
    pub certified_ms: u64,
    pub interventions_observed: usize,
    pub shadow_evaluations: usize,
}

pub fn run(out: &Path) -> Result<SuiteReport, Box<dyn std::error::Error>> {
    fs::create_dir_all(out.join("traces"))?;
    let mut learner = WatchLearner::new(out.join("state"))?;
    let mut initial_actions = 0;
    let mut certified_actions = 0;
    let mut initial_ms = 0;
    let mut certified_ms = 0;
    let mut certified = 0;
    let mut run_id = 0;
    for family in FAMILIES {
        run_id += 1;
        learner.observe_file(out, &write_trace(out, run_id, family.name, family.slow, family.initial_ms, family.initial_interventions)?)?;
        initial_actions += family.slow.len();
        initial_ms += family.initial_ms;
        for duration in family.efficient_ms {
            run_id += 1;
            learner.observe_file(out, &write_trace(out, run_id, family.name, family.efficient, duration, 0)?)?;
        }
        if let Some((actions, _, duration)) = learner.certified_stats(family.name) {
            certified += 1;
            certified_actions += actions;
            certified_ms += duration;
        }
    }
    let report = SuiteReport {
        families: FAMILIES.len(), observations: learner.metrics.watched_tasks, certified,
        initial_actions, certified_actions, initial_ms, certified_ms,
        interventions_observed: learner.metrics.observed_interventions,
        shadow_evaluations: learner.metrics.shadow_evaluations,
    };
    fs::write(out.join("suite_summary.md"), summary(&report))?;
    Ok(report)
}

fn write_trace(out: &Path, run: usize, family: &str, actions: &[&str], duration: u64, interventions: usize) -> Result<PathBuf, std::io::Error> {
    let relative = PathBuf::from(format!("traces/{run}.trace"));
    let mut trace = format!("run_id={run}\nfamily={family}\nsuccess=true\nduration_ms={duration}\ninterventions={interventions}\n");
    for action in actions { trace.push_str(&format!("action={action}\n")); }
    fs::write(out.join(&relative), trace)?;
    Ok(relative)
}

fn summary(report: &SuiteReport) -> String {
    format!(
        "# Observe-first application-suite benchmark\n\nFamilies: {}. Observations watched: {}. Certified policies: {}.\n\n| Metric | Initial completed workflows | Certified reuse | Change |\n|---|---:|---:|---:|\n| Actions per suite cycle | {} | {} | {:.1}% |\n| Duration per suite cycle (ms) | {} | {} | {:.1}% |\n| User interventions observed while learning | {} | 0 projected | — |\n\nShadow evaluations: {}. No uncertified workflow executed against a live application.\n",
        report.families, report.observations, report.certified,
        report.initial_actions, report.certified_actions, reduction(report.initial_actions as f64, report.certified_actions as f64),
        report.initial_ms, report.certified_ms, reduction(report.initial_ms as f64, report.certified_ms as f64),
        report.interventions_observed, report.shadow_evaluations
    )
}

fn reduction(before: f64, after: f64) -> f64 { 100.0 * (before - after) / before }
