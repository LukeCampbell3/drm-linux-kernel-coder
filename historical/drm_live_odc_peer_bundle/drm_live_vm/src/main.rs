use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

const ROOT: [&str; 3] = ["OBSERVE", "DERIVE", "COMMIT"];
type Seq = Vec<String>;

fn root_expansion(cap: &str) -> &'static [&'static str] {
    match cap {
        "fs.read" | "state.read" => &["OBSERVE"],
        "http.get" => &["DERIVE", "OBSERVE"],
        "process.run" => &["DERIVE", "COMMIT", "OBSERVE"],
        "transform.extract" | "transform.summarize" => &["DERIVE"],
        "fs.write" | "state.write" | "notify.send" => &["DERIVE", "COMMIT"],
        _ => &[],
    }
}

fn known_capability(cap: &str) -> bool {
    !root_expansion(cap).is_empty()
}

#[derive(Clone, Debug)]
struct Episode {
    idx: usize,
    task: String,
    phase: String,
    seq: Seq,
    source: String,
    output: String,
    url_path: String,
    ancestral: bool,
}

#[derive(Clone, Debug, Default)]
struct PlanMetrics {
    semantic: usize,
    recovery: usize,
    local_repair: usize,
    structural_change: usize,
    derived: usize,
    active: usize,
    structure_bytes: usize,
    avg_depth: f64,
    max_depth: usize,
    uniform: bool,
}

#[derive(Clone, Debug, Default)]
struct Vocabulary {
    derived: BTreeMap<String, Seq>,
    counter: usize,
}

impl Vocabulary {
    fn expand_symbol_inner(&self, sym: &str, stack: &mut HashSet<String>) -> Result<Seq, String> {
        if known_capability(sym) {
            return Ok(vec![sym.to_string()]);
        }
        if !stack.insert(sym.to_string()) {
            return Err(format!("cycle:{sym}"));
        }
        let def = self
            .derived
            .get(sym)
            .ok_or_else(|| format!("unknown:{sym}"))?;
        let mut out = Vec::new();
        for part in def {
            out.extend(self.expand_symbol_inner(part, stack)?);
        }
        stack.remove(sym);
        Ok(out)
    }

    fn expand_symbol(&self, sym: &str) -> Result<Seq, String> {
        self.expand_symbol_inner(sym, &mut HashSet::new())
    }

    fn expand_root(&self, sym: &str) -> Result<Seq, String> {
        let mut out = Vec::new();
        for cap in self.expand_symbol(sym)? {
            for root in root_expansion(&cap) {
                out.push((*root).to_string());
            }
        }
        Ok(out)
    }

    fn expansions(&self) -> Vec<(String, Seq)> {
        let mut out = self
            .derived
            .keys()
            .filter_map(|name| self.expand_symbol(name).ok().map(|s| (name.clone(), s)))
            .collect::<Vec<_>>();
        out.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        out
    }

    fn compress_with(&self, seq: &[String], mut extra: Vec<(String, Seq)>) -> Seq {
        extra.extend(self.expansions());
        extra.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < seq.len() {
            let mut hit: Option<(String, usize)> = None;
            for (name, ex) in &extra {
                if ex.len() >= 2 && i + ex.len() <= seq.len() && seq[i..i + ex.len()] == ex[..] {
                    hit = Some((name.clone(), ex.len()));
                    break;
                }
            }
            if let Some((name, n)) = hit {
                out.push(name);
                i += n;
            } else {
                out.push(seq[i].clone());
                i += 1;
            }
        }
        out
    }

    fn compress(&self, seq: &[String]) -> Seq {
        self.compress_with(seq, Vec::new())
    }

    fn depth_inner(&self, sym: &str, stack: &mut HashSet<String>) -> Result<usize, String> {
        if known_capability(sym) {
            return Ok(0);
        }
        if !stack.insert(sym.to_string()) {
            return Err(format!("cycle:{sym}"));
        }
        let def = self
            .derived
            .get(sym)
            .ok_or_else(|| format!("unknown:{sym}"))?;
        let mut max_child = 0usize;
        for part in def {
            max_child = max_child.max(self.depth_inner(part, stack)?);
        }
        stack.remove(sym);
        Ok(1 + max_child)
    }

    fn depth(&self, sym: &str) -> Result<usize, String> {
        self.depth_inner(sym, &mut HashSet::new())
    }

