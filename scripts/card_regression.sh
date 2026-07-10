#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  [[ -x tools/generate_cp_dsl.py ]]
  [[ -x tools/generate_cp_dsl_negative.py ]]
  [[ -x tools/run_cp_dsl_mutation.py ]]
  [[ -x tools/cp_dsl_metrics.py ]]
  [[ -x tools/oracle_semantic_metrics.py ]]
  echo "PASS card_regression.sh self-test"
  exit 0
fi

update_assets=false
gate=false
while (($#)); do
  case "$1" in
    --update) update_assets=true ;;
    --gate) gate=true ;;
    *)
      echo "usage: scripts/card_regression.sh [--update] [--gate]" >&2
      exit 2
      ;;
  esac
  shift
done

for command in cargo python3 cmp; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "ERROR: $command is required for card regression" >&2
    exit 1
  fi
done
for path in assets/card_catalog.json metrics/card_catalog.json cards/cp_dsl/source_cards.json; do
  if [[ ! -s "$path" ]]; then
    echo "ERROR: required compact card artifact is missing: $path" >&2
    exit 1
  fi
done

workers="$("$ROOT/scripts/local_workers.sh")"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$workers}"
export CARGO_NET_OFFLINE=true

python3 tools/generate_cp_dsl.py --check
python3 tools/generate_cp_dsl_negative.py --check
python3 tools/oracle_semantic_metrics.py --check
cargo build --locked --offline --quiet -p forge-cardc -p forge-cards -p forge-porttools -p forge-arena

build_dir="target/card-regression"
mkdir -p "$build_dir"
for index in 1 2 3; do
  cardc="target/debug/forge-cardc"
  if [[ "$gate" == true ]]; then
    isolated_target="$build_dir/isolated-$index"
    cargo clean --target-dir "$isolated_target" >/dev/null 2>&1
    if ! CARGO_TARGET_DIR="$isolated_target" cargo build \
      --locked --offline --quiet -p forge-cardc \
      >"$build_dir/isolated-build-$index.log" 2>&1; then
      cat "$build_dir/isolated-build-$index.log" >&2
      exit 1
    fi
    cardc="$isolated_target/debug/forge-cardc"
  fi
  "$cardc" build cards/cp_dsl/definitions \
    --catalog assets/card_catalog.json \
    -o "$build_dir/carddb-$index.bin"
  "$cardc" build cards/integration/layers \
    -o "$build_dir/layer-scenarios-$index.bin"
done

cmp -s "$build_dir/carddb-1.bin" "$build_dir/carddb-2.bin"
cmp -s "$build_dir/carddb-1.bin" "$build_dir/carddb-3.bin"
cmp -s "$build_dir/carddb-1.index.json" "$build_dir/carddb-2.index.json"
cmp -s "$build_dir/carddb-1.index.json" "$build_dir/carddb-3.index.json"
cmp -s "$build_dir/layer-scenarios-1.bin" "$build_dir/layer-scenarios-2.bin"
cmp -s "$build_dir/layer-scenarios-1.bin" "$build_dir/layer-scenarios-3.bin"
cmp -s "$build_dir/layer-scenarios-1.index.json" "$build_dir/layer-scenarios-2.index.json"
cmp -s "$build_dir/layer-scenarios-1.index.json" "$build_dir/layer-scenarios-3.index.json"

if [[ "$update_assets" == true ]]; then
  cp "$build_dir/carddb-1.bin" assets/carddb.bin
  cp "$build_dir/carddb-1.index.json" assets/carddb.index.json
  cp "$build_dir/layer-scenarios-1.bin" assets/layer_scenarios.carddb.bin
  cp "$build_dir/layer-scenarios-1.index.json" assets/layer_scenarios.carddb.index.json
else
  cmp -s "$build_dir/carddb-1.bin" assets/carddb.bin || {
    echo "ERROR: assets/carddb.bin is stale; run scripts/card_regression.sh --update" >&2
    exit 1
  }
  cmp -s "$build_dir/carddb-1.index.json" assets/carddb.index.json || {
    echo "ERROR: assets/carddb.index.json is stale; run scripts/card_regression.sh --update" >&2
    exit 1
  }
  cmp -s "$build_dir/layer-scenarios-1.bin" assets/layer_scenarios.carddb.bin || {
    echo "ERROR: assets/layer_scenarios.carddb.bin is stale; run scripts/card_regression.sh --update" >&2
    exit 1
  }
  cmp -s "$build_dir/layer-scenarios-1.index.json" assets/layer_scenarios.carddb.index.json || {
    echo "ERROR: assets/layer_scenarios.carddb.index.json is stale; run scripts/card_regression.sh --update" >&2
    exit 1
  }
fi

target/debug/forge-cards validate "$build_dir/carddb-1.bin"
target/debug/forge-porttools quarantine --list --catalog assets/card_catalog.json
cargo test --quiet -p forge-cardc --test malformed_corpus
target/debug/forge-arena --nightmare-suite \
  --games "${FORGE_CARD_NIGHTMARE_GAMES:-100}" \
  --max-turns "${FORGE_CARD_NIGHTMARE_MAX_TURNS:-4}"
scripts/run_oracle.sh --all

if [[ "$gate" == true ]]; then
  if ! python3 tools/run_cp_dsl_mutation.py --check; then
    python3 tools/run_cp_dsl_mutation.py
  fi
else
  python3 tools/run_cp_dsl_mutation.py --check
fi
python3 tools/cp_dsl_metrics.py

echo "PASS card regression: 100 cards, 25 strata, deterministic database, local-only evidence"
