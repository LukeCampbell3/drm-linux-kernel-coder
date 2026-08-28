# GLM user-agent frontend

`drmd assist` uses GLM-5.3-Flash by default to translate a specific natural-language goal into a small, guarded DRM task proposal. `--provider qwen` selects Qwen3.8-Flash for like-for-like evaluation.

The model is not an application operator. Its output is parsed from a four-field protocol, limited to known DRM capabilities, and never executed by `assist`. A proposed `app.execute` still has to pass DRM's independently learned workflow certification and application adapter allowlists. New application work is routed to `task.watch`; ambiguous, consequential, or safeguard-evading goals must be clarified or denied.

```sh
export DRMD_MODEL_ADAPTER=/usr/local/lib/drmd/openai_compatible_frontend.py
export ZAI_API_KEY=...
drmd assist --goal "Use my certified research workflow to summarize this topic"

export DASHSCOPE_API_KEY=...
drmd assist --provider qwen --goal "Use my certified research workflow to summarize this topic"
```

The adapter defaults to `glm-5.3-flash` and `qwen3.8-flash`. Deployments can pin a model or regional endpoint with `DRMD_GLM_MODEL`, `DRMD_GLM_BASE_URL`, `DRMD_QWEN_MODEL`, and `DRMD_QWEN_BASE_URL`. API keys are read only by the adapter and are never put in prompts or DRM state.

## Reproducible comparison

Run both providers over the same versioned goal set and record: valid guarded plans / total, expected decision and family accuracy, unsafe execution attempts rejected, median and p95 latency, input/output tokens, and provider-reported cost. Do not mix Qwen3.8-Flash-Next into hosted latency/cost results until its API is generally available; it is an open-weight architecture target, while `qwen3.8-flash` is the current deployable Flash comparison.
