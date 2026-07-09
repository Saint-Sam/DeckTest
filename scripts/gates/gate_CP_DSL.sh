#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

mode="${1:-}"
if [[ "$mode" == "--self-test" ]]; then
  [[ -x scripts/card_regression.sh ]]
  [[ -x scripts/fuzz_local_parallel.sh ]]
  [[ -x tools/cp_dsl_metrics.py ]]
  [[ -x tools/local_platform_metrics.py ]]
  [[ -x tools/oracle_semantic_metrics.py ]]
  echo "PASS gate_CP_DSL.sh self-test"
  exit 0
fi
if [[ -n "$mode" && "$mode" != "--reuse-current-evidence" ]]; then
  echo "usage: scripts/gates/gate_CP_DSL.sh [--reuse-current-evidence]" >&2
  exit 2
fi

export CARGO_NET_OFFLINE="${CARGO_NET_OFFLINE:-true}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(scripts/local_workers.sh)}"

cargo fmt --all -- --check
cargo fmt --manifest-path fuzz/Cargo.toml --all -- --check
cargo clippy --locked --offline \
  -p forge-carddef \
  -p forge-cardc \
  -p forge-cards \
  -p forge-porttools \
  -p forge-arena \
  --all-targets --all-features -- -D warnings
cargo test --locked --offline --quiet \
  -p forge-carddef \
  -p forge-cardc \
  -p forge-cards \
  -p forge-porttools \
  -p forge-arena
cargo deny --offline --locked check licenses bans sources
python3 tools/local_platform_metrics.py

if [[ "$mode" == "--reuse-current-evidence" ]]; then
  python3 tools/run_local_fuzz.py --check --minimum-worker-seconds 2400
  python3 tools/run_cp_dsl_mutation.py --check
  scripts/card_regression.sh
else
  scripts/fuzz_local_parallel.sh gate
  scripts/card_regression.sh --gate
fi
python3 tools/local_platform_metrics.py --validate-only
python3 tools/oracle_semantic_metrics.py --check
python3 tools/cp_dsl_metrics.py --check

echo "PASS CP-DSL local gate"
