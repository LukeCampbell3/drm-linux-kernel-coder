#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
rm -rf peer-results
mkdir -p peer-results
# network_mode:none is intentional: the benchmark's HTTP traffic is loopback-only and served inside the process.
docker compose build --pull
docker compose run --rm drm-live
sha256sum peer-results/* 2>/dev/null | sort > peer-results/SHA256SUMS.txt || true
docker image inspect drm-live-odc:peer-v1 > peer-results/docker_image_inspect.json
printf '\nPeer results: %s/peer-results\n' "$ROOT"
