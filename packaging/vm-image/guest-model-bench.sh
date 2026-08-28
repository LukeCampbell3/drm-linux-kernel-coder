#!/bin/sh
set -eu

[ "$#" -ge 4 ] || { echo "usage: $0 PROFILE MODEL OUTPUT_CSV -- COMMAND [ARGS...]" >&2; exit 2; }
command -v /usr/bin/time >/dev/null 2>&1 || { echo "/usr/bin/time is required for resource measurements" >&2; exit 1; }
PROFILE=$1
MODEL=$2
OUTPUT=$3
shift 3
[ "$1" = "--" ] || { echo "expected -- before command" >&2; exit 2; }
shift
[ "$#" -gt 0 ] || { echo "benchmark command is required" >&2; exit 2; }

TMP_BASE=${TMPDIR:-/tmp}
METRICS="$TMP_BASE/drm-model-bench-$$.time"
STDOUT="$TMP_BASE/drm-model-bench-$$.out"
STDERR="$TMP_BASE/drm-model-bench-$$.err"
cleanup() { rm -f "$METRICS" "$STDOUT" "$STDERR"; }
trap cleanup EXIT HUP INT TERM

START_NS=$(date +%s%N)
set +e
/usr/bin/time -v -o "$METRICS" "$@" >"$STDOUT" 2>"$STDERR"
STATUS=$?
set -e
END_NS=$(date +%s%N)
WALL_MS=$(( (END_NS - START_NS) / 1000000 ))
RSS_KB=$(awk -F: '/Maximum resident set size/ {gsub(/^[ \t]+/, "", $2); print $2}' "$METRICS")
USER_S=$(awk -F: '/User time/ {gsub(/^[ \t]+/, "", $2); print $2}' "$METRICS")
SYS_S=$(awk -F: '/System time/ {gsub(/^[ \t]+/, "", $2); print $2}' "$METRICS")
[ -n "$RSS_KB" ] || RSS_KB=0
[ -n "$USER_S" ] || USER_S=0
[ -n "$SYS_S" ] || SYS_S=0
mkdir -p "$(dirname "$OUTPUT")"
if [ ! -f "$OUTPUT" ]; then
    echo "profile,model,status,wall_ms,max_rss_kb,user_s,system_s,output_bytes" >"$OUTPUT"
fi
BYTES=$(wc -c <"$STDOUT" | tr -d ' ')
printf '%s,%s,%s,%s,%s,%s,%s,%s\n' "$PROFILE" "$MODEL" "$STATUS" "$WALL_MS" "$RSS_KB" "$USER_S" "$SYS_S" "$BYTES" >>"$OUTPUT"
cat "$STDOUT"
cat "$STDERR" >&2
exit "$STATUS"
