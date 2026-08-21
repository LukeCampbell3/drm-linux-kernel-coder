# DRM Near-Machine Vocabulary Prototype

This experiment compiles the existing DRM developmental representation into compact machine-like execution forms while preserving the frozen semantic roots:

- `OBSERVE`
- `DERIVE`
- `COMMIT`

## Encodings

### Explicit O/D/C bytecode (16-bit)

Each semantic phase is emitted as a 16-bit word:

- bits 15:14 — semantic opcode (`OBS`, `DRV`, `CMT`; `11` reserved)
- bits 13:8 — capability id
- bits 7:0 — typed operand/context slot

### Dense short-form micro-op (8-bit, preferred)

Each capability is one byte because each L1 capability has an immutable O/D/C signature:

- bits 7:4 — capability id (0..15)
- bits 3:2 — O/D/C signature
  - `00`: `OBSERVE`
  - `01`: `DERIVE`
  - `10`: `DERIVE -> COMMIT`
  - `11`: `DERIVE -> COMMIT -> OBSERVE`
- bits 1:0 — reserved/version flags

The dense word is rejected if its encoded signature does not match the frozen capability table. Therefore it can always be expanded back to the same O/D/C root sequence.

A production format should reserve one capability id as an escape to an extended capability id rather than widening every common instruction.

## Build

```bash
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build
ctest --test-dir build --output-on-failure
./build/drm_bytecode --out results/run
```

The test executes real local Linux filesystem, process, loopback TCP, Unix-socket IPC, timer, state, and `/proc` workflows. No external network is required.
