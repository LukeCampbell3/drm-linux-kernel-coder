# Constrained local-model VM matrix

The matrix measures local LM choices under reproducible 8, 12, 16, and 24 GiB memory ceilings. It separates model feasibility from task quality: fitting in memory is necessary, but DRM admits a profile only after it completes locked tasks without increasing permission or verification failures.

## Environments

| Profile | RAM | vCPU | Model-weight budget | Intended candidates |
|---|---:|---:|---:|---|
| `8gb-cpu` | 8 GiB | 2 | 4.6 GiB | 0.6B, 1.7B, 4B Q4 |
| `12gb-cpu` | 12 GiB | 4 | 7.6 GiB | prior tiers plus 8B Q4 |
| `16gb-cpu` | 16 GiB | 6 | 10.6 GiB | prior tiers plus 14B Q4 |
| `24gb-cpu` | 24 GiB | 8 | 17 GiB | larger contexts, higher quantization, CPU/GPU hybrid |

Budgets reserve memory for the OS, Rust runtimes, tokenizer, KV cache, verifier, browser, and application adapters. They are admission ceilings, not predicted RSS values.

`qwen3-coder-next-q4` and `glm-5.3-flash-q4` deliberately fail this target on complete-checkpoint weight size. Active-parameter count affects compute, not the bytes required for the complete checkpoint. They remain teacher/large-host options; smaller students distilled from verified trajectories are the deployable candidates.

## Launch

```sh
packaging/vm-image/run-profile.sh \
  --image build/drm-desktop.raw \
  --profile 12gb-cpu
```

Add `--gpu 0000:01:00.0` only on a correctly isolated VFIO host. The launcher also supports `--dry-run`, which CI can validate without KVM.

Inside the guest, wrap each native Rust LM invocation:

```sh
packaging/vm-image/guest-model-bench.sh \
  12gb-cpu student-8b-q4 results/resources.csv -- \
  drm-lmd bench --profile student-8b-q4 --suite locked-agentic-v1
```

## Required measurements

Every model/environment combination must report:

- cold and warm load time;
- peak RSS and peak VRAM;
- time to first token, prefill tokens/s, and decode tokens/s;
- KV-cache bytes by context length;
- energy or CPU/GPU time proxy;
- verified task completions per minute;
- proposal acceptance, rollback, clarification, and guardrail-violation rates;
- escalation count and marginal completion gain from escalation;
- bytes and milliseconds per verified completion.

Run at least three independent repetitions per cell. Freeze prompts and expected outcomes before comparison. Mark missing hardware, weights, or runtime support as `NOT_RUN`; never substitute estimated throughput for observed throughput.

## DRM selection rule

DRM filters profiles that exceed current memory, device, permission, or compatibility constraints. Among admissible profiles it minimizes resource cost subject to locked completion and safety floors. An escalation is retained only when its verified completion gain exceeds its incremental latency and memory cost. Results are learned separately per hardware identity, model digest, quantization, adapter digest, task family, and context bucket.
