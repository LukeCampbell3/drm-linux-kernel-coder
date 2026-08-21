# DRM O/D/C Peer Replication Protocol

Root vocabulary is frozen to exactly `OBSERVE`, `DERIVE`, `COMMIT`. Every capability and every derived vocabulary entry must recursively reduce to only these roots; cycles or unknown leaves fail the audit.

## Native
Requirements: Linux x86-64, GCC 14+ or Clang 17+, CMake 3.20+, Ninja, coreutils.

```bash
./scripts/run_peer.sh
```

Expected deterministic result for the frozen 99-episode workload:
- success: 99/99
- semantic_total: 214
- derived_final: 11
- structure_bytes_final: 1797
- recoveries: 4
- local_repairs: 4
- raw_task_tokens: 273
- compressed_task_tokens: 70
- definition_tokens: 37
- description-length reduction: 0.567766
- root counts: OBSERVE=141, DERIVE=390, COMMIT=230

Wall time and RSS are host-dependent.

## Docker
```bash
docker compose build
docker compose run --rm drm
```
The runtime container has no external network, a read-only root filesystem, 2 CPUs, 512 MiB memory, and 128 PID limit. Test TCP/Unix-socket services are loopback/local only.

Return `summary.json`, `baseline_comparison.csv`, `vocabulary_audit.csv`, `live_trace.csv`, compiler/kernel versions, and `SHA256SUMS.txt` for peer comparison.
