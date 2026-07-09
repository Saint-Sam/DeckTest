#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS gate_T3.sh self-test"
  exit 0
fi

echo "ERROR: gate_T3.sh is fail-closed until the T3 exit criteria are implemented" >&2
exit 1
