#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS gate_T4.sh self-test"
  exit 0
fi

echo "ERROR: gate_T4.sh is unopened and fail-closed" >&2
exit 1
