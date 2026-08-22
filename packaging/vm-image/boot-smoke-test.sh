#!/usr/bin/env bash
# Boot a built drmd VM image under QEMU (software emulation, no KVM
# required) and verify it reaches a usable state: the kernel boots,
# systemd reaches multi-user.target, and at least one drmd unit starts
# -- either the single shared `drmd.service` (the desktop image's
# default, and always installed-but-disabled on the server image too)
# or a per-application `drmd@<app>.service` instance (the server
# image's default). This checks systemd's own "Started drmd...service"
# status line on the console rather than drmd's own "listening on"
# stderr line -- under systemd, a service's stdout/stderr goes to the
# journal by default, not the console, so the latter never appears in
# the serial log even on a fully successful boot.
#
# Usage: packaging/vm-image/boot-smoke-test.sh path/to/drmd.qcow2 [timeout_seconds]

set -euo pipefail

IMAGE="${1:?usage: boot-smoke-test.sh <image.qcow2> [timeout_seconds]}"
TIMEOUT="${2:-90}"

command -v qemu-system-x86_64 >/dev/null 2>&1 || { echo "error: qemu-system-x86_64 not found" >&2; exit 1; }

WORK="$(mktemp -d /tmp/drmd-boot-test.XXXXXX)"
SNAPSHOT="$WORK/snapshot.qcow2"
SERIAL_LOG="$WORK/serial.log"
trap 'rm -rf "$WORK"' EXIT

# Boot a disposable overlay so the smoke test never mutates the built image.
qemu-img create -f qcow2 -b "$(cd "$(dirname "$IMAGE")" && pwd)/$(basename "$IMAGE")" -F qcow2 "$SNAPSHOT" >/dev/null

qemu-system-x86_64 \
  -m 1024 -smp 2 \
  -drive file="$SNAPSHOT",if=virtio \
  -net nic,model=virtio -net user \
  -display none -serial file:"$SERIAL_LOG" \
  -no-reboot \
  >"$WORK/qemu.log" 2>&1 &
QEMU_PID=$!

echo "booting under QEMU (pid $QEMU_PID), watching $SERIAL_LOG for up to ${TIMEOUT}s..." >&2

deadline=$((SECONDS + TIMEOUT))
found=0
while [ $SECONDS -lt $deadline ]; do
  if [ -f "$SERIAL_LOG" ] && grep -aqE "Started.*drmd(@[^[:space:]]+)?\.service" "$SERIAL_LOG" 2>/dev/null; then
    found=1
    break
  fi
  if ! kill -0 "$QEMU_PID" 2>/dev/null; then
    echo "qemu exited early" >&2
    break
  fi
  sleep 2
done

kill "$QEMU_PID" 2>/dev/null || true
wait "$QEMU_PID" 2>/dev/null || true

if [ "$found" = "1" ]; then
  echo "PASS: systemd started a drmd unit within ${TIMEOUT}s" >&2
  exit 0
else
  echo "FAIL: did not observe a drmd unit starting within ${TIMEOUT}s -- last serial output:" >&2
  tail -n 60 "$SERIAL_LOG" 2>/dev/null >&2 || true
  exit 1
fi
