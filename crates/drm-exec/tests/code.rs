use std::path::PathBuf;

use drm_exec::evolve_task;

fn workspace(name: &str, source: &str, cases: &[(&str, &str, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("drm-evolve-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(root.join("tasks/program.py"), source).unwrap();
    let mut goal = String::from("source=tasks/program.py\nmax_candidates=256\ntimeout_ms=1000\n");
    for (case_name, input, expected) in cases {
        goal.push_str(&format!("case={case_name}|{}|{}\n", input.replace('\n', "\\n"), expected.replace('\n', "\\n")));
    }
    std::fs::write(root.join("goal.drm"), goal).unwrap();
    root
}

#[test]
fn repairs_boundary_behavior_from_executable_goal() {
    let source = "value = int(input())\nprint('high' if value > 10 else 'low')\n";
    let root = workspace("boundary", source, &[("below", "9\n", "low"), ("boundary", "10\n", "high"), ("above", "11\n", "high")]);
    let report = evolve_task(&root, std::path::Path::new("goal.drm")).unwrap();
    assert_eq!((report.initial_passed, report.final_passed, report.total_cases), (2, 3, 3));
    assert_eq!(report.mutations_committed, 1);
    assert!(std::fs::read_to_string(root.join("tasks/program.py")).unwrap().contains(">="));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn repairs_numeric_policy_and_quantifies_search() {
    let source = "attempts = int(input())\nprint(min(attempts, 2))\n";
    let root = workspace("retry", source, &[("one", "1\n", "1"), ("three", "3\n", "3"), ("five", "5\n", "3")]);
    let report = evolve_task(&root, std::path::Path::new("goal.drm")).unwrap();
    assert_eq!((report.initial_passed, report.final_passed), (1, 3));
    assert!(report.candidates_evaluated > 0);
    assert_eq!(report.mutations_committed, 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rejects_workspace_escape_without_git() {
    let root = workspace("escape", "print(1)\n", &[("one", "", "1")]);
    std::fs::write(root.join("goal.drm"), "source=../outside.py\ncase=x||1\n").unwrap();
    assert!(evolve_task(&root, std::path::Path::new("goal.drm")).unwrap_err().to_string().contains("unsafe task path"));
    let _ = std::fs::remove_dir_all(root);
}
