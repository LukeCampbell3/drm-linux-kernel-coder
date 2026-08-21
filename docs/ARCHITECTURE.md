# Architecture

## The idea

`drmd` runs a "developmental runtime" that learns a compressed vocabulary
of recurring workflow motifs from a stream of *episodes* -- a task
identity paired with a sequence of *capabilities* (`fs.read`,
`http.request`, `transform.summarize`, ...). Every capability, and every
symbol later *derived* from capabilities, reduces recursively to nothing
but three root tokens: `OBSERVE`, `DERIVE`, `COMMIT`. That reduction is
audited on every planning step; it is the one invariant that must never
break (see `drm-core::capability` and `drm-core::vocabulary::Vocabulary::audit`).

Each episode is both **planned** (how many "decisions" does representing
this episode cost, given what the runtime has already learned?) and
**executed** (a real, durable side effect against the host: an atomic
file write, an appended log, a spawned process, a socket round-trip).
Planning cost falls over time as the vocabulary learns to name recurring
motifs -- that convergence is the thing this system is for.

## Crate layout

```
crates/
  drm-core/   the planner engine -- pure logic over Vec<String> sequences,
              no I/O, no OS dependency
  drm-exec/   real-Linux execution of planned capabilities
  drmd/       the shipped CLI/daemon binary
```

### `drm-core`

- `capability`: the frozen root vocabulary and the fixed capability ->
  root mapping.
- `vocabulary::Vocabulary`: the learned dictionary of derived words --
  recursive expansion (to capabilities or to roots, both cycle-detected),
  an audit, and a greedy longest-match compressor.
- `planner::DrmPlanner`: an LRU-bounded "active" working set backed by
  unbounded "history", growing the permanent vocabulary via a
  minimum-description-length (MDL) admission rule -- a candidate motif is
  promoted only when doing so *provably* shrinks the total encoded size
  of the history corpus.
- `hybrid::HybridPlanner`: adds a second, faster-forming tier of
  *provisional* vocabulary (capped, grace-period-expiring) plus a
  **deferred consolidation** step (`consolidate_pending`) that moves
  expensive whole-corpus MDL rescoring off the synchronous planning path.
  This is what `drmd serve` uses by default -- see "Why deferred
  consolidation" below.
- `episode::{Episode, PlanMetrics}`, `baseline::Baseline`: the unit of
  work, its planning outcome, and trivial comparison planners
  (stateless / template-cache / checkpoint-replay) used to demonstrate
  that the real planner beats naive caching, not just naive replanning.

### `drm-exec`

- `executor::LiveExecutor`: executes each capability for real --
  filesystem reads/atomic writes, `/proc/self/status` observation, a
  timer wait, a loopback TCP round-trip, a loopback `AF_UNIX`
  round-trip, and a spawned `sha256sum` child process. "COMMIT" in this
  codebase means an atomic write-then-rename, an appended state file, or
  an appended notification log entry -- a real durable effect on the
  local machine, never simulated.
- `servers`: the in-process loopback fixture servers backing
  `http.request`/`ipc.request`, so those capabilities have a real socket
  round-trip to perform without requiring external network access.

### `drmd`

The shipped binary. Subcommands:

| Command | What it does |
|---|---|
| `drmd serve` | Long-running Unix-socket daemon. Accepts real episode submissions, plans and executes each one, returns JSON metrics. **This is the product.** |
| `drmd submit` | CLI client: submit one episode to a running daemon. |
| `drmd status` | CLI client: query a running daemon's state. |
| `drmd bench` | Runs the frozen 99-episode workload end to end and writes the same CSV/JSON report shape the historical benchmarks did. Doubles as a deterministic regression check. |
| `drmd selftest` | Fast, no-I/O invariant check -- a container healthcheck / pre-deploy smoke test. |

`serve`'s wire protocol (`drmd::protocol`) is a deliberately simple
tab-separated `key=value` line in, one line of JSON out, over a Unix
domain socket -- see its module docs for the exact grammar. It carries
only plain identifiers and paths, so it doesn't need a JSON parser for
input whose shape is fully under this project's control.

## Why deferred consolidation

The staged-deployment research prototype (`historical/drm_staged_deployment_peer`)
found that rescoring the *entire* history corpus's MDL cost synchronously,
on every single episode, is the dominant cost of planning at scale: it
measured planner p95 latency falling from ~41ms to ~0.5ms once that
rescoring was moved to a deferred, batched pass. `HybridPlanner` builds
that lesson in from the start: `plan()` only ever does bounded, local
work (diffing against the active/history entry for one task); any new
structural evidence is queued and only actually rescored when
`consolidate_pending()` runs. `drmd serve` runs that on a background
timer (default every 250ms, `--consolidate-ms` to change it) so
submitting an episode never pays for vocabulary maintenance inline.

## Concurrency and persistence (current limits)

`drmd serve` guards its planner and executor behind a single `Mutex`;
each accepted connection is handled on its own thread, serialized through
that lock. This trades fine-grained concurrency for straightforward
correctness -- the right default until profiling says otherwise for a
given deployment's request volume.

There is **no persistence across restarts** in this version: the learned
vocabulary and history live only in memory. This is safe -- every commit
a capability makes to the filesystem is atomic (write-then-rename or
append), so an unplanned restart never corrupts on-disk state -- but it
does mean a restart forgets what the planner had learned. Persisting and
restoring vocabulary state (e.g. periodic snapshot to
`$STATE_DIRECTORY/vocabulary.json`) is the natural next step; it isn't
in v1 because it wants its own design pass (schema versioning, what
happens when the frozen root vocabulary or capability set changes
between versions) rather than being bolted on.

## What did not make it into the shipped product

See `historical/README.md` for the full lineage. In short: the bytecode
dispatch encoding, the coordinate-descent runtime auto-tuner, and the
week-simulation workload generators were valuable research directions
but are orthogonal to the planner's core correctness, and were kept as
archived research rather than bolted onto the production binary.

## Packaging

- `packaging/docker/Dockerfile`: multi-stage build (Rust build stage,
  Debian-slim runtime stage), runs as a non-root user, ships a
  `HEALTHCHECK` that calls `drmd status`.
- `packaging/systemd/drmd.service`: a hardened unit (`DynamicUser`,
  `ProtectSystem=strict`, a narrow `RestrictAddressFamilies`, syscall
  filtering, no capabilities) whose `RuntimeDirectory`/`StateDirectory`
  line up exactly with `drmd`'s own CLI defaults.
- `packaging/vm-image/build-image.sh`: builds a bootable Debian-based
  disk image (qcow2) with `drmd` installed and enabled as that same
  systemd service -- a full, minimal Linux distribution whose entire
  purpose is running this service. See its own `--help` and the root
  README's "Run it as a VM" section.
