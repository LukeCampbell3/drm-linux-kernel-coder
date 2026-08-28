use std::path::PathBuf;

use drm_core::{Episode, ExecutionContext};
use drm_exec::{LiveExecutor, WebConfig};

fn tmp_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("drm-web-test-{name}-{}", std::process::id()))
}

fn fake_bridge(work: &std::path::Path) -> PathBuf {
    let bridge = work.join("bridge.sh");
    std::fs::create_dir_all(work).unwrap();
    std::fs::write(&bridge, "#!/bin/sh\nprintf '%s' '{\"title\":\"fixture\",\"text\":\"hello web\"}'\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bridge, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bridge
}

fn config(bridge: PathBuf) -> WebConfig {
    WebConfig {
        python: PathBuf::from("/bin/sh"),
        bridge,
        webdriver_url: None,
        allowed_hosts: vec!["example.com".into()],
        allow_private: false,
        timeout_secs: 1,
        max_output_bytes: 4096,
    }
}

#[test]
fn selenium_output_flows_into_the_normal_commit_pipeline() {
    let work = tmp_dir("success");
    let bridge = fake_bridge(&work);
    let mut executor = LiveExecutor::start(work.clone()).unwrap().with_web(config(bridge));
    let episode = Episode {
        idx: 1,
        ctx: ExecutionContext::simple("browser-app", "research"),
        phase: "test".into(),
        ops: vec!["web.selenium".into(), "fs.write".into()],
        output: "outputs/page.json".into(),
        url_path: "https://example.com/page?q=safe".into(),
        ..Episode::default()
    };
    executor.execute(&episode).unwrap();
    assert!(std::fs::read_to_string(work.join("outputs/page.json"))
        .unwrap()
        .contains("hello web"));
    assert_eq!(executor.web_requests, 1);
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn disallowed_host_never_starts_the_bridge() {
    let work = tmp_dir("denied");
    let bridge = fake_bridge(&work);
    let mut executor = LiveExecutor::start(work.clone()).unwrap().with_web(config(bridge));
    let episode = Episode {
        ctx: ExecutionContext::simple("browser-app", "research"),
        ops: vec!["web.selenium".into()],
        url_path: "https://attacker-example.com/".into(),
        ..Episode::default()
    };
    assert!(executor
        .execute(&episode)
        .unwrap_err()
        .to_string()
        .contains("not in DRMD_WEB_ALLOWED_HOSTS"));
    assert_eq!(executor.web_requests, 0);
    let _ = std::fs::remove_dir_all(work);
}
