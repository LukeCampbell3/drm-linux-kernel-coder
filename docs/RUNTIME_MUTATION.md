# Goal-driven runtime mutation

`code.evolve` adapts a task program while the task is executing. It does not use Git, commits, branches, patches, or a repository-level workflow.

The loop is:

1. Run the current program against executable goal cases.
2. Derive bounded one-change candidate programs.
3. Execute and score each candidate in an isolated Python process.
4. Atomically retain only a strict score improvement.
5. Continue from the improved program until the goal is met or the search budget is exhausted.

A goal file lives inside the DRM instance work directory:

```text
source=tasks/retry_policy.py
max_candidates=256
timeout_ms=1000
case=one|1\n|1
case=target|3\n|3
case=cap|5\n|3
```

Submit it as a normal developmental episode:

```bash
drmd submit --task adapt_retry_policy \
  --ops code.evolve,fs.write \
  --source goals/retry_policy.drm \
  --output reports/retry_policy.json
```

The resulting JSON records initial and final goal scores, candidates evaluated, mutations committed, and elapsed time. Aggregate daemon metrics expose `mutation_candidates` and `mutations_committed`.

## Guardrails

- Goal manifests and source files are confined to the instance work directory.
- Absolute paths, parent traversal, and source symlinks are rejected.
- Source size, candidate count, per-case execution time, and manifest values are bounded.
- Programs run as `python3 -I`, with piped input and no inherited Python module path.
- Candidate files replace the live task program atomically.
- A candidate must strictly improve held-out executable goal cases before it survives.
- The last accepted program is restored after every rejected candidate.

## Scope

The current mutation grammar covers comparison operators, arithmetic operators, and numeric constants. This is a real execution-and-feedback agent loop, but it is a bounded program-repair baseline—not yet a general replacement for an LLM coding agent. Broader agent competition requires learned or model-proposed structural mutations under the same scorer and commit rules.

Run `drmd agent-bench --out results/agent-bench` to execute the included measurable tasks and produce `agentic_metrics.csv` plus `agentic_summary.md`.
