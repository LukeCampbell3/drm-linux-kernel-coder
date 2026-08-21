#!/usr/bin/env bash
set -euo pipefail
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build -j2
ctest --test-dir build --output-on-failure
rm -rf peer-results && mkdir peer-results
./build/drm_odc_live --out peer-results
sha256sum build/drm_odc_live peer-results/*.csv peer-results/summary.json | tee peer-results/SHA256SUMS.txt
