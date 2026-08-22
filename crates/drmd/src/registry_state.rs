//! Daemon-side wiring for `drm_core::registry::Registry`: atomic snapshot
//! file I/O and a best-effort SIGTERM/SIGINT snapshot alongside the
//! existing periodic one.
//!
//! `drm-core`'s `persistence` module is pure serialize/deserialize with
//! no filesystem access of its own (see its module docs); this is the
//! one place that actually owns the daemon's state directory and writes
//! to it, matching `drm-exec::LiveExecutor`'s existing write-temp-then-
//! rename convention for every other durable commit this codebase makes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use drm_core::persistence::{from_json, to_json, to_snapshot, Snapshot};
use drm_core::registry::Registry;

pub fn state_file(state_dir: &Path) -> PathBuf {
    state_dir.join("registry.json")
}

/// Atomic write: a temp file in the same directory, then `rename`. A
/// crash mid-write can never leave a torn snapshot in place of a good
/// one -- `rename` within one filesystem is atomic, so the previous
/// snapshot (or none) is all a reader can ever observe until this
/// completes.
pub fn save(reg: &Registry, state_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let snap = to_snapshot(reg);
    let json = to_json(&snap).map_err(std::io::Error::other)?;
    let final_path = state_file(state_dir);
    let tmp_path = state_dir.join("registry.json.tmp");
    {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Load a snapshot if one is present and valid. Any problem (missing
/// file, corrupt JSON, schema-version mismatch) is reported to stderr
/// and treated as "no snapshot" -- the daemon always starts, it just
/// starts fresh. "Never silently load incompatible state" (spec §4)
/// means refusing to *use* mismatched state, not refusing to run; the
/// offending file is left on disk untouched for inspection, never
/// overwritten by this call.
pub fn load(state_dir: &Path) -> Option<Registry> {
    let path = state_file(state_dir);
    let contents = std::fs::read_to_string(&path).ok()?;
    let snap: Snapshot = match from_json(&contents) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "drmd: snapshot at {} is not valid JSON ({e}) -- starting with fresh state",
                path.display()
            );
            return None;
        }
    };
    if let Err(e) = snap.validate_version() {
        eprintln!(
            "drmd: {e} -- starting with fresh state (old snapshot left at {} for inspection)",
            path.display()
        );
        return None;
    }
    Some(drm_core::persistence::from_snapshot(snap))
}

/// Set by the SIGTERM/SIGINT handler; polled by `serve::run`'s
/// connection-accept loop so a routine `systemctl stop`/restart gets a
/// fresh snapshot on top of the periodic one, without needing a signal
/// crate for a single atomic-flag handler.
pub static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_shutdown_signal(_sig: i32) {
    // Async-signal-safe: only an atomic store, nothing else. The actual
    // snapshot-and-exit happens back on the main thread once it observes
    // the flag.
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
}

// The POSIX `signal(2)` binding, not `sigaction(2)`: `signal` has a
// trivial, stable, ABI-simple `(i32, handler) -> old_handler` shape with
// no struct layout to get right across libc versions, and a bare
// "set a flag" handler like ours doesn't need `sigaction`'s extra
// flags/mask control. This is the entire FFI surface drmd needs for
// graceful shutdown -- deliberately not worth a signal-handling crate.
extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
}

const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

pub fn install_shutdown_handler() {
    unsafe {
        signal(SIGTERM, handle_shutdown_signal);
        signal(SIGINT, handle_shutdown_signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drm_core::identity::ExecutionContext;

    fn tmp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("drmd-registry-state-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tmp_dir("roundtrip");
        let mut reg = Registry::new();
        for i in 0..6 {
            reg.plan(
                &ExecutionContext::new("h", "u", "app", "wl", format!("t{i}")),
                vec![
                    "fs.read".into(),
                    "transform.extract".into(),
                    "transform.summarize".into(),
                    "fs.write".into(),
                    "notify.send".into(),
                ],
                "x",
                false,
            );
        }
        reg.consolidate();
        let derived_before = reg.applications["app"].planner.base.vocab.derived.len();

        save(&reg, &dir).unwrap();
        let loaded = load(&dir).expect("a freshly saved snapshot must load");
        assert_eq!(loaded.applications["app"].planner.base.vocab.derived.len(), derived_before);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_snapshot_is_none_not_an_error() {
        let dir = tmp_dir("missing");
        assert!(load(&dir).is_none());
    }

    #[test]
    fn corrupt_snapshot_is_rejected_not_crashed_on() {
        let dir = tmp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(state_file(&dir), b"{ not json").unwrap();
        assert!(load(&dir).is_none());
        // The corrupt file must survive for inspection, not be silently
        // deleted or overwritten by a failed load.
        assert!(state_file(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
