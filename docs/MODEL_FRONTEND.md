# GLM user-agent frontend

`drmd assist` uses a locally served GLM-5.3-Flash by default to translate a specific natural-language goal into a small, guarded DRM task proposal. `--provider qwen` selects local Qwen3-Coder-Next as the lower-active-parameter comparison. The bundled adapter accepts loopback endpoints only and contains no cloud API-key path.

The model is not an application operator. Its output is parsed from a four-field protocol, limited to known DRM capabilities, and never executed by `assist`. A proposed `app.execute` still has to pass DRM's independently learned workflow certification and application adapter allowlists. New application work is routed to `task.watch`; ambiguous, consequential, or safeguard-evading goals must be clarified or denied.

```sh
export DRMD_MODEL_ADAPTER=/usr/local/lib/drmd/openai_compatible_frontend.py
vllm serve zai-org/GLM-5.3-Flash --port 8000
drmd assist --goal "Use my certified research workflow to summarize this topic"

vllm serve Qwen/Qwen3-Coder-Next --port 8001
drmd assist --provider qwen --goal "Use my certified research workflow to summarize this topic"
```

GLM-5.3-Flash is a 320B-parameter MoE with 18B active parameters. Qwen3-Coder-Next is an 80B-parameter MoE with 3B active parameters. Sparse activation reduces per-token computation, but every selected deployment still needs storage and a serving strategy for the complete checkpoint. Quantization reduces weight memory; LoRA tuning by itself does not.

## DRM as teacher

DRM remains authoritative for state, permissions, observations, tests, application certification, web policy, commit, and rollback. The local model proposes a bounded task family or mutation; DRM supplies minimal retrieved state, evaluates the proposal, and records the result. Only verified commits are eligible positive tuning examples. Rejected proposals remain negative/preference examples with the verifier reason. Raw browsing data, credentials, and uncommitted model output are excluded.

Training is deliberately offline and versioned. Export a frozen teacher dataset from committed DRM episodes, split it by task and time, train an adapter or distilled student, then admit the candidate only if it improves the locked agentic suite without increasing guardrail violations. The running model never updates its weights directly from a user task.

## Reproducible comparison

Run both local providers over the same versioned goal set and record: valid guarded plans / total, verified task completion, expected decision and family accuracy, unsafe execution attempts rejected, median and p95 time-to-first-token and completion latency, tokens/second, peak RAM/VRAM, active parameters, energy, mutations proposed, commits retained, and rollbacks. Compare cold-start and warm-resident operation separately.
