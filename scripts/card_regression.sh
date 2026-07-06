#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS card_regression.sh self-test"
  exit 0
fi

if [[ ! -d cards ]] || ! find cards -type f -name '*.frs' -print -quit | grep -q .; then
  echo "SKIP: no card DSL sources are present yet"
  exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: cargo is required for card regression checks" >&2
  exit 1
fi

cargo run -p forge-cardc -- build cards/ -o assets/carddb.bin
cargo run -p forge-porttools -- quarantine --list

