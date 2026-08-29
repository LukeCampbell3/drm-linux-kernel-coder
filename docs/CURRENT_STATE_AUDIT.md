# Productization current-state audit

Baseline commit: `e0fb1da0eff2fa720aac8342862ac580ec956169` (`master`).

## Preserved nucleus

`drm-core` is platform-independent and dependency-free. It owns the frozen `OBSERVE`, `DERIVE`, `COMMIT` roots, the 12 capability-to-root expansions, recursive vocabulary auditing, `DrmPlanner`, and `HybridPlanner`. The frozen workload is guarded by an end-to-end regression test.

| Area | Present on master | Product gap |
|---|---|---|
| `crates/drm-core` | O/D/C roots, vocabulary, planner, HybridPlanner, baselines | Versioned serialization and public planner metrics |
| `crates/drm-exec` | Linux filesystem, `/proc`, timer, fixture TCP/Unix IPC, fixed `sha256sum` process | Platform interfaces, policy, transactions, general process/network operations |
| `crates/drmd` | Unix-socket daemon, submit/status CLI, selftest, frozen benchmark | Persistent state, IPC v2, task lifecycle, approvals, audit, separate `drmctl`, Windows transport/service |
| `packaging/` | Docker, hardened systemd unit, Debian VM image | Archives, DEB/RPM, Windows ZIP/MSI, upgrades, release manifest/signing |
| `historical/` | 55 tracked research files retained unchanged | None; this directory remains an archive |
| `docs/` | Architecture and README | Product, installation, security, policy, state, recovery, API, release documentation |
| `.github/` | Ubuntu debug/release CI and Docker smoke test | Windows matrix, packages, release assets, clean-machine acceptance |

## Security and correctness observations

- The root vocabulary is explicitly frozen and audited on every plan.
- The daemon uses one global mutex. This is the compatibility baseline and must be measured before lock splitting.
- Planner learning is memory-only; restart loses vocabulary and history.
- Filesystem commits are individually atomic, but an episode has no transaction journal or rollback.
- `process.run` is hard-coded to direct `sha256sum`; it does not invoke a shell.
- Network and IPC capabilities use deterministic loopback fixtures, not real external networking.
- The Linux systemd service uses `DynamicUser`, an empty capability bounding set, and strong sandboxing.
- The v1 protocol is line-oriented and Unix-only.
- There is no Windows implementation, policy engine, approval system, desktop client, installer, update system, or release signing.

## Frozen acceptance values

The following values are release invariants unless an explicitly versioned benchmark supersedes them:

| Metric | Value |
|---|---:|
| Episodes / successes | 99 / 99 |
| Semantic total | 214 |
| Derived words | 11 |
| Recoveries / local repairs | 4 / 4 |
| Final structure bytes | 1,797 |
| Root counts | OBSERVE 141; DERIVE 390; COMMIT 230 |
| Vocabulary audit | uniform = true |

`scripts/product-baseline.sh` records the machine-dependent baseline—test output, benchmark files, release binary bytes, daemon startup, idle/active RSS, submission percentiles, and throughput—without modifying repository state.
