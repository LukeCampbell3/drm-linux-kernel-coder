# Observe-first web and application learning

The application layer learns from completed work before it is allowed to act. This avoids using the user's live browser, documents, spreadsheets, or other applications as a trial-and-error environment.

## Lifecycle

1. `task.watch` records a completed workflow trace: task family, independent run ID, outcome, duration, user interventions, and ordered application actions.
2. The shadow learner groups successful traces and evaluates candidate workflows without replaying them against live applications.
3. A workflow requires at least three independent successful observations and at least 90% observed success for the exact sequence.
4. Among admissible candidates, the learner certifies the shortest workflow, breaking ties by median duration.
5. A later workflow replaces the certified one only when it uses fewer actions, or has equal actions and lower median duration.
6. `app.execute` refuses every task family without a certified policy.

## Trace example

```text
run_id=research-004
family=web_research_to_notes
success=true
duration_ms=620
interventions=0
action=browser|navigate|https://example.com/article|
action=browser|extract|article|
action=notes|write|research|summary
```

```bash
drmd submit --task observe_research --ops task.watch \
  --source traces/research-004.trace

drmd submit --task reuse_research --ops app.execute \
  --source web_research_to_notes
```

## Application adapters

Application actions use operator-installed executable adapters. Set `DRMD_APP_ADAPTER_DIR` to the directory and `DRMD_APP_ALLOWED` to a comma-separated application allowlist. Adapter filenames exactly match application names. Each receives three direct arguments: verb, target, and value. No shell evaluates task content.

The existing Selenium bridge can back a `browser` adapter; accessibility or application-native automation can back `notes`, `spreadsheet`, `files`, and other suite adapters.

Risky verbs (`delete`, `purchase`, `send`, `submit`, and `authenticate`) are denied unless the operator explicitly sets `DRMD_APP_ALLOW_RISKY=1`. Adapters must be regular, non-symlinked files.

## Longitudinal evidence

Every observation appends one row to `watch-state/longitudinal_metrics.csv`, including cumulative observation number, family success rate, mean actions, mean duration, mean interventions, shadow evaluations, certification state, and certified action count.

Run `drmd suite-bench --out results/suite-bench` for the frozen browser/notes and spreadsheet/files benchmark. It compares already-successful initial workflows with policies learned purely by watching later successful executions.
