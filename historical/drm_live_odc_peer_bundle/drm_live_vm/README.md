# DRM Live Linux O/D/C Evaluation

This package tests the lowest-level DRM vocabulary:

- `OBSERVE`
- `DERIVE`
- `COMMIT`

Every capability and every learned/derived word must recursively reduce to those three roots. The benchmark exercises real Linux filesystem operations, atomic writes, state updates, process spawning (`sha256sum`), loopback TCP/HTTP retrieval, and notification-log commits.

## What was executed in this environment

The available host is a live Linux container (kernel 6.18.35), not a hypervisor VM. It did not contain `rustc`, `cargo`, Docker, or Podman, and shell network access is disabled. Therefore:

1. The live workload was executed with `live_eval.py`, a deterministic behavioral reference of the Rust algorithm.
2. The same workload was repeated inside a separate Linux user/mount/PID namespace using `unshare`.
3. The Rust implementation is included in `src/main.rs`, with unit tests and a pinned multi-stage Docker build for peer execution.
4. The Rust binary and Docker image were **not compiled/built on this host**. Peer execution is required before Rust wall/RSS numbers can be claimed.

This distinction is important: the live metrics in `results/` validate the DRM mechanism against real Linux operations, but the resource numbers are Python-reference numbers, not Rust-runtime performance numbers.

## Live workload

65 episodes per run:

- 12 warm-up/repetitive episodes
- 25 novel compositional tasks
- 12 repeats of previously novel tasks
- 3 workflow-drift/local-repair tasks
- 7 hot-context eviction tasks
- 3 forced ancestral recoveries
- 3 immediate post-recovery repeats

Five complete direct runs were executed, plus one isolated namespace run.

## Main live result

See `results/aggregate_metrics.json`.

- Task success: 100% (65/65 per run; 5/5 runs)
- Root vocabulary audit: 100% uniform
- Semantic decisions: 128 total / 1.9692 mean
- Final derived vocabulary: 8 words
- Average abstraction depth: 1.875
- Maximum abstraction depth: 3
- Historical recoveries: 3
- Local repairs: 3
- Final Python-reference structure: 1,813 bytes
- Full-run wall time: 1,138.845 ms mean over 5 runs, 66.862 ms sample standard deviation
- Model/Ollama calls: 0 (model deliberately excluded from this kernel test)

## Convergence

The first five novel compositional tasks averaged 4.2 semantic decisions. The last five novel compositional tasks averaged 1.6, a 61.9% reduction while the tasks remained novel exact identities.

Repeated acquired tasks converged to 1 semantic decision. Each forced ancestral recovery required recovery once; the immediately repeated task then required 1 semantic decision and no recovery.

## Structural growth

For the final 36 unique task identities:

- Raw task corpus: 165 capability tokens
- Compressed task representations: 40 tokens
- Derived definitions: 21 tokens
- Derived word headers: 8 tokens
- Total description length: 69 tokens
- Description-length reduction: 58.2%

The flat exact-template baseline occupied 3,071 serialized reference bytes versus 1,813 bytes for DRM, a 41.0% reduction in the reference representation.

## Baseline comparison

Planner comparison on the identical 65-episode workload:

| System | Mean semantic decisions | Final reference structure |
|---|---:|---:|
| Stateless replanning | 4.5385 | 0 B |
| Exact template cache | 3.3077 | 3,071 B |
| Checkpoint replay | 2.9846 | 3,071 B |
| DRM O/D/C | 1.9692 | 1,813 B |

DRM reduced semantic decisions by 56.6% vs stateless replanning, 40.5% vs exact-template caching, and 34.0% vs checkpoint replay on this workload.

These are mechanism baselines, not direct measurements of OpenHands/LangGraph/AutoGen products.

## Uniform vocabulary

`results/vocabulary_audit.csv` contains every learned word, its recursive capability expansion, and its terminal root expansion. All leaves are one of `OBSERVE`, `DERIVE`, or `COMMIT`; no cycles or unknown leaves occurred.

Example:

`d006 = fs.read -> d003`

reduces to:

`fs.read -> transform.extract -> transform.summarize -> fs.write -> notify.send`

and ultimately:

`OBSERVE -> DERIVE -> DERIVE -> DERIVE -> COMMIT -> DERIVE -> COMMIT`

## Peer Docker test

The Dockerfile is intentionally dependency-free at the Cargo layer and uses pinned image digests. Runtime networking is disabled; its HTTP test is loopback-only and served inside the Rust process.

Run:

```bash
./scripts/run_peer.sh
```

or:

```bash
docker compose build --pull
docker compose run --rm drm-live
```

Export the exact peer image:

```bash
./scripts/export_image.sh
```

This creates:

```text
artifacts/drm-live-odc-peer-v1.tar
artifacts/drm-live-odc-peer-v1.tar.sha256
```

A peer can reproduce with:

```bash
docker load -i drm-live-odc-peer-v1.tar
mkdir -p peer-results
docker run --rm \
  --network none \
  --memory 2g \
  --cpus 2 \
  -v "$PWD/peer-results:/results" \
  drm-live-odc:peer-v1
```

## Expected Rust peer invariants

The Rust peer run should be checked first for invariants, not wall time:

- 65 episodes
- 100% task success
- root vocabulary exactly `[OBSERVE, DERIVE, COMMIT]`
- uniform vocabulary audit true
- no vocabulary cycles
- 3 ancestral recoveries
- 3 local repairs
- immediate post-recovery tasks require no second recovery
- repeated acquired tasks collapse to one semantic decision

The Rust implementation uses a slightly different byte-accounting function from the Python JSON reference, so exact `structure_bytes` equality between languages is not a required invariant. Vocabulary count, reductions, decisions, recovery/repair counts, and task outcomes are the cross-language invariants.

## Chromium / Ollama status

Chromium is installed on the host, but headless Chromium hangs under this sandbox's missing DBus/system-service environment. It was therefore excluded from the frozen live workload rather than converting environment failure into a DRM failure.

Ollama is not installed and cannot be downloaded through the shell in this environment. `model_calls`, `model_input_tokens`, and `model_output_tokens` remain present as metrics and are zero for this kernel-only test. A model-coupled peer experiment should be a separate layer so model calibration/inference cost does not confound this structural validation.

## Files

- `src/main.rs` — std-only Rust DRM runtime + live workload + unit tests
- `Cargo.toml` — no external crates
- `Dockerfile` — pinned Rust/Debian peer image
- `compose.yaml` — 2 CPU / 2 GiB constrained peer execution
- `live_eval.py` — live behavioral reference used on this host
- `results/live_trace.csv` — per-episode Linux/runtime metrics
- `results/phase_summary.csv` — phase aggregates
- `results/baseline_comparison.csv` — mechanism baselines
- `results/vocabulary_audit.csv` — complete recursive uniformity audit
- `results/convergence_bins.csv` — convergence through time
- `results/aggregate_metrics.json` — final metrics
- `repeats/repeat_summary.csv` — five-run stability results
