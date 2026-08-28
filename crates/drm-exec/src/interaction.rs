//! Observe-first workflow learning and guarded application-suite execution.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::executor::ExecError;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct Action {
    application: String,
    verb: String,
    target: String,
    value: String,
}

#[derive(Clone, Debug)]
struct Trace {
    run_id: String,
    family: String,
    success: bool,
    duration_ms: u64,
    interventions: usize,
    actions: Vec<Action>,
}

#[derive(Clone, Debug)]
struct CertifiedPolicy {
    actions: Vec<Action>,
    independent_successes: usize,
    median_duration_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct WatchMetrics {
    pub watched_tasks: usize,
    pub successful_tasks: usize,
    pub shadow_evaluations: usize,
    pub certified_policies: usize,
    pub observed_actions: usize,
    pub observed_interventions: usize,
}

pub struct WatchLearner {
    traces: HashMap<String, Vec<Trace>>,
    certified: HashMap<String, CertifiedPolicy>,
    metrics_path: PathBuf,
    pub metrics: WatchMetrics,
    min_independent_successes: usize,
}

impl WatchLearner {
    pub fn new(state_dir: PathBuf) -> Result<Self, ExecError> {
        fs::create_dir_all(&state_dir)?;
        Ok(Self {
            traces: HashMap::new(),
            certified: HashMap::new(),
            metrics_path: state_dir.join("longitudinal_metrics.csv"),
            metrics: WatchMetrics::default(),
            min_independent_successes: 3,
        })
    }

    pub fn observe_file(&mut self, work: &Path, relative: &Path) -> Result<String, ExecError> {
        let content = fs::read_to_string(confined(work, relative)?)?;
        if content.len() > 1_048_576 {
            return Err(ExecError::AppDenied("trace exceeds 1048576 bytes".into()));
        }
        let trace = parse_trace(&content)?;
        self.metrics.watched_tasks += 1;
        self.metrics.observed_actions += trace.actions.len();
        self.metrics.observed_interventions += trace.interventions;
        if trace.success { self.metrics.successful_tasks += 1; }
        let family = trace.family.clone();
        self.traces.entry(family.clone()).or_default().push(trace);
        self.reconsider(&family)?;
        self.append_metrics(&family)?;
        Ok(self.family_json(&family))
    }

    pub fn execute_certified(&self, family: &str, apps: &AppConfig) -> Result<(usize, u64), ExecError> {
        let policy = self.certified.get(family).ok_or_else(|| ExecError::AppDenied(format!("task family `{family}` has no certified workflow")))?;
        for action in &policy.actions { apps.execute(action)?; }
        Ok((policy.actions.len(), policy.median_duration_ms))
    }

    pub fn is_certified(&self, family: &str) -> bool { self.certified.contains_key(family) }

    pub fn certified_stats(&self, family: &str) -> Option<(usize, usize, u64)> {
        self.certified.get(family).map(|policy| {
            (policy.actions.len(), policy.independent_successes, policy.median_duration_ms)
        })
    }

    fn reconsider(&mut self, family: &str) -> Result<(), ExecError> {
        let traces = self.traces.get(family).expect("family inserted before reconsider");
        let mut candidates: HashMap<Vec<Action>, Vec<&Trace>> = HashMap::new();
        for trace in traces.iter().filter(|trace| trace.success) {
            candidates.entry(trace.actions.clone()).or_default().push(trace);
        }
        self.metrics.shadow_evaluations += candidates.len();
        let best = candidates.into_iter().filter_map(|(actions, supporting)| {
            let independent: HashSet<&str> = supporting.iter().map(|trace| trace.run_id.as_str()).collect();
            if independent.len() < self.min_independent_successes { return None; }
            let failures = traces.iter().filter(|trace| !trace.success && trace.actions == actions).count();
            let success_rate = supporting.len() as f64 / (supporting.len() + failures) as f64;
            if success_rate < 0.9 { return None; }
            let mut durations: Vec<u64> = supporting.iter().map(|trace| trace.duration_ms).collect();
            durations.sort_unstable();
            Some((actions, independent.len(), durations[durations.len() / 2]))
        }).min_by_key(|(actions, _, duration)| (actions.len(), *duration));

        if let Some((actions, independent_successes, median_duration_ms)) = best {
            let should_replace = self.certified.get(family).map_or(true, |old| {
                actions.len() < old.actions.len()
                    || (actions.len() == old.actions.len() && median_duration_ms < old.median_duration_ms)
            });
            if should_replace {
                self.certified.insert(family.to_string(), CertifiedPolicy { actions, independent_successes, median_duration_ms });
                self.metrics.certified_policies += 1;
            }
        }
        Ok(())
    }

