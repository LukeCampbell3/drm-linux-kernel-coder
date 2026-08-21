# Changelog

All notable changes to this project are documented here.
Format loosely follows [Keep a Changelog](https://keepachangelog.com/).

## [1.0.0] - 2026-08-21

First production release. Consolidates eight experimental research
prototypes (see `historical/`) into one shipped product.

### Added
- `drm-core`: the O/D/C planner engine (`Vocabulary`, `DrmPlanner`,
  `HybridPlanner` with a two-tier provisional/permanent vocabulary and
  deferred consolidation, `Baseline` comparators), ported to Rust and
  extended to the full 12-capability set.
- `drm-exec`: `LiveExecutor`, executing every capability against real
  Linux primitives (filesystem, `/proc`, loopback TCP/AF_UNIX, process
  spawn).
- `drmd`: the shipped CLI/daemon binary --
  - `drmd serve`: a long-running Unix-socket daemon that accepts real
    episode submissions -- the point at which this stopped being a
    benchmark that runs once and exits.
  - `drmd bench`: the frozen 99-episode regression workload, reproducing
    the historical projects' documented deterministic values exactly.
  - `drmd submit` / `drmd status` / `drmd selftest`.
- Packaging: multi-stage `Dockerfile`, a hardened `systemd` unit, and
  `packaging/vm-image/build-image.sh`, which builds a bootable Debian-based
  VM disk image with `drmd` installed and enabled.
- CI: build/test/lint/docker workflow on GitHub Actions.
- 25 automated tests, including a real end-to-end daemon test over a live
  Unix socket and a cross-language regression test against the values
  documented in the historical C++/Rust prototypes.

### Removed
- ~544 generated benchmark-output files (`results/`, `repeats/`) from
  `historical/` -- reproducible, regenerated fresh on every run, and not
  referenced by any source file.

### Changed
- The workspace builds and tests with **zero external crates** -- matching
  the historical projects' own convention, and meaning `cargo build` never
  touches the network.
