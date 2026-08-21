#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
mkdir -p artifacts
docker compose build --pull
docker save drm-live-odc:peer-v1 -o artifacts/drm-live-odc-peer-v1.tar
sha256sum artifacts/drm-live-odc-peer-v1.tar > artifacts/drm-live-odc-peer-v1.tar.sha256
printf 'Exported %s/artifacts/drm-live-odc-peer-v1.tar\n' "$ROOT"
