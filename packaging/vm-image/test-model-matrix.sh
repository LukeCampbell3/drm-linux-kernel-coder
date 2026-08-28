#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
MODELS="$ROOT/vm-profiles/models.tsv"
REPORT="$ROOT/../docs/reports/local-model-requirements.csv"
[ -f "$MODELS" ] && [ -f "$REPORT" ]

awk -F '\t' 'NR > 1 {
    if ($1 == "" || $4 + 0 <= 0 || $5 + 0 <= 0 || $7 + 0 <= 0) exit 1
    if ($5 > $4) exit 1
    seen[$1]++
} END { if (length(seen) < 7) exit 1 }' "$MODELS"

awk -F, 'NR > 1 {
    expected = ($4 + 0 >= $6 + 0) ? "true" : "false"
    if ($7 != expected || $8 != "NOT_RUN") exit 1
    profiles[$1]++
} END {
    if (!("8gb-cpu" in profiles) || !("12gb-cpu" in profiles) ||
        !("16gb-cpu" in profiles) || !("24gb-cpu" in profiles)) exit 1
}' "$REPORT"
echo MODEL_MATRIX_TEST_PASS
