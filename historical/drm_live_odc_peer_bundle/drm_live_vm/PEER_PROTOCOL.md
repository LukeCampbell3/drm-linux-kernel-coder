# DRM O/D/C Peer Replication Protocol

1. Use an x86-64 Linux host with Docker Engine + Compose.
2. Do not edit `src/main.rs`, `Cargo.toml`, `Dockerfile`, or workload constants before the first run.
3. Run `./scripts/run_peer.sh`.
4. Preserve `peer-results/` in full.
5. Export the image with `./scripts/export_image.sh` if a binary-identical image needs to be shared.
6. Report CPU model, RAM, kernel, Docker version, and whether virtualization is bare metal, VM, WSL2, or cloud.
7. Compare functional invariants before comparing timing.
8. Run at least five repetitions for timing; do not treat a single wall-time sample as an efficiency claim.
9. Any vocabulary entry whose recursive expansion does not terminate exclusively in `OBSERVE`, `DERIVE`, `COMMIT` invalidates the run.
10. Any task that uses ancestral recovery twice in succession without an intervening task change is a forward-integration failure.

## Recommended peer measurements

- task success rate
- total and per-phase semantic decisions
- wall-clock time
- CPU user/system time (container-level)
- peak RSS / container memory high-water mark
- bytes read/written
- process count / PIDs high-water mark
- derived vocabulary count
- average/max abstraction depth
- description-length ratio
- structural byte size
- vocabulary additions/removals
- ancestral recovery count
- local repair count
- post-recovery repeat cost
- model calls/tokens when Ollama is added later

The first frozen peer test should not add Ollama. It validates the O/D/C developmental substrate. Model-coupled routing is a second experiment.
