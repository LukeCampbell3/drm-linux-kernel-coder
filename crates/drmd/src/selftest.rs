//! `drmd selftest`: a fast, no-I/O invariant check suitable for a container
//! healthcheck or a pre-deploy smoke test -- doesn't spin up sockets or
//! touch the filesystem, just exercises the planner in memory.

use drm_core::{is_root, DrmPlanner, Episode, ExecutionContext, Vocabulary, CAPABILITIES, ROOT};

pub fn run() -> bool {
    if ROOT != ["OBSERVE", "DERIVE", "COMMIT"] {
        return false;
    }
    for cap in CAPABILITIES {
        if drm_core::root_expansion(cap).iter().any(|r| !is_root(r)) {
            return false;
        }
    }

    let mut v = Vocabulary::new();
    v.derived
        .insert("d001".into(), vec!["transform.summarize".into(), "fs.write".into()]);
    v.derived.insert("d002".into(), vec!["fs.read".into(), "d001".into()]);
    if !v.audit() {
        return false;
    }

    let mut p = DrmPlanner::new(1, 3);
    let ep = Episode {
        idx: 1,
        ctx: ExecutionContext::simple("selftest", "old"),
        phase: "x".into(),
        ops: vec!["fs.read".into(), "transform.summarize".into(), "fs.write".into()],
        source: "x".into(),
        output: "y".into(),
        url_path: "/".into(),
        ancestral: false,
    };
    p.plan(&ep);
    let other = Episode {
        ctx: ExecutionContext::simple("selftest", "other"),
        ..ep.clone()
    };
    p.plan(&other);
    let recovered = Episode {
        ancestral: true,
        ..ep.clone()
    };
    if p.plan(&recovered).recovery != 1 {
        return false;
    }
    if p.plan(&ep).recovery != 0 {
        return false;
    }
    true
}
