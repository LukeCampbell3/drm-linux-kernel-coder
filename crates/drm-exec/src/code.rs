//! In-execution program mutation driven by executable task goals.

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::executor::ExecError;

#[derive(Clone, Debug)]
struct GoalCase { name: String, input: String, expected: String }

#[derive(Clone, Debug)]
struct MutationTask { source: PathBuf, cases: Vec<GoalCase>, max_candidates: usize, timeout_ms: u64 }

#[derive(Clone, Debug, Default)]
pub struct MutationReport {
    pub initial_passed: usize,
    pub final_passed: usize,
    pub total_cases: usize,
    pub candidates_evaluated: usize,
    pub mutations_committed: usize,
    pub elapsed_ms: u128,
}

impl MutationReport {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"initial_passed\":{},\"final_passed\":{},\"total_cases\":{},\"candidates_evaluated\":{},\"mutations_committed\":{},\"elapsed_ms\":{}}}",
            self.initial_passed, self.final_passed, self.total_cases, self.candidates_evaluated,
            self.mutations_committed, self.elapsed_ms
        )
    }
}

pub fn evolve_task(work: &Path, manifest: &Path) -> Result<MutationReport, ExecError> {
    let started = Instant::now();
    let task = parse_manifest(&fs::read_to_string(confined(work, manifest)?)?)?;
    let source_path = confined(work, &task.source)?;
    if source_path.symlink_metadata().is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(ExecError::CodeDenied("source symlinks are not permitted".into()));
    }
    let mut source = fs::read_to_string(&source_path)?;
    if source.len() > 262_144 { return Err(ExecError::CodeDenied("source exceeds 262144 bytes".into())); }
    let mut score = evaluate(&source_path, &task.cases, task.timeout_ms)?;
    let initial = score;
    let mut evaluated = 0;
    let mut committed = 0;
    let mut seen = HashSet::new();
    seen.insert(source.clone());

    while score < task.cases.len() && evaluated < task.max_candidates {
        let mut improved = false;
        for candidate in derive_candidates(&source) {
            if evaluated >= task.max_candidates || !seen.insert(candidate.clone()) { continue; }
            evaluated += 1;
            atomic_write(&source_path, candidate.as_bytes())?;
            let candidate_score = evaluate(&source_path, &task.cases, task.timeout_ms)?;
            if candidate_score > score {
                source = candidate;
                score = candidate_score;
                committed += 1;
                improved = true;
                break;
            }
            atomic_write(&source_path, source.as_bytes())?;
        }
        if !improved { break; }
    }
    atomic_write(&source_path, source.as_bytes())?;
    Ok(MutationReport {
        initial_passed: initial, final_passed: score, total_cases: task.cases.len(),
        candidates_evaluated: evaluated, mutations_committed: committed,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn parse_manifest(text: &str) -> Result<MutationTask, ExecError> {
    let mut source = None;
    let mut cases = Vec::new();
    let mut max_candidates = 256;
    let mut timeout_ms = 2_000;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let (key, value) = line.split_once('=').ok_or_else(|| ExecError::CodeDenied(format!("invalid goal line `{line}`")))?;
        match key.trim() {
            "source" => source = Some(PathBuf::from(value.trim())),
            "max_candidates" => max_candidates = value.trim().parse().map_err(|_| ExecError::CodeDenied("invalid max_candidates".into()))?,
            "timeout_ms" => timeout_ms = value.trim().parse().map_err(|_| ExecError::CodeDenied("invalid timeout_ms".into()))?,
            "case" => {
                let mut fields = value.splitn(3, '|');
                let name = fields.next().unwrap_or_default().trim().to_string();
                let input = unescape(fields.next().unwrap_or_default());
                let expected = unescape(fields.next().unwrap_or_default());
                if name.is_empty() { return Err(ExecError::CodeDenied("goal case requires name|input|expected".into())); }
                cases.push(GoalCase { name, input, expected });
            }
            other => return Err(ExecError::CodeDenied(format!("unknown goal field `{other}`"))),
        }
    }
    if cases.is_empty() || !(1..=10_000).contains(&max_candidates) || !(10..=30_000).contains(&timeout_ms) {
        return Err(ExecError::CodeDenied("goal bounds or cases are invalid".into()));
    }
    Ok(MutationTask { source: source.ok_or_else(|| ExecError::CodeDenied("goal requires source".into()))?, cases, max_candidates, timeout_ms })
}

fn unescape(value: &str) -> String { value.replace("\\n", "\n").replace("\\t", "\t") }

fn confined(work: &Path, relative: &Path) -> Result<PathBuf, ExecError> {
    if relative.is_absolute() || relative.components().any(|part| matches!(part, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return Err(ExecError::CodeDenied(format!("unsafe task path `{}`", relative.display())));
    }
    let mut cursor = work.to_path_buf();
    for component in relative.components() {
        cursor.push(component);
        if cursor.symlink_metadata().is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(ExecError::CodeDenied(format!("symlink task path `{}`", cursor.display())));
        }
    }
    Ok(cursor)
}

fn evaluate(source: &Path, cases: &[GoalCase], timeout_ms: u64) -> Result<usize, ExecError> {
    let mut passed = 0;
    for case in cases {
        let _case_name = &case.name;
        let mut child = Command::new("python3").arg("-I").arg(source)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        child.stdin.as_mut().expect("piped stdin").write_all(case.input.as_bytes())?;
        drop(child.stdin.take());
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if let Some(status) = child.try_wait()? {
                let output = child.wait_with_output()?;
                if status.success() && String::from_utf8_lossy(&output.stdout).trim() == case.expected.trim() { passed += 1; }
                break;
            }
            if Instant::now() >= deadline { child.kill()?; let _ = child.wait(); break; }
            thread::sleep(Duration::from_millis(5));
        }
    }
    Ok(passed)
}

fn derive_candidates(source: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for (from, to) in [(">=", ">"), ("<=", "<"), ("==", "!="), (">", ">="), ("<", "<="), ("!=", "=="), (" + ", " - "), (" - ", " + "), (" * ", " + "), (" + ", " * ")] {
        push_each_replacement(source, from, to, &mut candidates);
    }
    let bytes = source.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        if bytes[start].is_ascii_digit() && (start == 0 || !bytes[start - 1].is_ascii_alphanumeric()) {
            let mut end = start + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() { end += 1; }
            if end == bytes.len() || !bytes[end].is_ascii_alphanumeric() {
                let current: i64 = source[start..end].parse().unwrap_or(0);
                for value in [current - 1, current + 1, 0, 1, 2, 3, 5, 10, 100] {
                    if value >= 0 && value != current {
                        let mut candidate = source.to_string();
                        candidate.replace_range(start..end, &value.to_string());
                        candidates.push(candidate);
                    }
                }
            }
            start = end;
        } else { start += 1; }
    }
    candidates
}

fn push_each_replacement(source: &str, from: &str, to: &str, output: &mut Vec<String>) {
    for (start, _) in source.match_indices(from) {
        let mut candidate = source.to_string();
        candidate.replace_range(start..start + from.len(), to);
        output.push(candidate);
    }
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), ExecError> {
    let candidate = path.with_extension("drm-candidate");
    fs::write(&candidate, content)?;
    fs::rename(candidate, path)?;
    Ok(())
}
