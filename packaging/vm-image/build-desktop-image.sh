#!/usr/bin/env bash
# Build a bootable Debian-based Linux distribution disk image with a
# full XFCE desktop (lightdm autologin) and drmd installed as a
# *user*-level systemd unit (spec S6's desktop deployment model: `User
# -> Applications -> DRM Adaptive Execution Layer -> OS -> Hardware`,
# single logged-in user, drmd running whenever the machine is, not just
# while actively logged in). The result is a standard qcow2 disk image
# that boots on QEMU/KVM, VirtualBox (after conversion), libvirt, or any
# BIOS-boot-compatible VM/cloud platform that accepts a raw or qcow2
# disk image.
#
# This is a sibling of build-image.sh (the server/generic image), not a
# variant of it: they share the same proven debootstrap -> grub -> qcow2
# mechanics (partition, format, debootstrap, grub-install-from-host,
# convert), duplicated here rather than parameterized, because the
# desktop-specific package set, account/autologin configuration, and
# systemd target are different enough in kind (not just in a few
# variable values) that a shared script would need as many branches as
# it saved lines. See build-image.sh's own comments for the parts that
# are identical (grub-install-from-the-host workaround, the pipefail-
# safe password generator, etc.) -- they are not re-explained here.
#
# Requires root (loop devices, mount, chroot) and: debootstrap, parted,
# mkfs.ext4, partprobe, grub-install (grub-pc-bin), qemu-img.
#
# Usage:
#   sudo packaging/vm-image/build-desktop-image.sh [--out PATH] [--size SIZE] \
#       [--suite SUITE] [--mirror URL] [--binary PATH]
#
# Env overrides: DRMD_IMG_OUT, DRMD_IMG_SIZE, DRMD_IMG_SUITE,
# DRMD_IMG_MIRROR, DRMD_IMG_BINARY, DRMD_IMG_ROOT_PASSWORD,
# DRMD_IMG_ADMIN_PASSWORD, DRMD_IMG_KEEP_WORKDIR=1 (skip cleanup for
# debugging).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

OUT="${DRMD_IMG_OUT:-$REPO_ROOT/dist/drmd-desktop.qcow2}"
# A full XFCE desktop is materially larger on disk than the headless
# server image (X11, fonts, XFCE itself, its dependencies) -- 2G (the
# server image's default) is not enough headroom.
SIZE="${DRMD_IMG_SIZE:-6G}"
SUITE="${DRMD_IMG_SUITE:-bookworm}"
MIRROR="${DRMD_IMG_MIRROR:-http://deb.debian.org/debian}"
BINARY="${DRMD_IMG_BINARY:-$REPO_ROOT/target/release/drmd}"
KEEP_WORKDIR="${DRMD_IMG_KEEP_WORKDIR:-0}"
HOSTNAME_VALUE="drmd-desktop"
DESKTOP_USER="drm"

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

WORK="$(mktemp -d /tmp/drmd-desktop-image.XXXXXX)"
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

log() { echo "[build-desktop-image] $*" >&2; }

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
mkfs.ext4 -q -F -L drmdesktop "$PART"
mount "$PART" "$MNT"

log "debootstrap (stage 1/unpack): $SUITE from $MIRROR, full XFCE desktop (this fetches several hundred MB, may take 10+ minutes)"
PACKAGES="systemd-sysv,linux-image-amd64,grub-pc,ca-certificates,coreutils,iproute2,openssh-server,sudo,less,procps,xserver-xorg,xinit,xfce4,xfce4-terminal,lightdm,lightdm-gtk-greeter,dbus-x11,fonts-dejavu-core,chromium,chromium-driver,python3,python3-selenium"
debootstrap --foreign --arch=amd64 --include="$PACKAGES" "$SUITE" "$MNT" "$MIRROR"

log "binding /dev, /proc, /sys into chroot"
mount --bind /dev "$MNT/dev"; MOUNTED_SPECIAL+=("$MNT/dev")
mount --bind /dev/pts "$MNT/dev/pts"; MOUNTED_SPECIAL+=("$MNT/dev/pts")
mount -t proc proc "$MNT/proc"; MOUNTED_SPECIAL+=("$MNT/proc")
mount -t sysfs sys "$MNT/sys"; MOUNTED_SPECIAL+=("$MNT/sys")

