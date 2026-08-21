//! The frozen 99-episode reference workload, ported byte-for-byte (in
//! capability-sequence terms) from the original C++ `workload()` in
//! `historical/drm_bytecode_peer/drm_bytecode_peer/src/base_main.cpp` and
//! replicated across five other historical projects as "the frozen
//! 99-episode workload" regression baseline.
//!
//! Planning metrics (semantic cost, recoveries, derived vocabulary size,
//! structure bytes, root-token counts) depend only on this exact sequence
//! of (task, capability-sequence, ancestral-flag) tuples and on the planner
//! algorithm -- not on executed payload content -- so this port reproduces
//! the documented deterministic values exactly. See `tests/bench_regression.rs`.

use drm_core::{Episode, Seq};

fn seq(xs: &[&str]) -> Seq {
    xs.iter().map(|x| x.to_string()).collect()
}

struct Builder {
    episodes: Vec<Episode>,
    idx: usize,
}

impl Builder {
    // This mirrors the original C++ `add()` helper's parameter list
    // one-for-one to keep the port trivially auditable against the
    // historical source; splitting it into a struct would obscure that.
    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, task: &str, phase: &str, ops: Seq, source: &str, output: Option<&str>, url: &str, ancestral: bool) {
        self.idx += 1;
        let output = output.map(|s| s.to_string()).unwrap_or_else(|| format!("outputs/{task}.txt"));
        self.episodes.push(Episode {
            idx: self.idx,
            task: task.to_string(),
            phase: phase.to_string(),
            ops,
            source: source.to_string(),
            output,
            url_path: url.to_string(),
            ancestral,
        });
    }
}

