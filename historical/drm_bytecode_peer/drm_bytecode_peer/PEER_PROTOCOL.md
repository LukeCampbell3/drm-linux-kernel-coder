# Peer Replication Protocol

1. Build on x86-64 Linux with GCC 14.x or Clang 17+ and CMake/Ninja.
2. Run `ctest --test-dir build --output-on-failure`.
3. Run `./build/drm_bytecode --out results/peer` at least five times for dispatch timing.
4. Treat representation sizes, round-trip counts, root audits, developmental counts, and historical-block immutability as deterministic checks.
5. Treat nanosecond dispatch ratios and live task wall time as hardware/compiler-dependent measurements; report median/mean rather than expecting exact equality.
6. Do not use live I/O wall time to infer bytecode dispatch speed. Use `dispatch_benchmark.csv` for that comparison.
7. Verify every dense micro-op expands to the capability's frozen O/D/C signature. Any mismatch is a hard test failure.
