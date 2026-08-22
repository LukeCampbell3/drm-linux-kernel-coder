use drm_core::Episode;
use drm_exec::{make_fixtures, LiveExecutor};

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("drm-exec-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

#[test]
fn full_capability_pipeline_produces_real_committed_output() {
    let work = tmp_dir("pipeline");
    make_fixtures(&work, 4).unwrap();
    let mut ex = LiveExecutor::start(work.clone()).unwrap();

    let ep = Episode {
        idx: 1,
        ctx: drm_core::ExecutionContext::simple("test-app", "t"),
        phase: "test".into(),
        ops: vec![
            "fs.read".into(),
            "transform.extract".into(),
            "transform.summarize".into(),
            "fs.write".into(),
            "notify.send".into(),
        ],
        source: "inputs/report_0.csv".into(),
        output: "outputs/t.txt".into(),
        url_path: "/news_0.html".into(),
        ancestral: false,
    };
    ex.execute(&ep).expect("pipeline should succeed");

    let out = std::fs::read_to_string(work.join("outputs/t.txt")).unwrap();
    assert!(out.starts_with("words="));
    let log = std::fs::read_to_string(work.join("notifications.log")).unwrap();
    assert!(!log.trim().is_empty());
    assert_eq!(ex.commits, 2); // fs.write + notify.send

    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn process_run_spawns_real_child_and_hashes_the_file() {
    let work = tmp_dir("process-run");
    make_fixtures(&work, 1).unwrap();
    let mut ex = LiveExecutor::start(work.clone()).unwrap();
    let ep = Episode {
        idx: 1,
        ctx: drm_core::ExecutionContext::simple("test-app", "hash"),
        phase: "test".into(),
        ops: vec!["process.run".into(), "transform.summarize".into(), "fs.write".into()],
        source: "inputs/report_0.csv".into(),
        output: "outputs/hash.txt".into(),
        url_path: "/".into(),
        ancestral: false,
    };
    ex.execute(&ep).expect("process.run should succeed");
    assert_eq!(ex.process_spawns, 1);
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn http_and_ipc_capabilities_round_trip_over_real_sockets() {
    let work = tmp_dir("sockets");
    make_fixtures(&work, 1).unwrap();
    let mut ex = LiveExecutor::start(work.clone()).unwrap();
    let ep = Episode {
        idx: 1,
        ctx: drm_core::ExecutionContext::simple("test-app", "net"),
        phase: "test".into(),
        ops: vec![
            "http.request".into(),
            "transform.extract".into(),
            "ipc.request".into(),
            "transform.summarize".into(),
            "fs.write".into(),
        ],
        source: "inputs/report_0.csv".into(),
        output: "outputs/net.txt".into(),
        url_path: "/news_3.html".into(),
        ancestral: false,
    };
    ex.execute(&ep).expect("networked pipeline should succeed");
    assert_eq!(ex.tcp_requests, 1);
    assert_eq!(ex.ipc_requests, 1);
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn fs_write_without_prior_data_is_rejected_by_verification() {
    let work = tmp_dir("verify-fail");
    make_fixtures(&work, 1).unwrap();
    let mut ex = LiveExecutor::start(work.clone()).unwrap();
    // fs.write with an empty `data` buffer (no prior capability populated it)
    // must fail output verification rather than silently commit an empty file.
    let ep = Episode {
        idx: 1,
        ctx: drm_core::ExecutionContext::simple("test-app", "empty"),
        phase: "test".into(),
        ops: vec!["fs.write".into()],
        source: "inputs/report_0.csv".into(),
        output: "outputs/empty.txt".into(),
        url_path: "/".into(),
        ancestral: false,
    };
    let result = ex.execute(&ep);
    assert!(result.is_err());
    let _ = std::fs::remove_dir_all(&work);
}