/// Reconstruct the frozen 99-episode workload.
pub fn classic_workload() -> Vec<Episode> {
    let mut b = Builder {
        episodes: Vec::new(),
        idx: 0,
    };

    let file = seq(&["fs.read", "transform.summarize", "fs.write", "notify.send"]);
    let hash = seq(&["process.run", "transform.summarize", "fs.write", "notify.send"]);
    let http = seq(&[
        "http.request",
        "transform.extract",
        "transform.summarize",
        "fs.write",
        "notify.send",
    ]);
    let state = seq(&["state.read", "transform.summarize", "state.write", "notify.send"]);
    let ipc = seq(&["fs.read", "transform.summarize", "ipc.request", "fs.write"]);
    let proc = seq(&["proc.observe", "transform.extract", "transform.summarize", "fs.write"]);
    let timer = seq(&["timer.observe", "state.read", "transform.summarize", "state.write"]);

    // Warm-up: 3 rounds of the 7 daily routines (21 episodes).
    for r in 0..3usize {
        b.add(
            "daily_file",
            "warmup",
            file.clone(),
            &format!("inputs/report_{r}.csv"),
            None,
            "/news_0.html",
            false,
        );
        b.add(
            "daily_hash",
            "warmup",
            hash.clone(),
            &format!("inputs/report_{}.csv", r + 1),
            None,
            "/news_0.html",
            false,
        );
        b.add(
            "daily_http",
            "warmup",
            http.clone(),
            "inputs/report_0.csv",
            None,
            &format!("/news_{r}.html"),
            false,
        );
        b.add(
            "daily_state",
            "warmup",
            state.clone(),
            "inputs/report_0.csv",
            None,
            "/news_0.html",
            false,
        );
        b.add(
            "daily_ipc",
            "warmup",
            ipc.clone(),
            &format!("inputs/report_{}.csv", r + 2),
            None,
            "/news_0.html",
            false,
        );
        b.add(
            "daily_proc",
            "warmup",
            proc.clone(),
            "inputs/report_0.csv",
            None,
            "/news_0.html",
            false,
        );
        b.add(
            "daily_timer",
            "warmup",
            timer.clone(),
            "inputs/report_0.csv",
            None,
            "/news_0.html",
            false,
        );
    }

    let combos: Vec<Seq> = vec![
        seq(&["fs.read", "transform.extract", "transform.summarize", "fs.write", "notify.send"]),
        seq(&[
            "http.request",
            "transform.extract",
            "transform.summarize",
            "state.write",
            "notify.send",
        ]),
        seq(&["process.run", "transform.extract", "transform.summarize", "fs.write"]),
        seq(&["state.read", "transform.extract", "transform.summarize", "fs.write", "notify.send"]),
        seq(&["fs.read", "transform.summarize", "ipc.request", "state.write", "notify.send"]),
        seq(&[
            "proc.observe",
            "transform.extract",
            "transform.summarize",
            "ipc.request",
            "fs.write",
        ]),
        seq(&["timer.observe", "state.read", "transform.summarize", "fs.write", "notify.send"]),
        seq(&[
            "http.request",
            "transform.extract",
            "ipc.request",
            "transform.summarize",
            "fs.write",
        ]),
    ];

    // 40 novel compositional tasks (61 episodes so far).
    for n in 0..40usize {
        let task = format!("novel_{n:02}");
        b.add(
            &task,
            "novel",
            combos[n % combos.len()].clone(),
            &format!("inputs/report_{}.csv", n % 16),
            None,
            &format!("/news_{}.html", n % 8),
            false,
        );
    }

    // Snapshot used both for the repeat block below and the ancestral block
    // further down -- matches the C++ original's single `snap` variable
    // that stays in scope for the rest of the function.
    let snapshot = b.episodes.clone();

    // 16 exact repeats of a subset of the novel tasks (77 episodes so far).
    for n in 0..16usize {
        let task = format!("novel_{n:02}");
        let e = snapshot.iter().find(|e| e.task == task).unwrap().clone();
        b.add(&e.task, "repeat", e.ops.clone(), &e.source, Some(&e.output), &e.url_path, false);
    }

    // Structural drift on 4 daily routines (81 episodes so far).
    b.add(
        "daily_file",
        "drift",
        seq(&["fs.read", "transform.extract", "transform.summarize", "fs.write", "notify.send"]),
        "inputs/report_13.csv",
        None,
        "/news_0.html",
        false,
    );
    b.add(
        "daily_http",
        "drift",
        seq(&[
            "http.request",
            "transform.extract",
            "transform.summarize",
            "state.write",
            "notify.send",
        ]),
        "inputs/report_0.csv",
        None,
        "/news_7.html",
        false,
    );
    b.add(
        "daily_hash",
        "drift",
        seq(&["process.run", "transform.summarize", "ipc.request", "state.write", "notify.send"]),
        "inputs/report_14.csv",
        None,
        "/news_0.html",
        false,
    );
    b.add(
        "daily_ipc",
        "drift",
        seq(&["fs.read", "transform.extract", "transform.summarize", "ipc.request", "fs.write"]),
        "inputs/report_15.csv",
        None,
        "/news_0.html",
        false,
    );

    // LRU eviction pressure: 10 never-repeated tail tasks (91 episodes so far).
    for n in 0..10usize {
        let task = format!("tail_{n}");
        b.add(
            &task,
            "evict",
            combos[n % combos.len()].clone(),
            &format!("inputs/report_{}.csv", (n + 3) % 16),
            None,
            &format!("/news_{}.html", n % 8),
            false,
        );
    }

    // Forced ancestral recovery + one-shot post-recovery repeat, for 4 tasks
    // that have by now aged out of the active LRU set (99 episodes total).
    for task in ["daily_http", "daily_file", "daily_hash", "daily_ipc"] {
        let e = snapshot.iter().find(|e| e.task == task).unwrap().clone();
        b.add(&e.task, "ancestral", e.ops.clone(), &e.source, Some(&e.output), &e.url_path, true);
        b.add(
            &e.task,
            "post_recovery",
            e.ops.clone(),
            &e.source,
            Some(&e.output),
            &e.url_path,
            false,
        );
    }

    b.episodes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_has_exactly_99_episodes() {
        assert_eq!(classic_workload().len(), 99);
    }
}
