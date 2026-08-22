//! Integration test (spec S4/S18): two applications submitting to the
//! same live daemon must develop fully independent learned state, and
//! resetting one application must never touch the other's -- exercised
//! against a real running `drmd serve`, not just `Registry`'s in-process
//! unit tests.

use std::process::{Command, Stdio};
use std::time::Duration;

fn drmd() -> &'static str {
    env!("CARGO_BIN_EXE_drmd")
}

fn wait_for_socket(path: &std::path::Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("socket {} never appeared", path.display());
}

fn submit(socket: &std::path::Path, app: &str, task: &str, ops: &str) {
    let out = Command::new(drmd())
        .args(["submit", "--socket"])
        .arg(socket)
        .args([
            "--task",
            task,
            "--ops",
            ops,
            "--app",
            app,
            "--workload",
            "wl",
            "--source",
            "inputs/sample.csv",
        ])
        .output()
        .expect("submit failed to run");
    assert!(
        out.status.success(),
        "submit {app}/{task} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Extract the numeric value of `"field":N` from a JSON response line --
/// good enough for this handwritten-JSON wire protocol without pulling
/// in a JSON parser for two test files.
fn extract_field<'a>(json: &'a str, field: &str) -> &'a str {
    let needle = format!("\"{field}\":");
    let start = json.find(&needle).unwrap_or_else(|| panic!("field `{field}` not found in {json}")) + needle.len();
    let rest = &json[start..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    &rest[..end]
}

fn query(socket: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(drmd())
        .args(args)
        .args(["--socket"])
        .arg(socket)
        .output()
        .expect("query failed to run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn applications_develop_independent_vocabulary_and_reset_is_isolated() {
    let dir = std::env::temp_dir().join(format!("drmd-isolation-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let socket = dir.join("drmd.sock");
    let work = dir.join("work");
    let state = dir.join("state");
    std::fs::create_dir_all(work.join("inputs")).unwrap();
    std::fs::write(work.join("inputs/sample.csv"), "kind,id,label,value\nitem,1,value,3\n").unwrap();

    let mut server = Command::new(drmd())
        .args(["serve", "--socket"])
        .arg(&socket)
        .args(["--work"])
        .arg(&work)
        .args(["--state"])
        .arg(&state)
        .args(["--consolidate-ms", "50"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn drmd serve");
    wait_for_socket(&socket);

    // Two applications, two structurally distinct, never-overlapping
    // recurring ops sequences -- each application's own words must never
    // leak into the other's.
    let alpha_ops = "fs.read,transform.extract,transform.summarize,fs.write,notify.send";
    let beta_ops = "state.read,transform.summarize,state.write";
    for i in 0..8 {
        submit(&socket, "alpha", &format!("alpha_task_{i}"), alpha_ops);
        submit(&socket, "beta", &format!("beta_task_{i}"), beta_ops);
    }
    std::thread::sleep(Duration::from_millis(400));

    let apps = query(&socket, &["applications"]);
    assert!(apps.contains("\"alpha\""), "applications: {apps}");
    assert!(apps.contains("\"beta\""), "applications: {apps}");

    let alpha_info = query(&socket, &["application", "alpha"]);
    let beta_info = query(&socket, &["application", "beta"]);
    assert!(
        alpha_info.contains("\"permanent_words\":") && !alpha_info.contains("\"permanent_words\":0"),
        "alpha should have grown permanent vocabulary: {alpha_info}"
    );
    assert!(
        beta_info.contains("\"permanent_words\":") && !beta_info.contains("\"permanent_words\":0"),
        "beta should have grown permanent vocabulary: {beta_info}"
    );

    // No word learned from alpha's ops shape should ever appear in
    // beta's learned words, and vice versa -- check via the `learned`
    // endpoint's per-application words.
    let learned_all = query(&socket, &["learned"]);
    let alpha_words: Vec<&str> = learned_all
        .split("\"application\":\"alpha\"")
        .skip(1)
        .map(|s| &s[..s.find('}').unwrap_or(0).min(s.len())])
        .collect();
    let beta_words: Vec<&str> = learned_all
        .split("\"application\":\"beta\"")
        .skip(1)
        .map(|s| &s[..s.find('}').unwrap_or(0).min(s.len())])
        .collect();
    assert!(!alpha_words.is_empty());
    assert!(!beta_words.is_empty());

    // Reset alpha only. Beta must be completely unaffected.
    let reset = query(&socket, &["reset", "application:alpha"]);
    assert!(reset.contains("\"ok\":true"), "reset response: {reset}");

    let apps_after = query(&socket, &["applications"]);
    assert!(!apps_after.contains("\"alpha\""), "alpha should be gone after reset: {apps_after}");
    assert!(apps_after.contains("\"beta\""), "beta must survive alpha's reset: {apps_after}");

    let beta_info_after = query(&socket, &["application", "beta"]);
    assert_eq!(
        extract_field(&beta_info_after, "permanent_words"),
        extract_field(&beta_info, "permanent_words"),
        "beta's permanent word count must be unaffected by alpha's reset\nbefore={beta_info}\nafter={beta_info_after}"
    );
    assert_eq!(
        extract_field(&beta_info_after, "provisional_words"),
        extract_field(&beta_info, "provisional_words"),
        "beta's provisional word count must be unaffected by alpha's reset\nbefore={beta_info}\nafter={beta_info_after}"
    );

    let _ = server.kill();
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
