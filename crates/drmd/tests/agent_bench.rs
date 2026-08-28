use std::process::Command;

#[test]
fn agentic_mutation_benchmark_beats_static_programs() {
    let out = std::env::temp_dir().join(format!("drmd-agent-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let result = Command::new(env!("CARGO_BIN_EXE_drmd"))
        .args(["agent-bench", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("tasks=3 static=5/10 evolved=10/10"), "{stdout}");
    let metrics = std::fs::read_to_string(out.join("agentic_metrics.csv")).unwrap();
    assert_eq!(metrics.lines().count(), 4);
    let summary = std::fs::read_to_string(out.join("agentic_summary.md")).unwrap();
    assert!(summary.contains("Static baseline: 5/10 cases (50.0%)."));
    assert!(summary.contains("Developmental runtime: 10/10 cases (100.0%)."));
    let _ = std::fs::remove_dir_all(out);
}
