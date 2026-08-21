# DRM staged deployment peer protocol

1. Build natively with GCC 14+ or Clang 17+:
   `g++ -std=c++23 -O3 -Wall -Wextra -Wpedantic -Werror src/staged_deploy.cpp -pthread -o drm_staged_deploy`
2. Run: `./drm_staged_deploy --out results/peer`
3. Expected deterministic structural values: episodes=708, success=708, semantic=831, permanent=22, provisional=20, fused_microcode_bytes=538, O/D/C audit=true.
4. Timing is host-dependent; report per-stage planner and consolidation P95 from `stage_metrics.csv`.
5. Docker alternative: `docker build -t drm-staged .` then `docker run --rm --network=none -v "$PWD/results/docker:/results" drm-staged`. Note: the benchmark internally uses loopback TCP/Unix IPC; Docker `--network=none` still permits loopback.