    fn audit(&self) -> bool {
        self.derived.keys().all(|name| {
            self.expand_root(name)
                .map(|roots| !roots.is_empty() && roots.iter().all(|r| ROOT.contains(&r.as_str())))
                .unwrap_or(false)
        })
    }
}

#[derive(Debug)]
struct DrmPlanner {
    vocab: Vocabulary,
    active_cap: usize,
    mdl_threshold: isize,
    active: HashMap<String, Seq>,
    lru: VecDeque<String>,
    history: HashMap<String, Seq>,
    history_version: HashMap<String, usize>,
    subseq_users: HashMap<Seq, HashSet<String>>,
    version: usize,
}

impl DrmPlanner {
    fn new(active_cap: usize, mdl_threshold: isize) -> Self {
        Self {
            vocab: Vocabulary::default(),
            active_cap,
            mdl_threshold,
            active: HashMap::new(),
            lru: VecDeque::new(),
            history: HashMap::new(),
            history_version: HashMap::new(),
            subseq_users: HashMap::new(),
            version: 0,
        }
    }

    fn touch(&mut self, task: &str, seq: &[String]) {
        self.active.insert(task.to_string(), seq.to_vec());
        self.lru.retain(|x| x != task);
        self.lru.push_back(task.to_string());
        while self.lru.len() > self.active_cap {
            if let Some(old) = self.lru.pop_front() {
                self.active.remove(&old);
            }
        }
    }

    fn note_subseqs(&mut self, task: &str, seq: &[String]) {
        let max_len = 5usize.min(seq.len());
        for len in 2..=max_len {
            for start in 0..=seq.len() - len {
                self.subseq_users
                    .entry(seq[start..start + len].to_vec())
                    .or_default()
                    .insert(task.to_string());
            }
        }
    }

    fn corpus_cost(&self, extra: Vec<(String, Seq)>) -> usize {
        self.history
            .values()
            .map(|s| self.vocab.compress_with(s, extra.clone()).len())
            .sum()
    }

    fn maybe_grow(&mut self) -> usize {
        let baseline = self.corpus_cost(Vec::new());
        let existing = self
            .vocab
            .derived
            .keys()
            .filter_map(|k| self.vocab.expand_symbol(k).ok())
            .collect::<HashSet<_>>();
        let candidates = self
            .subseq_users
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect::<Vec<_>>();
        let mut best: Option<((isize, usize, usize), Seq)> = None;
        for (cand, users) in candidates {
            if users < 2 || existing.contains(&cand) {
                continue;
            }
            let definition = self.vocab.compress(&cand);
            if definition.len() <= 1 {
                continue;
            }
            let new_cost = self.corpus_cost(vec![("__new__".to_string(), cand.clone())]);
            let gain = baseline as isize - new_cost as isize - (definition.len() as isize + 1);
            if gain < self.mdl_threshold {
                continue;
            }
            let key = (gain, users, cand.len());
            if best.as_ref().map(|x| key > x.0).unwrap_or(true) {
                best = Some((key, definition));
            }
        }
        if let Some((_key, definition)) = best {
            self.vocab.counter += 1;
            self.vocab
                .derived
                .insert(format!("d{:03}", self.vocab.counter), definition);
            1
        } else {
            0
        }
    }

    fn diff_middle(old: &[String], new: &[String]) -> Seq {
        let mut prefix = 0usize;
        while prefix < old.len().min(new.len()) && old[prefix] == new[prefix] {
            prefix += 1;
        }
        let mut suffix = 0usize;
        while suffix < (old.len() - prefix).min(new.len() - prefix)
            && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
        {
            suffix += 1;
        }
        let end = if suffix == 0 { new.len() } else { new.len() - suffix };
        new[prefix..end].to_vec()
    }

    fn structure_bytes(&self) -> usize {
        let mut n = ROOT.iter().map(|s| s.len()).sum::<usize>();
        for (k, v) in &self.vocab.derived {
            n += k.len() + v.iter().map(|s| s.len()).sum::<usize>() + v.len();
        }
        for (k, v) in &self.active {
            n += k.len() + v.iter().map(|s| s.len()).sum::<usize>() + v.len();
        }
        for (k, v) in &self.history_version {
            n += k.len() + v.to_string().len() + 2;
        }
        n
    }

