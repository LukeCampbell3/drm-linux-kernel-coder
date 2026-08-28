use std::path::PathBuf;
use std::process::Command;

use drm_exec::CodeConfig;

fn repo(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("drm-code-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    assert!(Command::new("git").args(["init", "-q"]).current_dir(&root).status().unwrap().success());
    assert!(Command::new("git").args(["add", "."]).current_dir(&root).status().unwrap().success());
    assert!(Command::new("git")
        .args(["-c", "user.name=DRM Test", "-c", "user.email=drm@example.invalid", "commit", "-qm", "fixture"])
        .current_dir(&root)
        .status()
        .unwrap()
        .success());
    root
}

fn config(root: PathBuf, verifier: &str) -> CodeConfig {
    CodeConfig {
        root,
        allowed_paths: vec![PathBuf::from("src")],
        max_patch_bytes: 4096,
        allow_delete: false,
        verify_program: PathBuf::from(verifier),
        verify_args: Vec::new(),
    }
}

const PATCH: &[u8] = b"diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn value() -> u8 { 1 }\n+pub fn value() -> u8 { 2 }\n";

#[test]
fn verified_patch_is_committed_to_the_worktree() {
    let root = repo("commit");
    config(root.clone(), "/bin/true").apply(PATCH).unwrap();
    assert!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap().contains("{ 2 }"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failed_verification_rolls_the_patch_back() {
    let root = repo("rollback");
    let error = config(root.clone(), "/bin/false").apply(PATCH).unwrap_err();
    assert!(error.to_string().contains("patch rolled back"));
    assert!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap().contains("{ 1 }"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn paths_outside_the_allowlist_are_rejected() {
    let root = repo("deny");
    let patch = b"diff --git a/README.md b/README.md\n--- /dev/null\n+++ b/README.md\n@@ -0,0 +1 @@\n+unsafe\n";
    assert!(config(root.clone(), "/bin/true").apply(patch).unwrap_err().to_string().contains("outside"));
    let _ = std::fs::remove_dir_all(root);
}
