#!/bin/sh
set -eu

usage() {
    echo "usage: $0 --image IMAGE --profile 8gb-cpu|12gb-cpu|16gb-cpu|24gb-cpu [--gpu PCI_ID] [--ssh-port PORT] [--dry-run]" >&2
    exit 2
}

IMAGE=
PROFILE=
GPU=
SSH_PORT=2222
DRY_RUN=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --image) [ "$#" -ge 2 ] || usage; IMAGE=$2; shift 2 ;;
        --profile) [ "$#" -ge 2 ] || usage; PROFILE=$2; shift 2 ;;
        --gpu) [ "$#" -ge 2 ] || usage; GPU=$2; shift 2 ;;
        --ssh-port) [ "$#" -ge 2 ] || usage; SSH_PORT=$2; shift 2 ;;
        --dry-run) DRY_RUN=1; shift ;;
        *) usage ;;
    esac
done
[ -n "$IMAGE" ] && [ -n "$PROFILE" ] || usage

case "$PROFILE" in
    8gb-cpu) MEMORY=8192; CPUS=2 ;;
    12gb-cpu) MEMORY=12288; CPUS=4 ;;
    16gb-cpu) MEMORY=16384; CPUS=6 ;;
    24gb-cpu) MEMORY=24576; CPUS=8 ;;
    *) echo "unknown profile: $PROFILE" >&2; exit 2 ;;
esac
case "$SSH_PORT" in *[!0-9]*|'') echo "invalid SSH port" >&2; exit 2 ;; esac
if [ -n "$GPU" ]; then
    case "$GPU" in *[!0-9a-fA-F:.]*) echo "invalid PCI id" >&2; exit 2 ;; esac
fi
if [ "$DRY_RUN" -ne 1 ]; then
    [ -f "$IMAGE" ] || { echo "image not found: $IMAGE" >&2; exit 1; }
    command -v qemu-system-x86_64 >/dev/null || { echo "qemu-system-x86_64 is required" >&2; exit 1; }
fi

set -- qemu-system-x86_64 -enable-kvm -machine q35,accel=kvm -cpu host \
    -m "$MEMORY" -smp "$CPUS" -drive "file=$IMAGE,format=raw,if=virtio" \
    -device virtio-rng-pci -netdev "user,id=net0,hostfwd=tcp:127.0.0.1:$SSH_PORT-:22" \
    -device virtio-net-pci,netdev=net0 -nographic
if [ -n "$GPU" ]; then
    set -- "$@" -device "vfio-pci,host=$GPU"
fi
if [ "$DRY_RUN" -eq 1 ]; then
    printf '%s\n' "$*"
else
    exec "$@"
fi