    fn plan(&mut self, ep: &Episode) -> PlanMetrics {
        let mut m = PlanMetrics::default();
        if let Some(old) = self.active.get(&ep.task).cloned() {
            if old == ep.seq {
                m.semantic = 1;
            } else {
                let delta = Self::diff_middle(&old, &ep.seq);
                m.semantic = 1usize.max(self.vocab.compress(&delta).len());
                m.local_repair = 1;
                m.structural_change += 1;
            }
        } else if let Some(old) = self.history.get(&ep.task).cloned() {
            if ep.ancestral {
                m.recovery = 1;
                m.semantic = 1usize.max(self.vocab.compress(&ep.seq).len());
                m.structural_change += 1;
            } else if old != ep.seq {
                let delta = Self::diff_middle(&old, &ep.seq);
                m.semantic = 1usize.max(self.vocab.compress(&delta).len());
                m.local_repair = 1;
                m.structural_change += 1;
            } else {
                m.semantic = 1;
            }
        } else {
            m.semantic = 1usize.max(self.vocab.compress(&ep.seq).len());
            m.structural_change += 1;
        }

        self.version += 1;
        self.history.insert(ep.task.clone(), ep.seq.clone());
        self.history_version.insert(ep.task.clone(), self.version);
        self.note_subseqs(&ep.task, &ep.seq);
        m.structural_change += self.maybe_grow();
        self.touch(&ep.task, &ep.seq);

        m.derived = self.vocab.derived.len();
        m.active = self.active.len();
        m.uniform = self.vocab.audit();
        let depths = self
            .vocab
            .derived
            .keys()
            .filter_map(|k| self.vocab.depth(k).ok())
            .collect::<Vec<_>>();
        if !depths.is_empty() {
            m.avg_depth = depths.iter().sum::<usize>() as f64 / depths.len() as f64;
            m.max_depth = *depths.iter().max().unwrap_or(&0);
        }
        m.structure_bytes = self.structure_bytes();
        m
    }
}

struct FixtureServer {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FixtureServer {
    fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buf = [0u8; 2048];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let path = req.split_whitespace().nth(1).unwrap_or("/news_0.html");
                        let idx = path
                            .trim_matches('/')
                            .trim_start_matches("news_")
                            .trim_end_matches(".html")
                            .parse::<usize>()
                            .unwrap_or(0);
                        let mut body = format!("<html><body><h1>News {idx}</h1><p>");
                        for j in 1..35 {
                            body.push_str(&format!("Story{idx}-{j} DRM local systems scheduling repeated task optimization Linux news "));
                        }
                        body.push_str("</p></body></html>");
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(), body
                        );
                        let _ = stream.write_all(resp.as_bytes());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            port,
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[derive(Debug)]
struct LiveExecutor {
    work: PathBuf,
    port: u16,
    state_runs: usize,
    commits: usize,
    root_counts: HashMap<String, usize>,
}

impl LiveExecutor {
    fn new(work: PathBuf, port: u16) -> Self {
        Self {
            work,
            port,
            state_runs: 0,
            commits: 0,
            root_counts: HashMap::new(),
        }
    }

    fn note_roots(&mut self, cap: &str) {
        for r in root_expansion(cap) {
            *self.root_counts.entry((*r).to_string()).or_default() += 1;
        }
    }

    fn http_get(&self, path: &str) -> Result<String, String> {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).map_err(|e| e.to_string())?;
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut resp = String::new();
        stream.read_to_string(&mut resp).map_err(|e| e.to_string())?;
        Ok(resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
    }

