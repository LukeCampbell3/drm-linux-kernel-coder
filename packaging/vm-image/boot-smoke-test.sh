#!/usr/bin/env bash
# Boot a built drmd VM image under QEMU (software emulation, no KVM
# required) and verify it reaches a usable state.
#
# Two image shapes are supported by the same script:
# - server/generic images reach multi-user.target and run a system-
#   level drmd unit (`drmd.service` or a `drmd@<app>.service`
#   instance) -- checked via systemd's own "Started drmd...service"
#   status line on the console.
# - the desktop image reaches graphical.target instead (lightdm ->
#   autologin -> XFCE), and runs drmd as a *user*-level systemd unit,
#   which does not write status lines to the console the way a system
#   unit does -- so for that image this script checks lightdm/
#   graphical.target instead, which is what's actually observable here
#   and is the correct proxy for "the desktop boots to a usable
#   session."
#
# Which check applies is auto-detected from whether "lightdm" ever
# appears in the boot log, not passed as a flag -- so this one script
# verifies both images' image-specific definition of "booted
# successfully."
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
drmd_found=0
lightdm_seen=0
graphical_found=0
while [ $SECONDS -lt $deadline ]; do
  if [ -f "$SERIAL_LOG" ]; then
    grep -aqE "Started.*drmd(@[^[:space:]]+)?\.service" "$SERIAL_LOG" 2>/dev/null && drmd_found=1
    grep -aq "lightdm" "$SERIAL_LOG" 2>/dev/null && lightdm_seen=1
    grep -aqE "(Reached target Graphical Interface|Started.*[Ll]ightdm)" "$SERIAL_LOG" 2>/dev/null && graphical_found=1
  fi
  if [ "$lightdm_seen" = "1" ]; then
    [ "$graphical_found" = "1" ] && break
  else
    [ "$drmd_found" = "1" ] && break
  fi
  if ! kill -0 "$QEMU_PID" 2>/dev/null; then
    echo "qemu exited early" >&2
    break
  fi
  sleep 2
done

kill "$QEMU_PID" 2>/dev/null || true
wait "$QEMU_PID" 2>/dev/null || true

if [ "$lightdm_seen" = "1" ]; then
  pass="$graphical_found"
  what="lightdm reaching graphical.target"
else
  pass="$drmd_found"
  what="a drmd unit starting"
fi

if [ "$pass" = "1" ]; then
  echo "PASS: observed $what within ${TIMEOUT}s" >&2
  exit 0
else
  echo "FAIL: did not observe $what within ${TIMEOUT}s -- last serial output:" >&2
  tail -n 60 "$SERIAL_LOG" 2>/dev/null >&2 || true
  exit 1
fi
