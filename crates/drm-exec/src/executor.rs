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

use crate::servers::{TcpFixtureServer, UnixFixtureServer};

#[derive(Debug)]
pub enum ExecError {
    Io(std::io::Error),
    UnknownCapability(String),
    ChildFailed(String),
    VerificationFailed(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Io(e) => write!(f, "io error: {e}"),
            ExecError::UnknownCapability(c) => write!(f, "unknown capability: {c}"),
            ExecError::ChildFailed(c) => write!(f, "child process failed: {c}"),
            ExecError::VerificationFailed(p) => write!(f, "output verification failed for {p}"),
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
    pub root_counts: HashMap<String, usize>,
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
            root_counts: HashMap::new(),
        })
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
        let mut data = String::new();
        for cap in &ep.ops {
            self.note_roots(cap);
            match cap.as_str() {
                "fs.read" => {
                    data = fs::read_to_string(self.work.join(&ep.source))?;
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
                "ipc.request" => {
                    let payload = if data.is_empty() {
                        ep.task.clone()
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
                "transform.extract" => {
                    let mut clean = String::with_capacity(data.len());
                    let mut in_tag = false;
                    for c in data.chars() {
                        match c {
                            '<' => in_tag = true,
                            '>' => {
                                in_tag = false;
                                clean.push(' ');
                            }
                            _ if !in_tag => clean.push(c),
                            _ => {}
                        }
                    }
                    data = clean.split_whitespace().collect::<Vec<_>>().join(" ");
                }
                "transform.summarize" => {
                    let words: Vec<&str> = data.split_whitespace().collect();
                    let head = words.iter().take(10).copied().collect::<Vec<_>>().join(" ");
                    data = format!("words={} head={}", words.len(), head);
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
                }
                "state.write" => {
                    self.state_runs += 1;
                    let tmp = self.work.join("state.candidate");
                    let out = self.work.join("state.txt");
                    let snippet: String = data.chars().take(120).collect();
                    fs::write(&tmp, format!("runs={} last={}", self.state_runs, snippet))?;
                    fs::rename(&tmp, &out)?;
                    self.commits += 1;
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