    fn execute(&mut self, ep: &Episode) -> Result<(), String> {
        let mut data = String::new();
        for cap in &ep.seq {
            self.note_roots(cap);
            match cap.as_str() {
                "fs.read" => {
                    data = fs::read_to_string(self.work.join(&ep.source)).map_err(|e| e.to_string())?;
                }
                "state.read" => {
                    let p = self.work.join("state.txt");
                    data = fs::read_to_string(p).unwrap_or_else(|_| format!("runs={}", self.state_runs));
                }
                "http.get" => {
                    data = self.http_get(&ep.url_path)?;
                }
                "process.run" => {
                    let p = self.work.join(&ep.source);
                    let out = Command::new("sha256sum")
                        .arg(&p)
                        .output()
                        .map_err(|e| e.to_string())?;
                    if !out.status.success() {
                        return Err("sha256sum failed".to_string());
                    }
                    data = String::from_utf8_lossy(&out.stdout).trim().to_string();
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
                    let words = data.split_whitespace().collect::<Vec<_>>();
                    let head = words.iter().take(12).copied().collect::<Vec<_>>().join(" ");
                    data = format!("words={} head={}", words.len(), head);
                }
                "fs.write" => {
                    let out = self.work.join(&ep.output);
                    if let Some(parent) = out.parent() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    let tmp = out.with_extension("candidate");
                    fs::write(&tmp, &data).map_err(|e| e.to_string())?;
                    fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
                    self.commits += 1;
                }
                "state.write" => {
                    self.state_runs += 1;
                    let tmp = self.work.join("state.candidate");
                    let out = self.work.join("state.txt");
                    fs::write(&tmp, format!("runs={} last={}", self.state_runs, data.chars().take(120).collect::<String>()))
                        .map_err(|e| e.to_string())?;
                    fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
                    self.commits += 1;
                }
                "notify.send" => {
                    let p = self.work.join("notifications.log");
                    let mut f = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(p)
                        .map_err(|e| e.to_string())?;
                    writeln!(f, "{}", data.replace('\n', " ")).map_err(|e| e.to_string())?;
                    self.commits += 1;
                }
                other => return Err(format!("unknown capability:{other}")),
            }
        }
        if ep.seq.iter().any(|x| x == "fs.write") {
            let p = self.work.join(&ep.output);
            if !p.exists() || p.metadata().map_err(|e| e.to_string())?.len() == 0 {
                return Err("output verification failed".to_string());
            }
        }
        Ok(())
    }
}

fn make_fixtures(work: &Path) -> std::io::Result<()> {
    fs::create_dir_all(work.join("inputs"))?;
    fs::create_dir_all(work.join("outputs"))?;
    for i in 0..12usize {
        let mut s = String::from("kind,id,label,value\n");
        for j in 1..45usize {
            s.push_str(&format!("item,{j},value,{}\n", i * j + 3));
        }
        fs::write(work.join("inputs").join(format!("report_{i}.csv")), s)?;
    }
    Ok(())
}

fn seq(xs: &[&str]) -> Seq {
    xs.iter().map(|x| (*x).to_string()).collect()
}

fn add_episode(
    out: &mut Vec<Episode>,
    idx: &mut usize,
    task: String,
    phase: &str,
    s: Seq,
    source: String,
    output: String,
    url_path: String,
    ancestral: bool,
) {
    *idx += 1;
    out.push(Episode {
        idx: *idx,
        task,
        phase: phase.to_string(),
        seq: s,
        source,
        output,
        url_path,
        ancestral,
    });
}

fn make_workload() -> Vec<Episode> {
    let file = seq(&["fs.read", "transform.summarize", "fs.write", "notify.send"]);
    let hash = seq(&["process.run", "transform.summarize", "fs.write", "notify.send"]);
    let http = seq(&["http.get", "transform.extract", "transform.summarize", "fs.write", "notify.send"]);
    let state = seq(&["state.read", "transform.summarize", "state.write", "notify.send"]);
    let combos = vec![
        seq(&["fs.read", "transform.extract", "transform.summarize", "fs.write", "notify.send"]),
        seq(&["http.get", "transform.extract", "transform.summarize", "state.write", "notify.send"]),
        seq(&["process.run", "transform.extract", "transform.summarize", "fs.write"]),
        seq(&["state.read", "transform.extract", "transform.summarize", "fs.write", "notify.send"]),
        seq(&["fs.read", "transform.summarize", "state.write", "notify.send"]),
    ];
    let mut out = Vec::new();
    let mut idx = 0usize;
    for r in 0..3usize {
        add_episode(&mut out, &mut idx, "daily_file".into(), "warmup", file.clone(), format!("inputs/report_{r}.csv"), "outputs/daily_file.txt".into(), "/news_0.html".into(), false);
        add_episode(&mut out, &mut idx, "daily_hash".into(), "warmup", hash.clone(), format!("inputs/report_{}.csv", r + 1), "outputs/daily_hash.txt".into(), "/news_0.html".into(), false);
        add_episode(&mut out, &mut idx, "daily_http".into(), "warmup", http.clone(), "inputs/report_0.csv".into(), "outputs/daily_http.txt".into(), format!("/news_{}.html", r % 3), false);
        add_episode(&mut out, &mut idx, "daily_state".into(), "warmup", state.clone(), "inputs/report_0.csv".into(), "outputs/daily_state.txt".into(), "/news_0.html".into(), false);
    }
    for i in 0..25usize {
        add_episode(&mut out, &mut idx, format!("novel_{i:02}"), "novel", combos[i % combos.len()].clone(), format!("inputs/report_{}.csv", i % 12), format!("outputs/novel_{i:02}.txt"), format!("/news_{}.html", i % 8), false);
    }
    let snapshot = out.clone();
    for i in 0..12usize {
        let task = format!("novel_{i:02}");
        let ep = snapshot.iter().find(|e| e.task == task).unwrap().clone();
        add_episode(&mut out, &mut idx, ep.task, "repeat", ep.seq, ep.source, ep.output, ep.url_path, false);
    }
    add_episode(&mut out, &mut idx, "daily_file".into(), "drift", seq(&["fs.read", "transform.extract", "transform.summarize", "fs.write", "notify.send"]), "inputs/report_9.csv".into(), "outputs/daily_file.txt".into(), "/news_0.html".into(), false);
    add_episode(&mut out, &mut idx, "daily_http".into(), "drift", seq(&["http.get", "transform.extract", "transform.summarize", "state.write", "notify.send"]), "inputs/report_0.csv".into(), "outputs/daily_http.txt".into(), "/news_7.html".into(), false);
    add_episode(&mut out, &mut idx, "daily_hash".into(), "drift", seq(&["process.run", "transform.summarize", "state.write", "notify.send"]), "inputs/report_10.csv".into(), "outputs/daily_hash.txt".into(), "/news_0.html".into(), false);
    for i in 0..7usize {
        add_episode(&mut out, &mut idx, format!("tail_{i}"), "evict", combos[i % combos.len()].clone(), format!("inputs/report_{}.csv", (i + 2) % 12), format!("outputs/tail_{i}.txt"), format!("/news_{}.html", i % 8), false);
    }
    for task in ["daily_http", "daily_file", "daily_hash"] {
        let ep = snapshot.iter().find(|e| e.task == task).unwrap().clone();
        add_episode(&mut out, &mut idx, ep.task.clone(), "ancestral", ep.seq.clone(), ep.source.clone(), ep.output.clone(), ep.url_path.clone(), true);
        add_episode(&mut out, &mut idx, ep.task, "post_recovery", ep.seq, ep.source, ep.output, ep.url_path, false);
    }
    out
}

fn read_rss_kb() -> usize {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| {
                l.split_whitespace().nth(1).and_then(|x| x.parse::<usize>().ok())
            })
        })
        .unwrap_or(0)
}

