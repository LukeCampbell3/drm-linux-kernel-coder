#!/usr/bin/env bash
# Build a bootable Debian-based Linux distribution disk image with drmd
# installed and enabled as a systemd service. The result is a standard
# qcow2 disk image that boots on QEMU/KVM, VirtualBox (after conversion),
# libvirt, or any BIOS-boot-compatible VM/cloud platform that accepts a
# raw or qcow2 disk image.
#
# Requires root (loop devices, mount, chroot) and: debootstrap, parted,
# mkfs.ext4, partprobe, grub-install (grub-pc-bin), qemu-img.
#
# Usage:
#   sudo packaging/vm-image/build-image.sh [--out PATH] [--size SIZE] \
#       [--suite SUITE] [--mirror URL] [--binary PATH]
#
# Env overrides: DRMD_IMG_OUT, DRMD_IMG_SIZE, DRMD_IMG_SUITE,
# DRMD_IMG_MIRROR, DRMD_IMG_BINARY, DRMD_IMG_ROOT_PASSWORD,
# DRMD_IMG_ADMIN_PASSWORD, DRMD_IMG_KEEP_WORKDIR=1 (skip cleanup for
# debugging).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

OUT="${DRMD_IMG_OUT:-$REPO_ROOT/dist/drmd.qcow2}"
SIZE="${DRMD_IMG_SIZE:-2G}"
SUITE="${DRMD_IMG_SUITE:-bookworm}"
MIRROR="${DRMD_IMG_MIRROR:-http://deb.debian.org/debian}"
BINARY="${DRMD_IMG_BINARY:-$REPO_ROOT/target/release/drmd}"
KEEP_WORKDIR="${DRMD_IMG_KEEP_WORKDIR:-0}"
HOSTNAME_VALUE="drmd"

while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --size) SIZE="$2"; shift 2 ;;
    --suite) SUITE="$2"; shift 2 ;;
    --mirror) MIRROR="$2"; shift 2 ;;
    --binary) BINARY="$2"; shift 2 ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 1 ;;
  esac
done

if [ "$(id -u)" -ne 0 ]; then
  echo "error: must run as root (loop devices, mount, chroot)" >&2
  exit 1
fi

for tool in debootstrap parted mkfs.ext4 partprobe grub-install qemu-img blkid chroot; do
  command -v "$tool" >/dev/null 2>&1 || { echo "error: required tool not found: $tool" >&2; exit 1; }
done

if [ ! -e /usr/share/keyrings/debian-archive-keyring.gpg ]; then
  echo "warning: debian-archive-keyring not found on the host -- debootstrap will fetch $SUITE unsigned (install the 'debian-archive-keyring' package to verify Release signatures)" >&2
fi

if [ ! -x "$BINARY" ]; then
  echo "error: drmd binary not found at $BINARY -- run 'cargo build --release --workspace' first" >&2
  exit 1
fi

WORK="$(mktemp -d /tmp/drmd-image.XXXXXX)"
MNT="$WORK/mnt"
RAW="$WORK/disk.raw"
mkdir -p "$MNT"

LOOPDEV=""
MOUNTED_SPECIAL=()

cleanup() {
  set +e
  for m in "${MOUNTED_SPECIAL[@]:-}"; do
    [ -n "$m" ] && umount -R "$m" 2>/dev/null
  done
  if mountpoint -q "$MNT" 2>/dev/null; then
    umount -R "$MNT" 2>/dev/null
  fi
  if [ -n "$LOOPDEV" ]; then
    losetup -d "$LOOPDEV" 2>/dev/null
  fi
  if [ "$KEEP_WORKDIR" != "1" ]; then
    rm -rf "$WORK"
  else
    echo "keeping workdir for inspection: $WORK" >&2
  fi
}
trap cleanup EXIT

log() { echo "[build-image] $*" >&2; }

log "creating ${SIZE} raw disk image"
qemu-img create -f raw "$RAW" "$SIZE" >/dev/null

log "partitioning (msdos, single bootable ext4 partition)"
parted -s "$RAW" mklabel msdos mkpart primary ext4 1MiB 100% set 1 boot on

LOOPDEV="$(losetup -f)"
losetup "$LOOPDEV" "$RAW"
partprobe "$LOOPDEV"
udevadm settle 2>/dev/null || sleep 1
PART="${LOOPDEV}p1"
[ -b "$PART" ] || PART="${LOOPDEV}1"

log "formatting $PART as ext4"
mkfs.ext4 -q -F -L drmroot "$PART"
mount "$PART" "$MNT"

