#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS run_nightmare_suite.sh self-test"
  exit 0
fi

games="${1:-1000}"
max_turns="${2:-6}"

cargo run -p forge-arena -- --nightmare-suite --games "$games" --max-turns "$max_turns"
