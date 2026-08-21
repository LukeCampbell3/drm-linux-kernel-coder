use std::collections::{HashMap, HashSet, VecDeque};

const CANONICAL: &[&str] = &[
    "process.spawn", "process.wait", "fs.read", "fs.write", "fs.move",
    "http.get", "browser.navigate", "text.extract", "text.summarize",
    "state.read", "state.write", "notify.send", "schedule.trigger", "network.check",
];

type Seq = Vec<String>;

#[derive(Clone, Debug)]
struct Episode {
    task: String,
    seq: Seq,
    phase: &'static str,
    needs_ancestry: bool,
}

impl Episode {
    fn new(task: impl Into<String>, seq: &[&str], phase: &'static str) -> Self {
        Self {
            task: task.into(),
            seq: seq.iter().map(|x| (*x).to_string()).collect(),
            phase,
            needs_ancestry: false,
        }
    }

    fn ancestral(mut self) -> Self {
        self.needs_ancestry = true;
        self
    }
}

#[derive(Clone, Debug, Default)]
struct Metrics {
    semantic: usize,
    recovery: usize,
    local_repair: usize,
    structural_change: usize,
    runtime_storage: usize,
    active: usize,
    derived: usize,
    uniform: bool,
}

fn is_canonical(s: &str) -> bool {
    CANONICAL.iter().any(|x| *x == s)
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

#[derive(Clone, Debug)]
struct Derived {
    definition: Seq,
}

#[derive(Debug)]
struct Drm {
    active_cap: usize,
    max_derived: usize,
    active: HashMap<String, (Seq, Seq)>, // task -> (canonical IR, compressed rep)
    lru: VecDeque<String>,
    history: HashMap<String, Seq>,
    derived: HashMap<String, Derived>,
    subseq_users: HashMap<Seq, HashSet<String>>,
    derived_counter: usize,
    version: usize,
    uniformity_failures: usize,
}

impl Drm {
    fn new(active_cap: usize, max_derived: usize) -> Self {
        Self {
            active_cap,
            max_derived,
            active: HashMap::new(),
            lru: VecDeque::new(),
            history: HashMap::new(),
            derived: HashMap::new(),
            subseq_users: HashMap::new(),
            derived_counter: 0,
            version: 0,
            uniformity_failures: 0,
        }
    }

    fn expand_symbol_inner(&self, sym: &str, stack: &mut HashSet<String>) -> Result<Seq, String> {
        if is_canonical(sym) {
            return Ok(vec![sym.to_string()]);
        }
        if stack.contains(sym) {
            return Err(format!("derived vocabulary cycle at {sym}"));
        }
        let d = self
            .derived
            .get(sym)
            .ok_or_else(|| format!("unknown vocabulary symbol {sym}"))?;
        stack.insert(sym.to_string());
        let mut out = Vec::new();
        for part in &d.definition {
            out.extend(self.expand_symbol_inner(part, stack)?);
        }
        stack.remove(sym);
        Ok(out)
    }

    fn expand_symbol(&self, sym: &str) -> Result<Seq, String> {
        self.expand_symbol_inner(sym, &mut HashSet::new())
    }

    fn expansions(&self) -> Vec<(String, Seq)> {
        let mut out: Vec<_> = self
            .derived
            .keys()
            .filter_map(|name| self.expand_symbol(name).ok().map(|e| (name.clone(), e)))
            .collect();
        out.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        out
    }

    fn compress_with_expansions(seq: &[String], expansions: &[(String, Seq)]) -> Seq {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < seq.len() {
            let mut hit: Option<(&String, usize)> = None;
            for (name, ex) in expansions {
                let len = ex.len();
                if len >= 2 && i + len <= seq.len() && seq[i..i + len] == ex[..] {
                    hit = Some((name, len));
                    break;
                }
            }
            if let Some((name, len)) = hit {
                out.push(name.clone());
                i += len;
            } else {
                out.push(seq[i].clone());
                i += 1;
            }
        }
        out
    }

    fn compress(&self, seq: &[String]) -> Seq {
        Self::compress_with_expansions(seq, &self.expansions())
    }

    fn audit_uniformity(&self) -> bool {
        for name in self.derived.keys() {
            let Ok(expanded) = self.expand_symbol(name) else { return false; };
            if expanded.is_empty() || expanded.iter().any(|x| !is_canonical(x)) {
                return false;
            }
        }
        for (_, rep) in self.active.values() {
            for symbol in rep {
                if !is_canonical(symbol) && !self.derived.contains_key(symbol) {
                    return false;
                }
                if self.expand_symbol(symbol).is_err() {
                    return false;
                }
            }
        }
        true
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

    // Global minimum-description-length gate. A new word is legal only if:
    // 1) it is shared by >=2 task identities,
    // 2) it recursively reduces to the canonical vocabulary,
    // 3) it reduces total task-corpus description length after paying definition cost.
    fn maybe_add_derived(&mut self) -> usize {
        if self.derived.len() >= self.max_derived {
            return 0;
        }
        let existing: HashSet<Seq> = self
            .derived
            .keys()
            .filter_map(|name| self.expand_symbol(name).ok())
            .collect();
        let base_expansions = self.expansions();
        let baseline_corpus: usize = self.history.values().map(|s| self.compress(s).len()).sum();

        let mut best: Option<(isize, usize, usize, Seq, Seq)> = None;
        for (candidate, users) in &self.subseq_users {
            if users.len() < 2 || existing.contains(candidate) {
                continue;
            }
            let definition = self.compress(candidate);
            if definition.len() <= 1 {
                continue;
            }
            let mut extra = vec![("__candidate__".to_string(), candidate.clone())];
            extra.extend(base_expansions.clone());
            extra.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
            let new_corpus: usize = self
                .history
                .values()
                .map(|s| Self::compress_with_expansions(s, &extra).len())
                .sum();
            let gain = baseline_corpus as isize
                - new_corpus as isize
                - (definition.len() as isize + 1);
            if gain < 3 {
                continue;
            }
            let item = (gain, users.len(), candidate.len(), candidate.clone(), definition);
            let replace = best.as_ref().map_or(true, |b| {
                (item.0, item.1, item.2) > (b.0, b.1, b.2)
            });
            if replace {
                best = Some(item);
            }
        }

        let Some((_gain, _users, _len, _expansion, definition)) = best else {
            return 0;
        };
        if definition
            .iter()
            .any(|x| !is_canonical(x) && !self.derived.contains_key(x))
        {
            return 0;
        }
        self.derived_counter += 1;
        let name = format!("d{:02}", self.derived_counter);
        self.derived.insert(name, Derived { definition });

        let keys: Vec<String> = self.active.keys().cloned().collect();
        let mut changed = 1usize; // vocabulary insertion itself
        for key in keys {
            if let Some((canonical, old_rep)) = self.active.get(&key).cloned() {
                let new_rep = self.compress(&canonical);
                if new_rep.len() < old_rep.len() {
                    self.active.insert(key, (canonical, new_rep));
                    changed += 1;
                }
            }
        }
        changed
    }

    fn touch_active(&mut self, task: &str, seq: &[String]) -> usize {
        self.lru.retain(|x| x != task);
        self.lru.push_back(task.to_string());
        let rep = self.compress(seq);
        self.active.insert(task.to_string(), (seq.to_vec(), rep));
        let mut evictions = 0usize;
        while self.active.len() > self.active_cap {
            if let Some(victim) = self.lru.pop_front() {
                if self.active.remove(&victim).is_some() {
                    evictions += 1;
                }
            }
        }
        evictions
    }

    fn step(&mut self, ep: &Episode) -> Metrics {
        self.version += 1;
        let mut m = Metrics::default();
        let active_prev = self.active.get(&ep.task).cloned();
        let hist_prev = self.history.get(&ep.task).cloned();

        if let Some((old, _rep)) = active_prev {
            if old == ep.seq {
                m.semantic = 1;
            } else {
                let middle = diff_middle(&old, &ep.seq);
                m.semantic = 1 + self.compress(&middle).len();
                m.local_repair = 1;
                m.structural_change += 1;
            }
        } else if hist_prev.as_ref() == Some(&ep.seq) {
            if ep.needs_ancestry {
                m.semantic = 2;
                m.recovery = 1;
            } else {
                m.semantic = 1;
            }
            m.structural_change += 1;
        } else if let Some(old) = hist_prev {
            let middle = diff_middle(&old, &ep.seq);
            let repair = self.compress(&middle).len();
            m.semantic = if ep.needs_ancestry { 2 + repair } else { 1 + repair };
            m.recovery = if ep.needs_ancestry { 1 } else { 0 };
            m.local_repair = 1;
            m.structural_change += 1;
        } else {
            m.semantic = self.compress(&ep.seq).len();
            m.structural_change += 1;
        }

        self.history.insert(ep.task.clone(), ep.seq.clone());
        self.note_subseqs(&ep.task, &ep.seq);
        m.structural_change += self.touch_active(&ep.task, &ep.seq);
        m.structural_change += self.maybe_add_derived();

        m.uniform = self.audit_uniformity();
        if !m.uniform {
            self.uniformity_failures += 1;
        }
        let vocab_storage = CANONICAL.len()
            + self.derived.values().map(|d| d.definition.len()).sum::<usize>();
        let active_storage = self.active.values().map(|(_, rep)| rep.len()).sum::<usize>();
        m.runtime_storage = vocab_storage + active_storage;
        m.active = self.active.len();
        m.derived = self.derived.len();
        m
    }
}

#[derive(Default)]
struct TaskCheckpointReplay {
    cache: HashMap<String, Seq>,
}
impl TaskCheckpointReplay {
    fn step(&mut self, ep: &Episode) -> Metrics {
        let same = self.cache.get(&ep.task) == Some(&ep.seq);
        if !same {
            self.cache.insert(ep.task.clone(), ep.seq.clone());
        }
        Metrics {
            semantic: if same { 1 } else { ep.seq.len() },
            runtime_storage: self.cache.values().map(Vec::len).sum(),
            active: self.cache.len(),
            uniform: true,
            structural_change: if !same { 1 } else { 0 },
            ..Metrics::default()
        }
    }
}

#[derive(Default)]
struct FlatTemplateCache {
    templates: HashSet<Seq>,
}
impl FlatTemplateCache {
    fn step(&mut self, ep: &Episode) -> Metrics {
        let known = self.templates.contains(&ep.seq);
        if !known {
            self.templates.insert(ep.seq.clone());
        }
        Metrics {
            semantic: if known { 1 } else { ep.seq.len() },
            runtime_storage: self.templates.iter().map(Vec::len).sum(),
            active: self.templates.len(),
            uniform: true,
            structural_change: if !known { 1 } else { 0 },
            ..Metrics::default()
        }
    }
}

fn stateless(ep: &Episode) -> Metrics {
    Metrics { semantic: ep.seq.len(), uniform: true, ..Metrics::default() }
}

struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self { Self(seed.wrapping_add(1)) }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
    fn index(&mut self, n: usize) -> usize { (self.next() as usize) % n }
    fn shuffle<T>(&mut self, xs: &mut [T]) {
        for i in (1..xs.len()).rev() {
            let j = self.index(i + 1);
            xs.swap(i, j);
        }
    }
}

fn stable_workload(seed: u64) -> Vec<Episode> {
    let tasks: Vec<(&str, &[&str])> = vec![
        ("news.ai", &["process.spawn","browser.navigate","text.extract","text.summarize","notify.send"]),
        ("news.nba", &["process.spawn","browser.navigate","text.extract","text.summarize","notify.send"]),
        ("news.security", &["process.spawn","browser.navigate","text.extract","text.summarize","notify.send"]),
        ("api.weather", &["network.check","http.get","text.extract","text.summarize","notify.send"]),
        ("api.stocks", &["network.check","http.get","text.extract","text.summarize","notify.send"]),
        ("report.daily", &["fs.read","text.extract","state.write","notify.send"]),
        ("report.weekly", &["fs.read","text.extract","state.write","notify.send"]),
        ("files.archive", &["fs.read","fs.move","state.write","notify.send"]),
        ("system.health", &["process.spawn","process.wait","state.read","notify.send"]),
        ("repo.sync", &["process.spawn","process.wait","fs.write","notify.send"]),
    ];
    let mut rng = Lcg::new(seed);
    let mut out = Vec::new();
    for _ in 0..60 {
        let (name, seq) = tasks[rng.index(tasks.len())];
        out.push(Episode::new(name, seq, "warmup"));
    }
    out
}

fn long_tail_workload(seed: u64) -> Vec<Episode> {
    let sources: Vec<(&str, Vec<&str>)> = vec![
        ("web", vec!["process.spawn","browser.navigate","text.extract"]),
        ("api", vec!["network.check","http.get","text.extract"]),
        ("file", vec!["fs.read","text.extract"]),
        ("proc", vec!["process.spawn","process.wait","state.read"]),
        ("archive", vec!["fs.read","fs.move","state.read"]),
    ];
    let sinks: Vec<(&str, Vec<&str>)> = vec![
        ("summary", vec!["text.summarize","notify.send"]),
        ("persist", vec!["fs.write","state.write","notify.send"]),
        ("summary_persist", vec!["text.summarize","fs.write","state.write","notify.send"]),
        ("state", vec!["state.write","notify.send"]),
    ];
    let mut eps = Vec::new();
    let mut id = 0usize;
    for (sname, src) in &sources {
        for (kname, sink) in &sinks {
            for scheduled in [false, true] {
                for context in [false, true] {
                    id += 1;
                    let mut seq: Vec<&str> = Vec::new();
                    if scheduled { seq.push("schedule.trigger"); }
                    seq.extend(src.iter().copied());
                    if context { seq.push("state.read"); }
                    seq.extend(sink.iter().copied());
                    eps.push(Episode::new(
                        format!("longtail.{id:03}.{sname}.{kname}.{}.{}", scheduled as u8, context as u8),
                        &seq,
                        "novel_composition",
                    ));
                }
            }
        }
    }
    let mut rng = Lcg::new(seed);
    rng.shuffle(&mut eps);
    let repeats: Vec<Episode> = eps.iter().take(30).cloned().map(|mut e| {
        e.phase = "consolidated_repeat";
        e
    }).collect();
    eps.extend(repeats);
    eps
}

#[derive(Default)]
struct Summary {
    episodes: usize,
    semantic: usize,
    recoveries: usize,
    repairs: usize,
    changes: usize,
    final_storage: usize,
    final_derived: usize,
}
impl Summary {
    fn add(&mut self, m: &Metrics) {
        self.episodes += 1;
        self.semantic += m.semantic;
        self.recoveries += m.recovery;
        self.repairs += m.local_repair;
        self.changes += m.structural_change;
        self.final_storage = m.runtime_storage;
        self.final_derived = m.derived;
    }
    fn mean(&self) -> f64 { self.semantic as f64 / self.episodes as f64 }
}

fn run(label: &str, episodes: &[Episode]) {
    let mut replay = TaskCheckpointReplay::default();
    let mut flat = FlatTemplateCache::default();
    let mut drm = Drm::new(12, 16);
    let mut a = Summary::default();
    let mut b = Summary::default();
    let mut c = Summary::default();
    let mut d = Summary::default();
    for ep in episodes {
        a.add(&stateless(ep));
        b.add(&replay.step(ep));
        c.add(&flat.step(ep));
        d.add(&drm.step(ep));
    }
    println!("\n{label}");
    println!("system,mean_semantic,total_semantic,runtime_storage,derived,recoveries,repairs");
    println!("stateless,{:.3},{},{},0,0,0", a.mean(), a.semantic, a.final_storage);
    println!("checkpoint_replay,{:.3},{},{},0,0,0", b.mean(), b.semantic, b.final_storage);
    println!("flat_template,{:.3},{},{},0,0,0", c.mean(), c.semantic, c.final_storage);
    println!("drm,{:.3},{},{},{},{},{}", d.mean(), d.semantic, d.final_storage, d.final_derived, d.recoveries, d.repairs);
    println!("uniform_vocabulary={}", drm.audit_uniformity());
    println!("canonical_count={}", CANONICAL.len());
}

fn main() {
    run("stable_repetition", &stable_workload(0));
    run("long_tail_composition", &long_tail_workload(0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_vocabulary_is_fixed_and_unique() {
        let set: HashSet<_> = CANONICAL.iter().copied().collect();
        assert_eq!(set.len(), CANONICAL.len());
    }

    #[test]
    fn derived_words_reduce_to_canonical_roots() {
        let mut drm = Drm::new(12, 16);
        let a = Episode::new("a", &["process.spawn","browser.navigate","text.extract","text.summarize","notify.send"], "t");
        let b = Episode::new("b", &["process.spawn","browser.navigate","text.extract","text.summarize","notify.send"], "t");
        drm.step(&a);
        drm.step(&b);
        assert!(drm.audit_uniformity());
        for name in drm.derived.keys() {
            assert!(drm.expand_symbol(name).unwrap().iter().all(|x| is_canonical(x)));
        }
    }

    #[test]
    fn ancestral_recovery_is_forward_integrated() {
        let mut drm = Drm::new(2, 16);
        let old = Episode::new("old", &["fs.read","fs.move","network.check","state.read","notify.send"], "t");
        drm.step(&old);
        drm.step(&Episode::new("x", &["fs.read","notify.send"], "t"));
        drm.step(&Episode::new("y", &["http.get","notify.send"], "t"));
        let recovered = drm.step(&old.clone().ancestral());
        assert_eq!(recovered.recovery, 1);
        let repeated = drm.step(&old);
        assert_eq!(repeated.recovery, 0);
        assert_eq!(repeated.semantic, 1);
    }

    #[test]
    fn drm_converges_below_stateless_on_stable_workload() {
        let eps = stable_workload(3);
        let mut drm = Drm::new(12, 16);
        let stateless_cost: usize = eps.iter().map(|e| e.seq.len()).sum();
        let drm_cost: usize = eps.iter().map(|e| drm.step(e).semantic).sum();
        assert!(drm_cost < stateless_cost);
        assert!(drm.audit_uniformity());
    }
}
