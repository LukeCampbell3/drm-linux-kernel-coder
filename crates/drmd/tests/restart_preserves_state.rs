//! Integration test (spec S21): a `drmd serve` process that has grown
//! real vocabulary, sent a graceful shutdown signal (SIGTERM -- the
//! signal `registry_state::install_shutdown_handler` actually handles,
//! not the `SIGKILL` `Child::kill()` sends), and restarted against the
//! same state directory must come back up with that vocabulary intact,
//! not empty. This is the actual daemon-restart path, not just the
//! `registry_state::save`/`load` unit round-trip -- it exercises the
//! real background snapshot thread and the real signal handler together.

use std::io::Read;
use std::process::{Child, Command, Stdio};
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

fn wait_for_exit(child: &mut Child) {
    for _ in 0..100 {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("process {} never exited after SIGTERM", child.id());
}

fn submit(socket: &std::path::Path, task: &str, ops: &str) {
    let out = Command::new(drmd())
        .args(["submit", "--socket"])
        .arg(socket)
        .args([
            "--task",
            task,
            "--ops",
            ops,
            "--app",
            "restart-app",
            "--workload",
            "wl",
            "--source",
            "inputs/sample.csv",
        ])
        .output()
        .expect("submit failed to run");
    assert!(
        out.status.success(),
        "submit {task} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn learned(socket: &std::path::Path) -> String {
    let out = Command::new(drmd())
        .args(["learned", "--socket"])
        .arg(socket)
        .output()
        .expect("learned failed to run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn daemon_restart_preserves_learned_vocabulary() {
    let dir = std::env::temp_dir().join(format!("drmd-restart-{}", std::process::id()));
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

    // Same ops sequence recurring across enough distinct tasks to clear
    // drm-core's MDL admission threshold and actually grow permanent
    // vocabulary, not just provisional.
    let ops = "fs.read,transform.extract,transform.summarize,fs.write,notify.send";
    for i in 0..8 {
        submit(&socket, &format!("restart_task_{i}"), ops);
    }
    // Give the background consolidation timer (50ms tick) time to run
    // and snapshot at least once.
    std::thread::sleep(Duration::from_millis(400));

    let before = learned(&socket);
    assert!(
        before.contains("\"application\":\"restart-app\""),
        "expected learned vocabulary before restart: {before}"
    );
    let permanent_before = before.matches("\"stage\":\"Permanent\"").count();
    assert!(
        permanent_before > 0,
        "expected at least one Permanent word before restart: {before}"
    );

    // Graceful shutdown: SIGTERM, the signal registry_state actually
    // installs a handler for. `Child::kill()` sends SIGKILL, which would
    // test nothing about the graceful-shutdown snapshot path.
    let pid = server.id().to_string();
    let term = Command::new("kill").args(["-TERM", &pid]).status().expect("failed to send SIGTERM");
    assert!(term.success(), "kill -TERM must succeed");
    wait_for_exit(&mut server);
    let mut stderr = String::new();
    if let Some(mut s) = server.stderr.take() {
        let _ = s.read_to_string(&mut stderr);
    }
    assert!(
        stderr.contains("final snapshot written"),
        "expected graceful-shutdown log line, got: {stderr}"
    );

    // A Unix domain socket file is not removed automatically when its
    // listening process exits -- the first server's socket file is
    // still sitting on disk right now. `serve::run` would remove and
    // recreate it itself, but that happens *after* this test's
    // `wait_for_socket` polling could already observe the stale file and
    // return early, connecting to nothing. Remove it ourselves first so
    // "the socket exists" only becomes true once the new process has
    // actually bound it.
    let _ = std::fs::remove_file(&socket);

    // Restart against the same state directory.
    let mut server2 = Command::new(drmd())
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
        .expect("failed to respawn drmd serve");
    wait_for_socket(&socket);

    let after = learned(&socket);
    let permanent_after = after.matches("\"stage\":\"Permanent\"").count();
    assert_eq!(
        permanent_after, permanent_before,
        "restart must preserve exactly the permanent vocabulary that existed before shutdown\nbefore={before}\nafter={after}"
    );

    let _ = server2.kill();
    let _ = server2.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
