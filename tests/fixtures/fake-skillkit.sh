#!/bin/sh
# Fake skillkit binary for unit tests.
# Usage variants controlled by environment variables:
#
#   FAKE_SKILLKIT_EXIT=0    (default) exit 0 with JSON output
#   FAKE_SKILLKIT_EXIT=1    exit 1 with error message to stderr
#   FAKE_SKILLKIT_EXIT=2    exit 2 with "unknown flag" to stderr (--json unavailable)
#   FAKE_SKILLKIT_SLEEP=N   sleep N seconds before responding (timeout testing)
#   FAKE_SKILLKIT_JSON=...  override the JSON output (default: empty array [])
#
# Called as: sh fake-skillkit.sh team install --manifest <path> [--json]

EXIT_CODE="${FAKE_SKILLKIT_EXIT:-0}"
SLEEP_SECS="${FAKE_SKILLKIT_SLEEP:-0}"
OUTPUT="${FAKE_SKILLKIT_JSON:-[]}"

if [ "$SLEEP_SECS" -gt 0 ] 2>/dev/null; then
    sleep "$SLEEP_SECS"
fi

if [ "$EXIT_CODE" = "2" ]; then
    echo "unknown flag --json" >&2
    exit 2
fi

if [ "$EXIT_CODE" != "0" ]; then
    echo "skillkit: error deploying skills" >&2
    exit "$EXIT_CODE"
fi

echo "$OUTPUT"
exit 0
