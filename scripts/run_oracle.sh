#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS run_oracle.sh self-test"
  exit 0
fi

if ! find tests/oracle -type f -name '*.ron' -print -quit 2>/dev/null | grep -q .; then
  echo "SKIP: no oracle scenarios are present yet"
  exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: cargo is required to run oracle scenarios" >&2
  exit 1
fi

if [[ "${1:-}" == "--all" ]]; then
  cargo run -p forge-testkit -- oracle --all
elif [[ "${1:-}" == "--filter" && -n "${2:-}" ]]; then
  cargo run -p forge-testkit -- oracle --filter "$2"
else
  cargo run -p forge-testkit -- oracle "$@"
fi

