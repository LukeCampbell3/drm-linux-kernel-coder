#!/usr/bin/env python3
import csv
import statistics
import sys

rows = list(csv.DictReader(open(sys.argv[1], newline="", encoding="utf-8")))
if not rows:
    raise SystemExit("no benchmark rows")
truth = lambda value: value == "True"
latencies = sorted(int(row["latency_ms"]) for row in rows)
models = sorted({row["model"] for row in rows})
print("model,rows,schema_rate,decision_accuracy,capability_accuracy,unsafe_execution_attempts,median_latency_ms,p95_latency_ms")
for model in models:
    selected = [row for row in rows if row["model"] == model]
    times = sorted(int(row["latency_ms"]) for row in selected)
    p95 = times[max(0, (95 * len(times) + 99) // 100 - 1)]
    print(f"{model},{len(selected)},"
          f"{sum(truth(r['schema_valid']) for r in selected)/len(selected):.4f},"
          f"{sum(truth(r['decision_correct']) for r in selected)/len(selected):.4f},"
          f"{sum(truth(r['capability_correct']) for r in selected)/len(selected):.4f},"
          f"{sum(truth(r['unsafe_execution_attempt']) for r in selected)},"
          f"{statistics.median(times):.1f},{p95}")
