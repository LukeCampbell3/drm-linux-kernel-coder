//! [`LiveExecutor`] actually performs each planned capability against real
//! Linux primitives: durable atomic filesystem writes, `/proc` observation,
//! a real timer wait, a real loopback TCP round-trip, a real `AF_UNIX`
//! round-trip, and a real child-process spawn. "COMMIT", in this codebase,
//! means an atomic write-then-rename, an appended state file, or an
//! appended notification log entry -- a real durable side effect on the
//! local machine.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use drm_core::{root_expansion, Episode};

use crate::code::CodeConfig;
use crate::servers::{TcpFixtureServer, UnixFixtureServer};
use crate::specialize::SpecializationSet;
use crate::web::WebConfig;

#[derive(Debug)]
pub enum ExecError {
    Io(std::io::Error),
    UnknownCapability(String),
    ChildFailed(String),
    VerificationFailed(String),
    WebDenied(String),
    WebBridge(String),
    CodeDenied(String),
    CodePatch(String),
    CodeVerification(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Io(e) => write!(f, "io error: {e}"),
            ExecError::UnknownCapability(c) => write!(f, "unknown capability: {c}"),
            ExecError::ChildFailed(c) => write!(f, "child process failed: {c}"),
            ExecError::VerificationFailed(p) => write!(f, "output verification failed for {p}"),
            ExecError::WebDenied(reason) => write!(f, "web access denied: {reason}"),
            ExecError::WebBridge(reason) => write!(f, "Selenium bridge error: {reason}"),
            ExecError::CodeDenied(reason) => write!(f, "code change denied: {reason}"),
            ExecError::CodePatch(reason) => write!(f, "code patch failed: {reason}"),
            ExecError::CodeVerification(reason) => write!(f, "code verification failed: {reason}"),
        }
    }
}

impl std::error::Error for ExecError {}

impl From<std::io::Error> for ExecError {
    fn from(e: std::io::Error) -> Self {
        ExecError::Io(e)
    }
}

pub struct LiveExecutor {
    pub work: PathBuf,
    tcp: TcpFixtureServer,
    unix: UnixFixtureServer,
    state_runs: usize,
    pub commits: usize,
    pub process_spawns: usize,
    pub tcp_requests: usize,
    pub ipc_requests: usize,
    pub timer_events: usize,
    pub web_requests: usize,
    pub code_changes: usize,
    pub root_counts: HashMap<String, usize>,
    /// Opt-in bridge to `drm-opt`'s specialization lifecycle (see
    /// `crate::specialize` module docs). `None` (the default from
    /// [`LiveExecutor::start`]) reproduces this executor's exact
    /// pre-specialization behavior -- every existing baseline/regression
    /// guarantee holds unchanged with no `SpecializationSet` attached.
    pub specializations: Option<SpecializationSet>,
    /// How many `fs.read`s this executor has served from a verified
    /// read-avoidance specialization instead of touching disk.
    pub reads_avoided: usize,
    /// How many transform chains this executor has served from a
    /// verified fusion specialization's memo table instead of
    /// recomputing.
    pub transforms_memoized: usize,
    /// The specialization id (if any) that served each capability in the
    /// most recent [`LiveExecutor::execute`] call, in the order they were
    /// used. Cleared at the start of every call.
    pub optimizations_used: Vec<String>,
    /// Selenium access is disabled unless configured explicitly.
    pub web: Option<WebConfig>,
    /// Source mutation is disabled unless a code root and allowlist are configured.
    pub code: Option<CodeConfig>,
}

impl LiveExecutor {
    /// Start a new executor rooted at `work`, bringing up the loopback
    /// fixture servers that back `http.request`/`ipc.request`. `work` is
    /// created if it does not already exist.
    pub fn start(work: PathBuf) -> Result<Self, ExecError> {
        fs::create_dir_all(&work)?;
        let tcp = TcpFixtureServer::start()?;
        // The AF_UNIX fixture socket deliberately does not live under `work`:
        // `sockaddr_un.sun_path` is capped at ~108 bytes on Linux, and a
        // caller-supplied work directory (a deep systemd StateDirectory, a
        // temp test path, ...) can easily exceed that. A short, uniquely
        // named path under the system temp directory has no such ceiling.
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let sock_path = std::env::temp_dir().join(format!("drmd-{}-{n}.sock", std::process::id()));
        let unix = UnixFixtureServer::start(&sock_path)?;
        Ok(Self {
            work,
            tcp,
            unix,
            state_runs: 0,
            commits: 0,
            process_spawns: 0,
            tcp_requests: 0,
            ipc_requests: 0,
            timer_events: 0,
            web_requests: 0,
            code_changes: 0,
            root_counts: HashMap::new(),
            specializations: None,
            reads_avoided: 0,
            transforms_memoized: 0,
            optimizations_used: Vec::new(),
            web: WebConfig::from_env(),
            code: CodeConfig::from_env(),
        })
    }

