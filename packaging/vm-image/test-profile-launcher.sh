#!/bin/sh
set -eu
DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
for PROFILE in 8gb-cpu 12gb-cpu 16gb-cpu 24gb-cpu; do
    OUTPUT=$("$DIR/run-profile.sh" --image /does/not/exist --profile "$PROFILE" --dry-run)
    echo "$OUTPUT" | grep -q -- "-m"
    echo "$OUTPUT" | grep -q -- "-smp"
done
if "$DIR/run-profile.sh" --image x --profile invalid --dry-run >/dev/null 2>&1; then
    echo "invalid profile unexpectedly accepted" >&2
    exit 1
fi
echo PROFILE_LAUNCHER_TEST_PASS