log "debootstrap (stage 1/unpack): $SUITE from $MIRROR (this fetches ~150-300MB, may take a few minutes)"
PACKAGES="systemd-sysv,linux-image-amd64,grub-pc,ca-certificates,coreutils,iproute2,openssh-server,sudo,less,procps"
debootstrap --foreign --arch=amd64 --include="$PACKAGES" "$SUITE" "$MNT" "$MIRROR"

# Package postinst scripts run during stage 2 (systemd-resolved, systemd's
# own units, etc.) expect /proc to exist -- bind-mount the real filesystems
# in *before* running the second stage, not after, or `dpkg --configure`
# fails repeatedly on any postinst that shells out to systemctl/journalctl.
log "binding /dev, /proc, /sys into chroot"
mount --bind /dev "$MNT/dev"; MOUNTED_SPECIAL+=("$MNT/dev")
mount --bind /dev/pts "$MNT/dev/pts"; MOUNTED_SPECIAL+=("$MNT/dev/pts")
mount -t proc proc "$MNT/proc"; MOUNTED_SPECIAL+=("$MNT/proc")
mount -t sysfs sys "$MNT/sys"; MOUNTED_SPECIAL+=("$MNT/sys")

log "debootstrap (stage 2/configure)"
chroot "$MNT" /debootstrap/debootstrap --second-stage

ROOT_UUID="$(blkid -s UUID -o value "$PART")"

log "installing drmd binary and unit"
install -D -m 755 "$BINARY" "$MNT/usr/local/bin/drmd"
install -D -m 644 "$REPO_ROOT/packaging/systemd/drmd.service" "$MNT/etc/systemd/system/drmd.service"
mkdir -p "$MNT/etc/systemd/system/multi-user.target.wants"
ln -sf /etc/systemd/system/drmd.service "$MNT/etc/systemd/system/multi-user.target.wants/drmd.service"

log "writing base system configuration"
echo "$HOSTNAME_VALUE" > "$MNT/etc/hostname"
cat > "$MNT/etc/hosts" <<EOF
127.0.0.1   localhost
127.0.1.1   $HOSTNAME_VALUE
::1         localhost ip6-localhost ip6-loopback
EOF

cat > "$MNT/etc/fstab" <<EOF
UUID=$ROOT_UUID  /  ext4  errors=remount-ro  0  1
EOF

mkdir -p "$MNT/etc/systemd/network"
cat > "$MNT/etc/systemd/network/20-wired-dhcp.network" <<'EOF'
[Match]
Name=en* eth*

[Network]
DHCP=yes
EOF

# systemd-resolved is not installed (its postinst is unreliable inside a
# chroot with no running init/dbus), so DNS is a plain static resolv.conf
# rather than the stub resolver. systemd-networkd still handles DHCP
# address/route configuration on its own.
cat > "$MNT/etc/resolv.conf" <<'EOF'
nameserver 1.1.1.1
nameserver 8.8.8.8
EOF

mkdir -p "$MNT/etc/default"
cat > "$MNT/etc/default/grub" <<EOF
GRUB_DEFAULT=0
GRUB_TIMEOUT=3
GRUB_DISTRIBUTOR="drmd"
GRUB_CMDLINE_LINUX_DEFAULT=""
# grub-mkconfig's own root-device autodetection (grub-probe) fails inside
# this build chroot the same way grub-install's BIOS-drive detection does
# (see the grub-install comment below) and falls back to writing the
# *build-time* device path (root=/dev/loopNpM) into grub.cfg -- a device
# that will not exist on the machine that actually boots this image.
# Pinning root= to the filesystem's UUID here, which we already know, side-
# steps that broken autodetection instead of fighting it.
GRUB_CMDLINE_LINUX="root=UUID=${ROOT_UUID} console=tty0 console=ttyS0,115200n8"
GRUB_DISABLE_OS_PROBER=true
GRUB_TERMINAL="console serial"
GRUB_SERIAL_COMMAND="serial --speed=115200 --unit=0 --word=8 --parity=no --stop=1"
EOF

# No manual serial-getty unit needed: systemd's getty generator starts
# serial-getty@ttyS0.service automatically because "console=ttyS0,..." is
# on the kernel command line above (GRUB_CMDLINE_LINUX).

mkdir -p "$MNT/etc/profile.d"
cat > "$MNT/etc/profile.d/zz-drmd-welcome.sh" <<'EOF'
if [ -z "${DRMD_WELCOME_SHOWN:-}" ] && [ -t 1 ]; then
  export DRMD_WELCOME_SHOWN=1
  echo
  echo "drmd is running as a systemd service (systemctl status drmd)."
  echo "  drmd status --socket /run/drmd/drmd.sock"
  echo "  drmd submit --socket /run/drmd/drmd.sock --task demo --ops fs.read,transform.summarize,fs.write,notify.send --source inputs/demo.csv"
  echo "See /usr/local/share/doc/drmd/ for full documentation."
  echo
