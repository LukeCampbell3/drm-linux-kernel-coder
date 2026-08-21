# Build and run

```bash
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build -j2
./build/drm_runtime_descent --online-only --out results/run
```

The executable first runs the 99-episode base DRM regression and then the guarded online runtime-descent benchmark.

Key outputs:
- `base_regression/summary.json`
- `runtime_descent_online/runtime_summary.json`
- `runtime_descent_online/online_trace.csv`
- `runtime_descent_online/online_descent_trace.csv`
- `runtime_descent_online/baseline_anchor.csv`
- `runtime_descent_online/online_local_certificate.csv`
- `runtime_descent_online/validation_pairs.csv`
- `runtime_descent_online/family_summary.csv`