fn write_csv_line(f: &mut fs::File, fields: &[String]) -> std::io::Result<()> {
    let escaped = fields
        .iter()
        .map(|x| {
            if x.contains(',') || x.contains('"') || x.contains('\n') {
                format!("\"{}\"", x.replace('"', "\"\""))
            } else {
                x.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    writeln!(f, "{escaped}")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let outdir = std::env::var("DRM_OUT").unwrap_or_else(|_| "/results".to_string());
    let outdir = PathBuf::from(outdir);
    fs::create_dir_all(&outdir)?;
    let work = outdir.join("workspace");
    let _ = fs::remove_dir_all(&work);
    make_fixtures(&work)?;
    let server = FixtureServer::start()?;
    let mut executor = LiveExecutor::new(work, server.port);
    let mut drm = DrmPlanner::new(7, 3);
    let episodes = make_workload();

    let mut trace = fs::File::create(outdir.join("rust_live_trace.csv"))?;
    writeln!(trace, "episode,task,phase,success,wall_ms,rss_kb,semantic,recovery,local_repair,structural_change,derived,active,structure_bytes,avg_depth,max_depth,uniform")?;

    let global_start = Instant::now();
    let mut success = 0usize;
    let mut semantic_total = 0usize;
    let mut recovery_total = 0usize;
    let mut repair_total = 0usize;
    let mut peak_rss = 0usize;
    for ep in &episodes {
        let t0 = Instant::now();
        let pm = drm.plan(ep);
        let result = executor.execute(ep);
        let wall = t0.elapsed().as_secs_f64() * 1000.0;
        let rss = read_rss_kb();
        peak_rss = peak_rss.max(rss);
        if result.is_ok() { success += 1; }
        semantic_total += pm.semantic;
        recovery_total += pm.recovery;
        repair_total += pm.local_repair;
        write_csv_line(&mut trace, &[
            ep.idx.to_string(), ep.task.clone(), ep.phase.clone(), (result.is_ok() as u8).to_string(),
            format!("{wall:.3}"), rss.to_string(), pm.semantic.to_string(), pm.recovery.to_string(),
            pm.local_repair.to_string(), pm.structural_change.to_string(), pm.derived.to_string(),
            pm.active.to_string(), pm.structure_bytes.to_string(), format!("{:.3}", pm.avg_depth),
            pm.max_depth.to_string(), (pm.uniform as u8).to_string(),
        ])?;
    }

    let mut audit = fs::File::create(outdir.join("rust_vocabulary_audit.csv"))?;
    writeln!(audit, "name,definition,capability_expansion,root_expansion,depth,uniform")?;
    for (name, def) in &drm.vocab.derived {
        let caps = drm.vocab.expand_symbol(name).unwrap_or_default();
        let roots = drm.vocab.expand_root(name).unwrap_or_default();
        let uniform = roots.iter().all(|x| ROOT.contains(&x.as_str()));
        write_csv_line(&mut audit, &[
            name.clone(), def.join(" > "), caps.join(" > "), roots.join(" > "),
            drm.vocab.depth(name).unwrap_or(0).to_string(), (uniform as u8).to_string(),
        ])?;
    }

    let wall_ms = global_start.elapsed().as_secs_f64() * 1000.0;
    let summary = format!(
        "{{\n  \"episodes\": {},\n  \"success_rate\": {:.6},\n  \"semantic_total\": {},\n  \"semantic_mean\": {:.6},\n  \"derived_final\": {},\n  \"structure_bytes_final\": {},\n  \"uniform_vocabulary\": {},\n  \"recoveries\": {},\n  \"local_repairs\": {},\n  \"commits\": {},\n  \"peak_rss_kb\": {},\n  \"wall_ms\": {:.3},\n  \"root_vocabulary\": [\"OBSERVE\", \"DERIVE\", \"COMMIT\"]\n}}\n",
        episodes.len(), success as f64 / episodes.len() as f64, semantic_total,
        semantic_total as f64 / episodes.len() as f64, drm.vocab.derived.len(), drm.structure_bytes(),
        drm.vocab.audit(), recovery_total, repair_total, executor.commits, peak_rss, wall_ms
    );
    fs::write(outdir.join("rust_summary.json"), &summary)?;
    print!("{summary}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_vocabulary_is_frozen_to_odc() {
        assert_eq!(ROOT, ["OBSERVE", "DERIVE", "COMMIT"]);
        for cap in [
            "fs.read", "state.read", "http.get", "process.run", "transform.extract",
            "transform.summarize", "fs.write", "state.write", "notify.send",
        ] {
            assert!(root_expansion(cap).iter().all(|x| ROOT.contains(x)));
        }
    }

    #[test]
    fn derived_vocabulary_reduces_to_root() {
        let mut v = Vocabulary::default();
        v.derived.insert("d001".into(), seq(&["transform.summarize", "fs.write"]));
        v.derived.insert("d002".into(), vec!["fs.read".into(), "d001".into()]);
        assert!(v.audit());
        assert_eq!(
            v.expand_root("d002").unwrap(),
            seq(&["OBSERVE", "DERIVE", "DERIVE", "COMMIT"])
        );
    }

    #[test]
    fn ancestral_recovery_is_one_shot_after_forward_integration() {
        let mut p = DrmPlanner::new(1, 3);
        let ep = Episode {
            idx: 1,
            task: "old".into(),
            phase: "warmup".into(),
            seq: seq(&["fs.read", "transform.summarize", "fs.write"]),
            source: "x".into(), output: "y".into(), url_path: "/".into(), ancestral: false,
        };
        let _ = p.plan(&ep);
        let other = Episode { task: "other".into(), ..ep.clone() };
        let _ = p.plan(&other);
        let recovered = Episode { ancestral: true, ..ep.clone() };
        let m1 = p.plan(&recovered);
        assert_eq!(m1.recovery, 1);
        let m2 = p.plan(&ep);
        assert_eq!(m2.recovery, 0);
        assert_eq!(m2.semantic, 1);
    }
}
