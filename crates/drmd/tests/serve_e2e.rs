//! End-to-end test of `drmd serve` against real `drmd submit`/`drmd status`
//! client invocations over a real Unix socket, using the actual built
//! binary for both sides.

use std::io::Read;
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

#[test]
fn serve_accepts_submit_and_status_over_real_socket() {
    let dir = std::env::temp_dir().join(format!("drmd-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("drmd.sock");
    let work = dir.join("work");

    let mut server = Command::new(drmd())
        .arg("serve")
        .arg("--socket")
        .arg(&socket)
        .arg("--work")
        .arg(&work)
        .arg("--consolidate-ms")
        .arg("50")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn drmd serve");

    wait_for_socket(&socket);

    // Seed a fixture the submitted episode's fs.read capability can read.
    std::fs::create_dir_all(work.join("inputs")).unwrap();
    std::fs::write(work.join("inputs/sample.csv"), "kind,id,label,value\nitem,1,value,3\n").unwrap();

    let submit = Command::new(drmd())
        .args(["submit", "--socket"])
        .arg(&socket)
        .args([
            "--task",
            "e2e_task",
            "--ops",
            "fs.read,transform.summarize,fs.write,notify.send",
            "--source",
            "inputs/sample.csv",
        ])
        .output()
        .expect("submit failed to run");
    assert!(submit.status.success(), "{}", String::from_utf8_lossy(&submit.stderr));
    let submit_out = String::from_utf8_lossy(&submit.stdout);
    assert!(submit_out.contains("\"ok\":true"), "submit response: {submit_out}");
    assert!(submit_out.contains("\"task\":\"e2e_task\""), "submit response: {submit_out}");

    assert!(
        work.join("outputs/e2e_task.txt").exists(),
        "submitted episode should have committed a real output file"
    );

    let status = Command::new(drmd())
        .args(["status", "--socket"])
        .arg(&socket)
        .output()
        .expect("status failed to run");
    assert!(status.status.success());
    let status_out = String::from_utf8_lossy(&status.stdout);
    assert!(status_out.contains("\"episodes_planned\":1"), "status response: {status_out}");

    // A malformed request must get a clean JSON error, not a dropped
    // connection or a crashed server.
    let bad = Command::new(drmd())
        .args(["submit", "--socket"])
        .arg(&socket)
        .args(["--task", "no_ops", "--ops", ""])
        .output()
        .unwrap();
    let bad_out = String::from_utf8_lossy(&bad.stdout);
    let bad_err = String::from_utf8_lossy(&bad.stderr);
    assert!(
        bad_out.contains("\"ok\":false") || bad_err.contains("submit failed"),
        "stdout={bad_out} stderr={bad_err}"
    );

    let _ = server.kill();
    let mut stderr = String::new();
    if let Some(mut s) = server.stderr.take() {
        let _ = s.read_to_string(&mut stderr);
    }
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
