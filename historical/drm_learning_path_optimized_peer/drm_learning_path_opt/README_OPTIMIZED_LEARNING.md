# DRM optimized learning path

Build:

```bash
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build
ctest --test-dir build --output-on-failure
./build/drm_learning_opt --out results/final
```

The frozen implementation is `src/hybrid_fast.cpp`. It imports the tested base DRM workload from `src/base_main.cpp`. See `results/final/RESULTS.md`.
