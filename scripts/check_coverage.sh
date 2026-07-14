#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  [[ -x "$ROOT/tools/coverage_summary.py" ]]
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
raw_coverage="target/coverage/coverage.raw.json"
mkdir -p "$(dirname "$raw_coverage")"
cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --no-report
cargo llvm-cov run -p forge-testkit --bin forge-testkit --no-report -- oracle --filter layers --no-junit
cargo llvm-cov run -p forge-testkit --bin forge-testkit --no-report -- oracle --path tests/oracle/t2_5_activated --no-junit
cargo llvm-cov run -p forge-testkit --bin forge-testkit --no-report -- oracle --path tests/oracle/t2_6_targeting --no-junit
cargo llvm-cov run -p forge-testkit --bin forge-testkit --no-report -- oracle --path tests/oracle/t2_7_counters_tokens_copy --no-junit
cargo llvm-cov run -p forge-testkit --bin forge-testkit --no-report -- oracle --path tests/oracle/t2_8_multiplayer_commander --no-junit
cargo llvm-cov run -p forge-testkit --bin forge-testkit --no-report -- oracle --path tests/oracle/t2_9_keyword_wave1 --no-junit

coverage_card_dir="target/coverage-card-regression"
mkdir -p "$coverage_card_dir"
cargo llvm-cov run -p forge-cardc --bin forge-cardc --no-report -- \
  build cards/cp_dsl/definitions \
  --catalog assets/card_catalog.json \
  -o "$coverage_card_dir/carddb.bin"
cargo llvm-cov run -p forge-cards --bin forge-cards --no-report -- \
  validate "$coverage_card_dir/carddb.bin"
cargo llvm-cov run -p forge-porttools --bin forge-porttools --no-report -- \
  quarantine --list --catalog assets/card_catalog.json
cargo llvm-cov run -p forge-arena --bin forge-arena --no-report -- \
  --nightmare-suite --games 10 --max-turns 2
cargo llvm-cov run -p forge-arena --bin forge-arena --no-report -- \
  --smoke 1 --random --max-turns 2

t3_6_paths=()
while IFS= read -r relative_path; do
  t3_6_paths+=("target/translated-cards/$relative_path")
done < <(jq -r '.cases[].translated_path' tests/t3_6/commander_semantic_cases.json)
cargo llvm-cov run -p forge-testkit --bin forge-t3-6-runtime-probe --no-report -- \
  "${t3_6_paths[@]}" >target/coverage/t3_6_runtime_probe.json
cargo llvm-cov run -p forge-testkit --bin forge-t3-9-four-player-pod --no-report -- \
  --games 4 --jobs 4 --output target/coverage/t3_9_four_player_pod.json
cargo llvm-cov report --fail-under-lines "$floor" --json --output-path "$raw_coverage"
python3 tools/coverage_summary.py --raw "$raw_coverage" --floor "$floor"