fi
EOF

mkdir -p "$MNT/usr/local/share/doc/drmd"
cp "$REPO_ROOT/README.md" "$MNT/usr/local/share/doc/drmd/README.md" 2>/dev/null || true
cp "$REPO_ROOT/docs/ARCHITECTURE.md" "$MNT/usr/local/share/doc/drmd/ARCHITECTURE.md" 2>/dev/null || true

ROOT_PASSWORD="${DRMD_IMG_ROOT_PASSWORD:-}"
ADMIN_PASSWORD="${DRMD_IMG_ADMIN_PASSWORD:-}"
if [ -z "$ADMIN_PASSWORD" ]; then
  # `head -c N /dev/urandom | tr -dc ...` (unbounded tr piped into head)
  # is SIGPIPE-prone under `set -o pipefail`: tr keeps writing after head
  # is satisfied and gets killed, and pipefail turns that 141 into a
  # script-aborting failure. Bounding the read first avoids it.
  ADMIN_PASSWORD="$(head -c 200 /dev/urandom | tr -dc 'A-Za-z0-9' | head -c 20)"
fi

log "configuring accounts and services inside chroot"
chroot "$MNT" /bin/bash -eux <<CHROOT
export DEBIAN_FRONTEND=noninteractive
systemctl enable systemd-networkd ssh drmd || true

id -u drm >/dev/null 2>&1 || useradd -m -s /bin/bash -G sudo drm
echo "drm:${ADMIN_PASSWORD}" | chpasswd
passwd -l root

sed -i 's/^#\?PermitRootLogin.*/PermitRootLogin no/' /etc/ssh/sshd_config
sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication yes/' /etc/ssh/sshd_config

update-initramfs -u -k all
CHROOT

# grub-install's BIOS-drive autodetection (matching a Linux device node to
# a "GRUB drive") reliably fails from *inside* a chroot targeting a loop
# device -- confirmed by testing: identical grub-install invocation,
# identical /proc and /sys bind-mounts, succeeds every time run directly
# on the host and fails every time run chrooted, even with a hand-written
# device.map. Installing the boot sector is therefore done from the host,
# where grub-install sees the loop device the way it would see any other
# real disk. grub-mkconfig (via update-grub, below) has no such issue --
# it only needs to enumerate kernels and filesystem UUIDs, not resolve
# BIOS drive numbers -- so it still runs inside the chroot, against the
# target's own kernel and /etc/default/grub.
log "installing the BIOS boot sector (grub-install, run from the host)"
grub-install --target=i386-pc --boot-directory="$MNT/boot" "$LOOPDEV"

log "generating grub.cfg inside chroot"
chroot "$MNT" update-grub

if [ -n "$ROOT_PASSWORD" ]; then
  chroot "$MNT" /bin/bash -c "echo root:${ROOT_PASSWORD} | chpasswd && passwd -u root"
fi

log "unmounting"
# -R on $MNT itself unmounts every submount (dev, dev/pts, proc, sys) in
# one recursive pass along with $MNT itself -- a separate final `umount
# "$MNT"` after this would always fail ("not mounted") and, under set -e,
# abort the script before it ever reached the qcow2 conversion below.
umount -R "$MNT"
MOUNTED_SPECIAL=()
losetup -d "$LOOPDEV"
LOOPDEV=""

mkdir -p "$(dirname "$OUT")"
log "converting to compressed qcow2: $OUT"
qemu-img convert -O qcow2 -c "$RAW" "$OUT"
qemu-img info "$OUT" >&2

CRED_FILE="${OUT%.qcow2}.credentials.txt"
{
  echo "drmd VM image credentials -- generated $(date -u +%FT%TZ)"
  echo "user: drm  password: ${ADMIN_PASSWORD}"
  echo "root account is password-locked; use 'sudo' from the drm user, or set"
  echo "DRMD_IMG_ROOT_PASSWORD before building to enable a root password."
  echo "Change this password on first login. This file is not committed to git."
} > "$CRED_FILE"
chmod 600 "$CRED_FILE"

log "done: $OUT"
log "credentials written to: $CRED_FILE"
log "boot with, e.g.:"
log "  qemu-system-x86_64 -m 1024 -smp 2 -drive file=$OUT,if=virtio -net nic,model=virtio -net user,hostfwd=tcp::2222-:22 -nographic"
