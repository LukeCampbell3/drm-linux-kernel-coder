use std::collections::HashSet;
use std::path::{Path, PathBuf};

use drm_exec::{AppConfig, WatchLearner};

fn root(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("drm-watch-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(path.join("traces")).unwrap();
    path
}

fn trace(path: &Path, run: usize, actions: &[&str], duration: u64, interventions: usize) -> PathBuf {
    let relative = PathBuf::from(format!("traces/run-{run}.trace"));
    let mut value =
        format!("run_id=run-{run}\nfamily=research_to_notes\nsuccess=true\nduration_ms={duration}\ninterventions={interventions}\n");
    for action in actions {
        value.push_str(&format!("action={action}\n"));
    }
    std::fs::write(path.join(&relative), value).unwrap();
    relative
}

fn adapter(path: &Path, application: &str, log: &Path) {
    let adapters = path.join("adapters");
    std::fs::create_dir_all(&adapters).unwrap();
    let executable = adapters.join(application);
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\nprintf '%s|%s|%s|%s\\n' '{application}' \"$1\" \"$2\" \"$3\" >> '{}'\n",
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn watches_successful_work_before_certifying_and_reusing_it() {
    let work = root("learn");
    let mut learner = WatchLearner::new(work.join("state")).unwrap();
    let slow = [
        "browser|navigate|search|query",
        "browser|click|result-1|",
        "browser|extract|article|",
        "notes|open|research|",
        "notes|write|research|summary",
    ];
    let efficient = [
        "browser|navigate|direct-url|",
        "browser|extract|article|",
        "notes|write|research|summary",
    ];
    let first = trace(&work, 1, &slow, 1200, 2);
    learner.observe_file(&work, &first).unwrap();
    for run in 2..=4 {
        let observed = trace(&work, run, &efficient, 600 - run as u64 * 20, 0);
        learner.observe_file(&work, &observed).unwrap();
        assert_eq!(learner.is_certified("research_to_notes"), run == 4);
    }

    let log = work.join("actions.log");
    adapter(&work, "browser", &log);
    adapter(&work, "notes", &log);
    let apps = AppConfig {
        adapter_dir: work.join("adapters"),
        allowed_applications: HashSet::from(["browser".into(), "notes".into()]),
        allow_risky: false,
    };
    let (actions, learned_ms) = learner.execute_certified("research_to_notes", &apps).unwrap();
    assert_eq!(actions, 3);
    assert!(learned_ms < 600);
    assert_eq!(std::fs::read_to_string(log).unwrap().lines().count(), 3);
    assert_eq!(learner.metrics.watched_tasks, 4);
    assert!(learner.metrics.shadow_evaluations >= 4);
    let metrics = std::fs::read_to_string(work.join("state/longitudinal_metrics.csv")).unwrap();
    assert_eq!(metrics.lines().count(), 5);
    let _ = std::fs::remove_dir_all(work);
}