log "debootstrap (stage 2/configure)"
chroot "$MNT" /debootstrap/debootstrap --second-stage

ROOT_UUID="$(blkid -s UUID -o value "$PART")"

log "installing drmd binary (no system-level unit -- this image runs drmd as a per-user systemd --user service)"
install -D -m 755 "$BINARY" "$MNT/usr/local/bin/drmd"
install -D -m 755 "$REPO_ROOT/packaging/selenium/selenium_bridge.py" "$MNT/usr/local/lib/drmd/selenium_bridge.py"

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

cat > "$MNT/etc/resolv.conf" <<'EOF'
nameserver 1.1.1.1
nameserver 8.8.8.8
EOF

mkdir -p "$MNT/etc/default"
cat > "$MNT/etc/default/grub" <<EOF
GRUB_DEFAULT=0
GRUB_TIMEOUT=3
GRUB_DISTRIBUTOR="drmd-desktop"
GRUB_CMDLINE_LINUX_DEFAULT=""
# See build-image.sh's identical stanza: grub-mkconfig's root-device
# autodetection is unreliable inside this build chroot, so root= is
# pinned to the filesystem's already-known UUID.
GRUB_CMDLINE_LINUX="root=UUID=${ROOT_UUID} console=tty0 console=ttyS0,115200n8"
GRUB_DISABLE_OS_PROBER=true
GRUB_TERMINAL="console serial"
GRUB_SERIAL_COMMAND="serial --speed=115200 --unit=0 --word=8 --parity=no --stop=1"
EOF

ROOT_PASSWORD="${DRMD_IMG_ROOT_PASSWORD:-}"
ADMIN_PASSWORD="${DRMD_IMG_ADMIN_PASSWORD:-}"
if [ -z "$ADMIN_PASSWORD" ]; then
  # Bounded read before `tr -dc` to avoid a SIGPIPE-under-pipefail abort
  # (see build-image.sh's identical comment).
  ADMIN_PASSWORD="$(head -c 200 /dev/urandom | tr -dc 'A-Za-z0-9' | head -c 20)"
fi

log "configuring accounts and services inside chroot"
chroot "$MNT" /bin/bash -eux <<CHROOT
export DEBIAN_FRONTEND=noninteractive
systemctl enable systemd-networkd ssh lightdm || true

id -u ${DESKTOP_USER} >/dev/null 2>&1 || useradd -m -s /bin/bash -G sudo ${DESKTOP_USER}
echo "${DESKTOP_USER}:${ADMIN_PASSWORD}" | chpasswd
passwd -l root

sed -i 's/^#\?PermitRootLogin.*/PermitRootLogin no/' /etc/ssh/sshd_config
sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication yes/' /etc/ssh/sshd_config

update-initramfs -u -k all
CHROOT

log "configuring lightdm autologin and graphical boot target"
mkdir -p "$MNT/etc/lightdm/lightdm.conf.d"
cat > "$MNT/etc/lightdm/lightdm.conf.d/50-drmd-autologin.conf" <<EOF
[Seat:*]
autologin-user=${DESKTOP_USER}
autologin-user-timeout=0
autologin-session=xfce
user-session=xfce
EOF
# Installed via apt inside a chroot with no running init, lightdm's own
# postinst cannot reliably wire itself up as the display manager the
# way it would on a normal running system -- set both symlinks
# explicitly instead of depending on that postinst step succeeding.
ln -sf /lib/systemd/system/lightdm.service "$MNT/etc/systemd/system/display-manager.service"
ln -sf /lib/systemd/system/graphical.target "$MNT/etc/systemd/system/default.target"

log "installing drmd as a per-user systemd --user service for ${DESKTOP_USER}"
USER_UNIT_DIR="$MNT/home/${DESKTOP_USER}/.config/systemd/user"
mkdir -p "$USER_UNIT_DIR/default.target.wants"
install -m 644 "$REPO_ROOT/packaging/systemd/drmd-user.service" "$USER_UNIT_DIR/drmd.service"
install -m 644 "$REPO_ROOT/packaging/systemd/drmd-simulate-desktop-user.service" "$USER_UNIT_DIR/drmd-simulate-desktop.service"
ln -sf ../drmd.service "$USER_UNIT_DIR/default.target.wants/drmd.service"
ln -sf ../drmd-simulate-desktop.service "$USER_UNIT_DIR/default.target.wants/drmd-simulate-desktop.service"
chroot "$MNT" chown -R "${DESKTOP_USER}:${DESKTOP_USER}" "/home/${DESKTOP_USER}/.config"
# The `systemctl --user enable`/`loginctl enable-linger` a running
# system would use both reduce, on disk, to the .wants symlinks above
# plus this marker file -- written directly for the same reason the
# .wants symlinks are, throughout this pipeline: there is no running
# systemd inside the build chroot to ask.
mkdir -p "$MNT/var/lib/systemd/linger"
touch "$MNT/var/lib/systemd/linger/${DESKTOP_USER}"