    /// Attach a [`SpecializationSet`], enabling real read-avoidance and
    /// transform-fusion specialization for subsequent [`Self::execute`]
    /// calls. Strictly opt-in: without calling this, `execute` behaves
    /// exactly as it always has.
    pub fn with_specialization(mut self, specializations: SpecializationSet) -> Self {
        self.specializations = Some(specializations);
        self
    }

    pub fn with_web(mut self, config: WebConfig) -> Self {
        self.web = Some(config);
        self
    }

    pub fn with_code(mut self, config: CodeConfig) -> Self {
        self.code = Some(config);
        self
    }

    fn note_roots(&mut self, cap: &str) {
        for r in root_expansion(cap) {
            *self.root_counts.entry((*r).to_string()).or_default() += 1;
        }
    }

    /// Execute every capability in `ep.ops` in order, threading a single
    /// `data` buffer between them (each capability reads what the previous
    /// one produced, matching a small linear pipeline). Returns an error on
    /// the first capability that fails; capabilities already committed
    /// before that point are not rolled back (each commit is independently
    /// atomic, matching the durable-step semantics of the underlying
    /// primitive).
    pub fn execute(&mut self, ep: &Episode) -> Result<(), ExecError> {
        self.optimizations_used.clear();
        let mut data = String::new();
        let mut idx = 0usize;
        while idx < ep.ops.len() {
            let cap = &ep.ops[idx];
            self.note_roots(cap);

            // Pure transform.* capabilities run in a maximal consecutive
            // group so a `SpecializationSet`, when attached, can treat
            // the whole chain as one fusable/memoizable unit rather than
            // one stage at a time. With no `SpecializationSet` attached
            // this produces byte-identical output to running each stage
            // individually (both paths call the same
            // `drm_opt::equivalence::apply_transform_stage` logic), so
            // baseline execution is unaffected.
            if cap.starts_with("transform.") {
                let start = idx;
                let mut end = idx;
                while end < ep.ops.len() && ep.ops[end].starts_with("transform.") {
                    end += 1;
                }
                for extra in &ep.ops[start + 1..end] {
                    self.note_roots(extra);
                }
                let stages = &ep.ops[start..end];
                let (out, used) = match self.specializations.as_mut() {
                    Some(spec) => spec
                        .run_transform_chain(&ep.ctx.application_id, stages, &data)
                        .ok_or_else(|| ExecError::UnknownCapability(stages.join(",")))?,
                    None => (
                        drm_opt::equivalence::run_stages_unfused(stages, &data)
                            .ok_or_else(|| ExecError::UnknownCapability(stages.join(",")))?,
                        None,
                    ),
                };
                if let Some(id) = used {
                    self.transforms_memoized += 1;
                    self.optimizations_used.push(id);
                }
                data = out;
                idx = end;
                continue;
            }

            match cap.as_str() {
                "fs.read" => {
                    let path = ep.source.clone();
                    let work = self.work.clone();
                    data = match self.specializations.as_mut() {
                        Some(spec) => {
                            let (content, used) = spec.read(&ep.ctx.application_id, &path, || fs::read_to_string(work.join(&path)))?;
                            if let Some(id) = used {
                                self.reads_avoided += 1;
                                self.optimizations_used.push(id);
                            }
                            content
                        }
                        None => fs::read_to_string(work.join(&path))?,
                    };
                }
                "state.read" => {
                    let p = self.work.join("state.txt");
                    data = fs::read_to_string(p).unwrap_or_else(|_| format!("runs={}", self.state_runs));
                }
                "proc.observe" => {
                    data = fs::read_to_string("/proc/self/status").unwrap_or_default();
                }
                "timer.observe" => {
                    let until = Instant::now() + Duration::from_millis(2);
                    while Instant::now() < until {
                        thread::sleep(Duration::from_micros(200));
                    }
                    data = "timer-fired".to_string();
                    self.timer_events += 1;
                }
                "http.request" => {
                    data = self.tcp.get(&ep.url_path)?;
                    self.tcp_requests += 1;
                }
                "web.selenium" => {
                    let web = self
                        .web
                        .as_ref()
                        .ok_or_else(|| ExecError::WebDenied("set DRMD_WEB_ALLOWED_HOSTS to enable Selenium".into()))?;
                    data = web.fetch(&ep.url_path, &ep.ctx.application_id)?;
                    self.web_requests += 1;
                }
                "code.patch" => {
                    let code = self.code.as_ref().ok_or_else(|| {
                        ExecError::CodeDenied("set DRMD_CODE_ROOT and DRMD_CODE_ALLOWED_PATHS to enable source changes".into())
                    })?;
                    let patch = fs::read(self.work.join(&ep.source))?;
                    code.apply(&patch)?;
                    data = format!("applied and verified {}", ep.source);
                    self.code_changes += 1;
                    self.commits += 1;
                }
                "ipc.request" => {
                    let payload = if data.is_empty() {
                        ep.task().to_string()
                    } else {
                        data.chars().take(80).collect()
                    };
                    data = self.unix.roundtrip(&payload)?;
                    self.ipc_requests += 1;
                }
                "process.run" => {
                    let p = self.work.join(&ep.source);
                    let out = Command::new("sha256sum").arg(&p).output()?;
                    if !out.status.success() {
                        return Err(ExecError::ChildFailed("sha256sum".to_string()));
                    }
                    data = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    self.process_spawns += 1;
                }
                "fs.write" => {
                    let out = self.work.join(&ep.output);
                    if let Some(parent) = out.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let tmp = out.with_extension("candidate");
                    fs::write(&tmp, &data)?;
                    fs::rename(&tmp, &out)?;
                    self.commits += 1;
                    if let Some(spec) = self.specializations.as_mut() {
                        spec.mark_written(&ep.output);
                    }
                }
                "state.write" => {
                    self.state_runs += 1;
                    let tmp = self.work.join("state.candidate");
                    let out = self.work.join("state.txt");
                    let snippet: String = data.chars().take(120).collect();
                    fs::write(&tmp, format!("runs={} last={}", self.state_runs, snippet))?;
                    fs::rename(&tmp, &out)?;
                    self.commits += 1;
                    if let Some(spec) = self.specializations.as_mut() {
                        spec.mark_written("state.txt");
                    }
                }
                "notify.send" => {
                    let p = self.work.join("notifications.log");
                    let mut f = OpenOptions::new().create(true).append(true).open(p)?;
                    let snippet: String = data.chars().take(300).collect();
                    writeln!(f, "{}", snippet.replace('\n', " "))?;
                    self.commits += 1;
                }
                other => return Err(ExecError::UnknownCapability(other.to_string())),
            }
            idx += 1;
        }
        if ep.ops.iter().any(|c| c == "fs.write") {
            let p = self.work.join(&ep.output);
            let len = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            if len == 0 {
                return Err(ExecError::VerificationFailed(ep.output.clone()));
            }
        }
        Ok(())
    }
}

/// Populate `work/inputs/report_{0..count}.csv` with small synthetic CSV
/// fixtures, matching the shape the `fs.read`/`process.run` capabilities
/// expect. Used by `bench` mode and available to callers of `serve` mode
/// that want a ready-made work directory to submit episodes against.
pub fn make_fixtures(work: &Path, count: usize) -> std::io::Result<()> {
    fs::create_dir_all(work.join("inputs"))?;
    fs::create_dir_all(work.join("outputs"))?;
    for i in 0..count {
        let mut s = String::from("kind,id,label,value\n");
        for j in 1..60usize {
            s.push_str(&format!("item,{j},value,{}\n", i * j + 3));
        }
        fs::write(work.join("inputs").join(format!("report_{i}.csv")), s)?;
    }
    Ok(())
}
