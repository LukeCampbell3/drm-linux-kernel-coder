# drmd -- the DRM O/D/C developmental runtime

`drmd` learns a compressed vocabulary of recurring workflow motifs from a
stream of task episodes, and executes each episode for real against the
host: atomic file writes, `/proc` observation, loopback sockets, spawned
processes. Every capability it knows about -- and every abstraction it
later learns -- reduces to exactly three root tokens: **OBSERVE**,
**DERIVE**, **COMMIT**. See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
for how it works.

This is a production consolidation of eight research prototypes (see
[`historical/`](historical/README.md)) into one tested, packaged product:
a Rust workspace with **zero external dependencies**, a Docker image, a
hardened systemd service, and a bootable Linux VM image.

## Quick start

```bash
cargo build --release --workspace
./target/release/drmd selftest                 # fast invariant check
./target/release/drmd bench --out /tmp/results  # frozen regression workload
```

`bench` reproduces the deterministic values documented across every
historical prototype's `PEER_PROTOCOL.md` exactly:

```
episodes=99 success=99 semantic=214 derived=11 recoveries=4 repairs=4 struct=1797 dl_reduction=0.567766
root_counts: OBSERVE=141 DERIVE=390 COMMIT=230
```

## Run it as a service

```bash
./target/release/drmd serve                     # binds /run/drmd/drmd.sock, uses /var/lib/drmd
./target/release/drmd status
./target/release/drmd submit \
    --task daily_report \
    --ops fs.read,transform.summarize,fs.write,notify.send \
    --source inputs/report.csv
```

On the desktop image, DRM instances can also browse JavaScript-rendered public pages through Selenium:

```bash
drmd submit --task web_research --ops web.selenium,transform.summarize,fs.write \
  --url https://www.rust-lang.org/learn --output outputs/research.txt
```

See [`docs/SELENIUM_WEB.md`](docs/SELENIUM_WEB.md) for host allowlists, Selenium Grid support, limits, and isolation behavior.

DRM instances can adapt task programs during execution through `code.evolve`: run executable goals, derive bounded mutations, retain only strict improvements, and continue from the improved live program without Git. `drmd agent-bench` measures the loop against static programs on three code-repair tasks. See [`docs/RUNTIME_MUTATION.md`](docs/RUNTIME_MUTATION.md).

For web and application suites, `task.watch` learns from successful user/application traces in shadow mode, while `app.execute` runs only independently supported, certified workflows through operator-allowlisted adapters. `drmd suite-bench` measures action and duration efficiency over successive observations without exposing live applications to exploratory mutations. See [`docs/OBSERVE_FIRST_APPS.md`](docs/OBSERVE_FIRST_APPS.md).

`drmd assist` adds a loopback-only GLM-5.3-Flash user-agent frontend for specific natural-language goals, with the 3B-active Qwen3-Coder-Next as the efficient local comparison. Models can only propose bounded task manifests; DRM retains state, certification, application allowlists, web policy, verification, and execution authority. See [`docs/MODEL_FRONTEND.md`](docs/MODEL_FRONTEND.md).

The constrained local-model matrix supplies reproducible 8/12/16/24 GiB QEMU environments, model feasibility declarations, and a guest resource-measurement harness. DRM compares multiple quantizations, sizes, devices, adapters, and context budgets by verified completion efficiency rather than model size alone. See [`docs/LOCAL_MODEL_VM_MATRIX.md`](docs/LOCAL_MODEL_VM_MATRIX.md).

`serve` uses the two-tier `HybridPlanner` (a fast-forming, capped
*provisional* vocabulary alongside the conservative *permanent* one) with
deferred background consolidation, so submitting an episode is never
blocked on vocabulary maintenance. See `docs/ARCHITECTURE.md#why-deferred-consolidation`.

## Run it as a container

```bash
docker build -f packaging/docker/Dockerfile -t drmd:latest .
docker run -d --name drmd -v drmd-state:/var/lib/drmd drmd:latest
docker exec drmd drmd status --socket /run/drmd/drmd.sock
```

Or `docker compose -f packaging/docker/compose.yaml up --build`, which
runs the container read-only, with all capabilities dropped and
`no-new-privileges`.

## Run it as a VM / Linux distribution image

```bash
cargo build --release --workspace
sudo packaging/vm-image/build-image.sh --out dist/drmd.qcow2
```

This builds a minimal, bootable Debian-based disk image with `drmd`
installed and enabled as a systemd service from first boot -- a small,
purpose-built Linux distribution whose only job is running this runtime.
It needs root (loop devices, mount, chroot) and normal internet access
(to fetch base packages from a Debian mirror); everything else is
scripted. Login credentials for the generated admin account are written
next to the image, in `dist/drmd.credentials.txt` (not committed to git,
and not printed to the terminal).

Boot it:

```bash
qemu-system-x86_64 -m 1024 -smp 2 \
    -drive file=dist/drmd.qcow2,if=virtio \
    -net nic,model=virtio -net user,hostfwd=tcp::2222-:22 \
    -nographic
```

Then, from another terminal: `ssh -p 2222 drm@localhost` and
`drmd status --socket /run/drmd/drmd.sock`, or `systemctl status drmd`.
The same qcow2 image works unmodified on any BIOS-boot-compatible
hypervisor (libvirt/KVM, VirtualBox after `qemu-img convert -O vdi`,
most cloud platforms that accept a raw/qcow2 disk import).

## Development

```bash
cargo build --workspace          # crates/drm-core, crates/drm-exec, crates/drmd
cargo test --workspace           # 25 tests: unit + real socket/process integration + e2e daemon + regression
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

No crate in this workspace depends on anything outside the Rust standard
library -- `cargo build` never touches the network. See
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the crate layout and
design rationale, and [`CHANGELOG.md`](CHANGELOG.md) for what changed in
this consolidation.

## License

[MIT](LICENSE)
