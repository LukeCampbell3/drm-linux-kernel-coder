#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
OUT=${1:-"$ROOT/results/product-baseline"}
mkdir -p "$OUT"

cd "$ROOT"
cargo fmt --all -- --check >"$OUT/fmt.log" 2>&1
cargo clippy --workspace --all-targets -- -D warnings >"$OUT/clippy.log" 2>&1
cargo test --workspace >"$OUT/tests.log" 2>&1
cargo build --workspace --release >"$OUT/build.log" 2>&1
"$ROOT/target/release/drmd" selftest >"$OUT/selftest.log"
"$ROOT/target/release/drmd" bench --out "$OUT/frozen" >"$OUT/bench.log"

case "$(uname -s)" in
    Linux) stat -c %s "$ROOT/target/release/drmd" >"$OUT/binary_size_bytes.txt" ;;
    Darwin) stat -f %z "$ROOT/target/release/drmd" >"$OUT/binary_size_bytes.txt" ;;
    *) wc -c <"$ROOT/target/release/drmd" | tr -d ' ' >"$OUT/binary_size_bytes.txt" ;;
esac

TMP_BASE=${TMPDIR:-/tmp}
RUN_DIR="$TMP_BASE/drmd-product-baseline-$$"
mkdir -p "$RUN_DIR"
trap 'rm -rf "$RUN_DIR"' EXIT HUP INT TERM
python3 "$ROOT/scripts/measure_daemon.py" \
    --binary "$ROOT/target/release/drmd" \
    --socket "$RUN_DIR/drmd.sock" \
    --work "$RUN_DIR/work" \
    --count 200 \
    --output "$OUT/daemon_metrics.json"

grep -q SELF_TEST_PASS "$OUT/selftest.log"
grep -q 'episodes=99 success=99 semantic=214 derived=11 recoveries=4 repairs=4 struct=1797' "$OUT/bench.log"
grep -q 'OBSERVE=141 DERIVE=390 COMMIT=230' "$OUT/bench.log"
grep -q '"uniform_vocabulary": true' "$OUT/frozen/summary.json"
echo PRODUCT_BASELINE_PASS
