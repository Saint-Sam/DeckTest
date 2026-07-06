#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS run_oracle_subset.sh self-test"
  exit 0
fi

filter="${1:-}"

if [[ ! -f Cargo.toml ]]; then
  echo "SKIP: no Cargo.toml; oracle subset is not active yet"
  exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: cargo is required for oracle subset checks" >&2
  exit 1
fi

if [[ ! -d tests/oracle ]]; then
  echo "SKIP: tests/oracle is absent; no oracle scenarios to run"
  exit 0
fi

if ! find tests/oracle -type f -name '*.ron' -print -quit | grep -q .; then
  echo "SKIP: no oracle scenarios are present yet"
  exit 0
fi

if [[ -n "$filter" ]]; then
  cargo run -p forge-testkit -- oracle --filter "$filter"
else
  cargo run -p forge-testkit -- oracle --changed
fi
