#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS gate_T1.sh self-test"
  exit 0
fi

echo "SKIP: gate_T1.sh is a Tier 0 stub; fill in Tier 1 exit criteria when that tier opens"
