use std::fs;
use std::process::Command;

#[cfg(unix)]
#[test]
fn glm_is_default_and_plan_is_proposal_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("drmd-model-frontend-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let adapter = dir.join("adapter.sh");
    fs::write(
        &adapter,
        "#!/bin/sh\ntest \"$2\" = glm || exit 9\nprintf 'decision=watch\\nfamily=research_to_notes\\ncapability=task.watch\\nconfidence_milli=910\\n'\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&adapter).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&adapter, permissions).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_drmd"))
        .args(["assist", "--goal", "Research a topic and put it in notes"])
        .env("DRMD_MODEL_ADAPTER", &adapter)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("provider=glm decision=watch family=research_to_notes capability=task.watch"));
    assert!(stdout.contains("proposal_only=true certified_execution_required=true"));
    let _ = fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn unsafe_model_plan_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("drmd-model-reject-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let adapter = dir.join("adapter.sh");
    fs::write(
        &adapter,
        "#!/bin/sh\nprintf 'decision=execute\\nfamily=x\\ncapability=web.selenium\\nconfidence_milli=999\\n'\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&adapter).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&adapter, permissions).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_drmd"))
        .args(["assist", "--goal", "do it"])
        .env("DRMD_MODEL_ADAPTER", &adapter)
        .status()
        .unwrap();
    assert!(!status.success());
    let _ = fs::remove_dir_all(dir);
}
