# Archived research

This directory holds the eight experimental peer-benchmark projects that
preceded the production runtime. They are **not built by the top-level
project** and are kept only for provenance and historical reference.

Each project explored one facet of the same idea — an "O/D/C" (OBSERVE /
DERIVE / COMMIT) developmental runtime that learns a compressed vocabulary
of recurring workflow motifs from a stream of task episodes:

| Directory | What it explored |
|---|---|
| `drm_bytecode_peer/` | A compact bytecode/dense-microop encoding for the O/D/C instruction stream (dispatch-speed microbenchmark). |
| `drm_cpp_odc_peer_bundle/` | The reference C++ engine, packaged as a standalone reproducible Docker peer artifact. |
| `drm_learning_path_optimized_peer/` | A two-tier "provisional + permanent" vocabulary that admits candidate abstractions faster, expiring the ones that don't earn permanence. |
| `drm_live_odc_peer_bundle/` | An independent, idiomatic, dependency-free Rust rewrite of the same engine — the direct ancestor of `crates/drm-core` and `crates/drm-exec`. |
| `drm_runtime_descent_peer/` | A coordinate-descent auto-tuner for I/O and execution parameters, layered on top of a seeded planner history. |
| `drm_rust_eval/` | An earlier, purely in-memory Rust prototype with a different (14-symbol) vocabulary contract; a design-exploration harness, not O/D/C. |
| `drm_staged_deployment_peer/` | A six-stage canary → GA deployment lifecycle with explicit gates (success rate, vocabulary audit, p95 planner latency) and a deferred/background vocabulary-consolidation pass that cut planner p95 from ~41ms to ~0.5ms. |
| `drm_week_sim_peer/` | A synthetic week-long usage simulation, including a "mature" variant that replays a full week as warm-up before measuring steady-state behavior. |

## What survived into the production runtime

The consolidated, production-grade rewrite lives in `crates/drm-core`,
`crates/drm-exec`, and `crates/drmd` at the repository root. It carries
forward:

- The core `Vocabulary` / `DrmPlanner` engine (from `drm_live_odc_peer_bundle`,
  extended to the fuller 12-capability set used by the C++ variants).
- The two-tier provisional/permanent vocabulary and deferred consolidation
  (from `drm_learning_path_optimized_peer` and `drm_staged_deployment_peer`),
  as the `HybridPlanner`.
- The real-Linux `LiveExecutor` (filesystem, `/proc`, loopback TCP/Unix
  sockets, process spawn) that gives every planned episode a real,
  durable side effect rather than a simulated one.
- The frozen 99-episode workload and its documented deterministic
  regression numbers, ported byte-for-byte as `drmd`'s `bench` subcommand
  and covered by an automated regression test.

The bytecode encoding, runtime-descent auto-tuner, and week-simulation
workload generators were deliberately **not** carried forward as shipped
product surface — they were valuable research directions but are orthogonal
to the planner's core correctness, and are preserved here rather than
bolted onto the production binary.

## Building the historical projects

Each subdirectory is still self-contained and buildable with the toolchain
its own `README.md`/`PEER_PROTOCOL.md` documents (GCC/Clang + CMake, or
Cargo). They are not part of the workspace build and are not covered by CI.

Generated benchmark output (`results/`, `repeats/`) has been removed from
version control — it was reproducible, regenerated fresh on every run, and
not referenced by any source file. Re-run a project's own scripts to
regenerate it locally.
