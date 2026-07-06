#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS gate_T0.sh self-test"
  exit 0
fi

"$ROOT/scripts/selftest.sh"
"$ROOT/scripts/check_toolchain.sh"
"$ROOT/scripts/vl.sh"

if [[ ! -f metrics/legacy_inventory.json ]]; then
  echo "ERROR: metrics/legacy_inventory.json is required for T0 gate" >&2
  exit 1
fi

if [[ ! -f docs/legacy_inventory.md ]]; then
  echo "ERROR: docs/legacy_inventory.md is required for T0 gate" >&2
  exit 1
fi

if [[ ! -f docs/vendor/comprehensive-rules.txt ]]; then
  echo "ERROR: docs/vendor/comprehensive-rules.txt is required for T0 gate" >&2
  exit 1
fi

if [[ ! -f reports/owner/brief-T0-gate.md ]]; then
  echo "ERROR: reports/owner/brief-T0-gate.md is required for T0 gate" >&2
  exit 1
fi

echo "PASS gate_T0.sh"
