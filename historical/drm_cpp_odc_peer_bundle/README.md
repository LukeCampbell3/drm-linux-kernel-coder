# DRM O/D/C Native C++ Live Benchmark

Native C++20 Linux benchmark for the developmental runtime using the frozen root vocabulary:
`OBSERVE`, `DERIVE`, `COMMIT`.

The workload uses real filesystem atomic commits, fork/exec child processes, loopback TCP, Unix-domain sockets, `/proc` observation, timer/event observation, state snapshots, local repair, hot-context eviction, and one-shot ancestral recovery. No browser or LLM is required for the core benchmark.

## Native run
```bash
./scripts/run_peer.sh
```

## Docker peer run
```bash
docker compose build
docker compose run --rm drm
```

Generated results include `live_trace.csv`, `vocabulary_audit.csv`, `baseline_comparison.csv`, and `summary.json`.