mkdir -p "$MNT/etc/profile.d"
cat > "$MNT/etc/profile.d/zz-drmd-welcome.sh" <<EOF
if [ -z "\${DRMD_WELCOME_SHOWN:-}" ] && [ -t 1 ]; then
  export DRMD_WELCOME_SHOWN=1
  echo
  echo "drmd is running as your per-user systemd service (systemctl --user status drmd)."
  echo "  drmd status --socket \\\$XDG_RUNTIME_DIR/drmd.sock"
  echo "  drmd submit --socket \\\$XDG_RUNTIME_DIR/drmd.sock --task demo --ops fs.read,transform.summarize,fs.write,notify.send --source inputs/demo.csv"
  echo "Comparative benchmark results: ~/.local/share/drmd-demo/simulate/ (systemctl --user status drmd-simulate-desktop)"
  echo "See /usr/local/share/doc/drmd/ for full documentation."
  echo
fi
EOF

mkdir -p "$MNT/usr/local/share/doc/drmd"
cp "$REPO_ROOT/README.md" "$MNT/usr/local/share/doc/drmd/README.md" 2>/dev/null || true
cp "$REPO_ROOT/docs/ARCHITECTURE.md" "$MNT/usr/local/share/doc/drmd/ARCHITECTURE.md" 2>/dev/null || true
cp "$REPO_ROOT/docs/SELENIUM_WEB.md" "$MNT/usr/local/share/doc/drmd/SELENIUM_WEB.md" 2>/dev/null || true
cp "$REPO_ROOT/docs/RUNTIME_MUTATION.md" "$MNT/usr/local/share/doc/drmd/RUNTIME_MUTATION.md" 2>/dev/null || true
cp "$REPO_ROOT/docs/OBSERVE_FIRST_APPS.md" "$MNT/usr/local/share/doc/drmd/OBSERVE_FIRST_APPS.md" 2>/dev/null || true
cp "$REPO_ROOT/docs/reports/desktop-experiment.md" "$MNT/usr/local/share/doc/drmd/desktop-experiment.md" 2>/dev/null || true

log "installing the BIOS boot sector (grub-install, run from the host -- see build-image.sh's comment on why)"
grub-install --target=i386-pc --boot-directory="$MNT/boot" "$LOOPDEV"

log "generating grub.cfg inside chroot"
chroot "$MNT" update-grub

if [ -n "$ROOT_PASSWORD" ]; then
  chroot "$MNT" /bin/bash -c "echo root:${ROOT_PASSWORD} | chpasswd && passwd -u root"
fi

log "unmounting"
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
  echo "drmd desktop VM image credentials -- generated $(date -u +%FT%TZ)"
  echo "user: ${DESKTOP_USER}  password: ${ADMIN_PASSWORD}  (auto-logs in graphically; this password is for sudo/ssh)"
  echo "root account is password-locked; use 'sudo' from the ${DESKTOP_USER} user, or set"
  echo "DRMD_IMG_ROOT_PASSWORD before building to enable a root password."
  echo "Change this password on first login. This file is not committed to git."
} > "$CRED_FILE"
chmod 600 "$CRED_FILE"

log "done: $OUT"
log "credentials written to: $CRED_FILE"
log "boot with, e.g. (graphical -- needs a local display or VNC):"
log "  qemu-system-x86_64 -m 2048 -smp 2 -drive file=$OUT,if=virtio -net nic,model=virtio -net user,hostfwd=tcp::2222-:22 -vga std"
log "boot headless (serial console only, for the smoke test):"
log "  qemu-system-x86_64 -m 2048 -smp 2 -drive file=$OUT,if=virtio -net nic,model=virtio -net user -display none -serial mon:stdio"
