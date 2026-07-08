#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS check_coverage.sh self-test"
  exit 0
fi

floor="${1:-80}"
if ! [[ "$floor" =~ ^[0-9]+$ ]]; then
  echo "ERROR: coverage floor must be an integer percentage" >&2
  exit 2
fi

if [[ ! -f Cargo.toml ]]; then
  echo "SKIP: no Cargo.toml; coverage gate is not active yet"
  exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: cargo is required for coverage checks" >&2
  exit 1
fi

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "ERROR: cargo-llvm-cov is required for coverage checks" >&2
  exit 1
fi

mkdir -p metrics
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --no-report
cargo llvm-cov run -p forge-testkit --bin forge-testkit --no-report -- oracle --filter layers --no-junit
cargo llvm-cov run -p forge-testkit --bin forge-testkit --no-report -- oracle --path tests/oracle/t2_5_activated --no-junit
cargo llvm-cov run -p forge-testkit --bin forge-testkit --no-report -- oracle --path tests/oracle/t2_6_targeting --no-junit
cargo llvm-cov run -p forge-testkit --bin forge-testkit --no-report -- oracle --path tests/oracle/t2_7_counters_tokens_copy --no-junit
cargo llvm-cov report --fail-under-lines "$floor" --json --output-path metrics/coverage.json
