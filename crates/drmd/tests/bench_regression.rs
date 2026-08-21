//! Regression test: `drmd bench`'s planning metrics on the frozen
//! 99-episode workload must match the deterministic values documented in
//! every historical project's `PEER_PROTOCOL.md` (originally produced by
//! the C++ reference engine, independently reproduced by the Rust
//! `drm_live_odc_peer_bundle` prototype). Planning metrics depend only on
//! the exact sequence of (task, capability-sequence, ancestral-flag)
//! tuples and the planner algorithm, not on executed payload content, so
//! this cross-language port is expected to match exactly -- any drift here
//! means either the workload or the planner algorithm changed.

// `bench` and `workload` are private modules of the `drmd` binary crate, so
// this integration test drives it through the same path a user would: the
// `bench` module writes files, and we read the printed/serialized results.
// We re-declare a thin path to the binary's `run` by including it via
// `include!` is unnecessary -- instead we just shell out to the built
// binary for a true end-to-end check.

use std::process::Command;

#[test]
fn bench_reproduces_documented_frozen_values() {
    let out_dir = std::env::temp_dir().join(format!("drmd-bench-regression-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out_dir);

    let exe = env!("CARGO_BIN_EXE_drmd");
    let output = Command::new(exe)
        .arg("bench")
        .arg("--out")
        .arg(&out_dir)
        .output()
        .expect("failed to run drmd bench");
    assert!(
        output.status.success(),
        "drmd bench exited with failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("episodes=99"), "stdout: {stdout}");
    assert!(stdout.contains("success=99"), "stdout: {stdout}");
    assert!(stdout.contains("semantic=214"), "stdout: {stdout}");
    assert!(stdout.contains("derived=11"), "stdout: {stdout}");
    assert!(stdout.contains("recoveries=4"), "stdout: {stdout}");
    assert!(stdout.contains("repairs=4"), "stdout: {stdout}");
    assert!(stdout.contains("struct=1797"), "stdout: {stdout}");
    assert!(stdout.contains("OBSERVE=141"), "stdout: {stdout}");
    assert!(stdout.contains("DERIVE=390"), "stdout: {stdout}");
    assert!(stdout.contains("COMMIT=230"), "stdout: {stdout}");

    let summary = std::fs::read_to_string(out_dir.join("summary.json")).expect("summary.json must be written");
    assert!(summary.contains("\"uniform_vocabulary\": true"));

    let _ = std::fs::remove_dir_all(&out_dir);
}