    fn append_metrics(&self, family: &str) -> Result<(), ExecError> {
        let new_file = !self.metrics_path.exists();
        let mut output = OpenOptions::new().create(true).append(true).open(&self.metrics_path)?;
        if new_file {
            writeln!(output, "observation,family,success_rate,mean_actions,mean_duration_ms,mean_interventions,shadow_evaluations,certified,certified_actions")?;
        }
        let traces = self.traces.get(family).expect("known family");
        let successes = traces.iter().filter(|trace| trace.success).count();
        let mean_actions = traces.iter().map(|trace| trace.actions.len()).sum::<usize>() as f64 / traces.len() as f64;
        let mean_duration = traces.iter().map(|trace| trace.duration_ms).sum::<u64>() as f64 / traces.len() as f64;
        let mean_interventions = traces.iter().map(|trace| trace.interventions).sum::<usize>() as f64 / traces.len() as f64;
        let certified_actions = self.certified.get(family).map_or(0, |policy| policy.actions.len());
        writeln!(output, "{},{},{:.4},{:.3},{:.3},{:.3},{},{},{}", self.metrics.watched_tasks, family, successes as f64 / traces.len() as f64, mean_actions, mean_duration, mean_interventions, self.metrics.shadow_evaluations, self.certified.contains_key(family), certified_actions)?;
        Ok(())
    }

    fn family_json(&self, family: &str) -> String {
        let traces = self.traces.get(family).expect("known family");
        match self.certified.get(family) {
            Some(policy) => format!("{{\"family\":\"{family}\",\"observations\":{},\"certified\":true,\"certified_actions\":{},\"independent_successes\":{},\"median_duration_ms\":{}}}", traces.len(), policy.actions.len(), policy.independent_successes, policy.median_duration_ms),
            None => format!("{{\"family\":\"{family}\",\"observations\":{},\"certified\":false}}", traces.len()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub adapter_dir: PathBuf,
    pub allowed_applications: HashSet<String>,
    pub allow_risky: bool,
}

impl AppConfig {
    pub fn from_env() -> Option<Self> {
        let adapter_dir = PathBuf::from(std::env::var_os("DRMD_APP_ADAPTER_DIR")?);
        let allowed_applications = std::env::var("DRMD_APP_ALLOWED")
            .ok()?
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect();
        Some(Self { adapter_dir, allowed_applications, allow_risky: std::env::var("DRMD_APP_ALLOW_RISKY").as_deref() == Ok("1") })
    }

    fn execute(&self, action: &Action) -> Result<(), ExecError> {
        if !self.allowed_applications.contains(&action.application) {
            return Err(ExecError::AppDenied(format!("application `{}` is not allowlisted", action.application)));
        }
        if !self.allow_risky && matches!(action.verb.as_str(), "delete" | "purchase" | "send" | "submit" | "authenticate") {
            return Err(ExecError::AppDenied(format!("risky action `{}` requires explicit authorization", action.verb)));
        }
        let adapter = self.adapter_dir.join(&action.application);
        if !adapter.is_file() || adapter.symlink_metadata()?.file_type().is_symlink() {
            return Err(ExecError::AppDenied(format!("invalid adapter for `{}`", action.application)));
        }
        let status = Command::new(adapter).args([&action.verb, &action.target, &action.value]).status()?;
        if status.success() { Ok(()) } else { Err(ExecError::AppFailed(format!("{} adapter rejected {}", action.application, action.verb))) }
    }
}

fn parse_trace(text: &str) -> Result<Trace, ExecError> {
    let mut values = BTreeMap::new();
    let mut actions = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
        let (key, value) = line.split_once('=').ok_or_else(|| ExecError::AppDenied(format!("invalid trace line `{line}`")))?;
        if key == "action" {
            let fields: Vec<&str> = value.splitn(4, '|').collect();
            if fields.len() != 4 { return Err(ExecError::AppDenied("action requires application|verb|target|value".into())); }
            actions.push(Action { application: fields[0].into(), verb: fields[1].into(), target: fields[2].into(), value: fields[3].into() });
        } else {
            values.insert(key, value);
        }
    }
    if actions.is_empty() || actions.len() > 256 {
        return Err(ExecError::AppDenied("trace requires between 1 and 256 actions".into()));
    }
    if actions.iter().any(|action| {
        action.application.len() > 128 || action.verb.len() > 128 || action.target.len() > 4096 || action.value.len() > 65_536
    }) {
        return Err(ExecError::AppDenied("trace action field exceeds its bound".into()));
    }
    let trace = Trace {
        run_id: required(&values, "run_id")?.into(),
        family: required(&values, "family")?.into(),
        success: required(&values, "success")? == "true",
        duration_ms: required(&values, "duration_ms")?.parse().map_err(|_| ExecError::AppDenied("invalid duration".into()))?,
        interventions: required(&values, "interventions")?.parse().map_err(|_| ExecError::AppDenied("invalid interventions".into()))?,
        actions,
    };
    if trace.run_id.len() > 256 || trace.family.len() > 256 || trace.duration_ms > 86_400_000 || trace.interventions > 10_000 {
        return Err(ExecError::AppDenied("trace metadata exceeds its bound".into()));
    }
    Ok(trace)
}

fn required<'a>(values: &BTreeMap<&'a str, &'a str>, key: &str) -> Result<&'a str, ExecError> {
    values.get(key).copied().ok_or_else(|| ExecError::AppDenied(format!("trace requires `{key}`")))
}

fn confined(work: &Path, relative: &Path) -> Result<PathBuf, ExecError> {
    if relative.is_absolute() || relative.components().any(|part| matches!(part, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return Err(ExecError::AppDenied(format!("unsafe trace path `{}`", relative.display())));
    }
    let path = work.join(relative);
    if path.symlink_metadata().is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(ExecError::AppDenied("trace symlinks are not permitted".into()));
    }
    Ok(path)
}
