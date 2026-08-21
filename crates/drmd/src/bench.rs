//! `drmd bench`: runs the frozen 99-episode workload end to end (real
//! planning + real execution against a scratch work directory) and writes
//! the same report shape the historical C++/Rust benchmarks did. Doubles
//! as a deterministic regression check: with the base planner, the
//! reported `semantic_total`, `derived_final`, `recoveries`, and root
//! counts must match the values documented in every historical project's
//! `PEER_PROTOCOL.md`.

use std::fs;
use std::path::Path;
use std::time::Instant;

use drm_core::{Baseline, BaselineKind, DrmPlanner};
use drm_exec::{make_fixtures, LiveExecutor};

use crate::fmt::{csv_line, json_string_array};
use crate::workload::classic_workload;

pub struct BenchReport {
    pub episodes: usize,
    pub success: usize,
    pub semantic_total: usize,
    pub derived_final: usize,
    pub structure_bytes_final: usize,
    pub recoveries: usize,
    pub local_repairs: usize,
    pub uniform: bool,
    pub root_observe: usize,
    pub root_derive: usize,
    pub root_commit: usize,
    pub description_length_reduction: f64,
}

pub fn run(out_dir: &Path) -> std::io::Result<BenchReport> {
    fs::create_dir_all(out_dir)?;
    let work = out_dir.join("workspace");
    let _ = fs::remove_dir_all(&work);
    make_fixtures(&work, 16)?;
    let mut executor = LiveExecutor::start(work).expect("fixture servers must start");
    let mut planner = DrmPlanner::new(8, 3);
    let episodes = classic_workload();

    let mut trace = fs::File::create(out_dir.join("live_trace.csv"))?;
    use std::io::Write;
    writeln!(trace, "episode,task,phase,success,wall_ms,semantic,recovery,local_repair,structural_change,derived,active,structure_bytes,avg_depth,max_depth,uniform")?;

    let global_start = Instant::now();
    let (mut success, mut semantic_total, mut recoveries, mut repairs, mut structural_total) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for ep in &episodes {
        let t0 = Instant::now();
        let pm = planner.plan(ep);
        let ok = executor.execute(ep).is_ok();
        let wall = t0.elapsed().as_secs_f64() * 1000.0;
        success += ok as usize;
        semantic_total += pm.semantic;
        recoveries += pm.recovery;
        repairs += pm.local_repair;
        structural_total += pm.structural_change;
        writeln!(
            trace,
            "{}",
            csv_line(&[
                ep.idx.to_string(),
                ep.task.clone(),
                ep.phase.clone(),
                (ok as u8).to_string(),
                format!("{wall:.3}"),
                pm.semantic.to_string(),
                pm.recovery.to_string(),
                pm.local_repair.to_string(),
                pm.structural_change.to_string(),
                pm.derived.to_string(),
                pm.active.to_string(),
                pm.structure_bytes.to_string(),
                format!("{:.3}", pm.avg_depth),
                pm.max_depth.to_string(),
                (pm.uniform as u8).to_string(),
            ])
        )?;
    }

    let mut audit = fs::File::create(out_dir.join("vocabulary_audit.csv"))?;
    writeln!(audit, "name,definition,capability_expansion,root_expansion,depth,uniform")?;
    let (mut raw_tokens, mut compressed_tokens, mut def_tokens) = (0usize, 0usize, 0usize);
    for s in planner.history.values() {
        raw_tokens += s.len();
        compressed_tokens += planner.vocab.compress(s).len();
    }
    for (name, def) in &planner.vocab.derived {
        def_tokens += def.len();
        let caps = planner.vocab.expand_symbol(name).unwrap_or_default();
        let roots = planner.vocab.expand_root(name).unwrap_or_default();
        let uniform = roots.iter().all(|r| drm_core::is_root(r));
        writeln!(
            audit,
            "{}",
            csv_line(&[
                name.clone(),
                def.join(" > "),
                caps.join(" > "),
                roots.join(" > "),
                planner.vocab.depth(name).unwrap_or(0).to_string(),
                (uniform as u8).to_string(),
            ])
        )?;
    }

    let mut baselines = fs::File::create(out_dir.join("baseline_comparison.csv"))?;
    writeln!(
        baselines,
        "system,episodes,semantic_total,semantic_mean,recoveries,local_repairs,structural_changes,final_structure_bytes"
    )?;
    for kind in [BaselineKind::Stateless, BaselineKind::TemplateCache, BaselineKind::CheckpointReplay] {
        let mut b = Baseline::new(kind);
        let (mut s, mut r, mut p, mut c) = (0usize, 0usize, 0usize, 0usize);
        let mut last_bytes = 0usize;
        for ep in &episodes {
            let m = b.plan(ep);
            s += m.semantic;
            r += m.recovery;
            p += m.local_repair;
            c += m.structural_change;
            last_bytes = m.structure_bytes;
        }
        let name = match kind {
            BaselineKind::Stateless => "stateless",
            BaselineKind::TemplateCache => "template_cache",
            BaselineKind::CheckpointReplay => "checkpoint_replay",
        };
        writeln!(
            baselines,
            "{name},{},{s},{:.6},{r},{p},{c},{last_bytes}",
            episodes.len(),
            s as f64 / episodes.len() as f64
        )?;
    }
    writeln!(
        baselines,
        "drmd,{},{semantic_total},{:.6},{recoveries},{repairs},{structural_total},{}",
        episodes.len(),
        semantic_total as f64 / episodes.len() as f64,
        planner.structure_bytes()
    )?;

    let vocab_headers = planner.vocab.derived.len();
    let compressed_total = compressed_tokens + def_tokens + vocab_headers;
    let dl_reduction = if raw_tokens > 0 {
        1.0 - (compressed_total as f64 / raw_tokens as f64)
    } else {
        0.0
    };
    let wall_ms = global_start.elapsed().as_secs_f64() * 1000.0;
    let root_observe = *executor.root_counts.get("OBSERVE").unwrap_or(&0);
    let root_derive = *executor.root_counts.get("DERIVE").unwrap_or(&0);
    let root_commit = *executor.root_counts.get("COMMIT").unwrap_or(&0);
    let uniform = planner.vocab.audit();

    let summary = format!(
        "{{\n  \"episodes\": {},\n  \"success_rate\": {:.6},\n  \"semantic_total\": {semantic_total},\n  \"semantic_mean\": {:.6},\n  \"derived_final\": {},\n  \"structure_bytes_final\": {},\n  \"uniform_vocabulary\": {uniform},\n  \"recoveries\": {recoveries},\n  \"local_repairs\": {repairs},\n  \"commits\": {},\n  \"process_spawns\": {},\n  \"tcp_requests\": {},\n  \"ipc_requests\": {},\n  \"timer_events\": {},\n  \"benchmark_wall_ms\": {:.3},\n  \"raw_task_tokens\": {raw_tokens},\n  \"compressed_task_tokens\": {compressed_tokens},\n  \"definition_tokens\": {def_tokens},\n  \"description_length_reduction\": {:.6},\n  \"root_counts\": {{\"OBSERVE\": {root_observe}, \"DERIVE\": {root_derive}, \"COMMIT\": {root_commit}}},\n  \"root_vocabulary\": {}\n}}\n",
        episodes.len(),
        success as f64 / episodes.len() as f64,
        semantic_total as f64 / episodes.len() as f64,
        planner.vocab.derived.len(),
        planner.structure_bytes(),
        executor.commits,
        executor.process_spawns,
        executor.tcp_requests,
        executor.ipc_requests,
        executor.timer_events,
        wall_ms,
        dl_reduction,
        json_string_array(&drm_core::ROOT.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
    );
    fs::write(out_dir.join("summary.json"), &summary)?;

    Ok(BenchReport {
        episodes: episodes.len(),
        success,
        semantic_total,
        derived_final: planner.vocab.derived.len(),
        structure_bytes_final: planner.structure_bytes(),
        recoveries,
        local_repairs: repairs,
        uniform,
        root_observe,
        root_derive,
        root_commit,
        description_length_reduction: dl_reduction,
    })
}
