use std::process::Command;

#[test]
fn observe_first_suite_benchmark_certifies_only_supported_workflows() {
    let out = std::env::temp_dir().join(format!("drmd-suite-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let result = Command::new(env!("CARGO_BIN_EXE_drmd"))
        .args(["suite-bench", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains("families=2 observations=8 certified=2 actions=11->6 duration_ms=2300->1250 interventions=4"),
        "{stdout}"
    );
    let metrics = std::fs::read_to_string(out.join("state/longitudinal_metrics.csv")).unwrap();
    assert_eq!(metrics.lines().count(), 9);
    let summary = std::fs::read_to_string(out.join("suite_summary.md")).unwrap();
    assert!(summary.contains("| Actions per suite cycle | 11 | 6 | 45.5% |"));
    assert!(summary.contains("No uncertified workflow executed against a live application."));
    let _ = std::fs::remove_dir_all(out);
}
