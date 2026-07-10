#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

legacy_repo="${FORGE_LEGACY_REPO:-vendor/legacy-forge}"
cards_root="${FORGE_LEGACY_CARDS_ROOT:-$legacy_repo/forge-gui/res/cardsfolder}"
metrics="metrics/legacy_parse.json"
failures="metrics/legacy_parse_failures.json"

if [[ ! -d "$cards_root" ]]; then
  echo "ERROR: local legacy cards root is missing: $cards_root" >&2
  exit 1
fi

if [[ -n "$(git -C "$legacy_repo" status --porcelain)" ]]; then
  echo "ERROR: local legacy Forge source has uncommitted changes" >&2
  exit 1
fi

export CARGO_NET_OFFLINE=true
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(scripts/local_workers.sh)}"

cargo fmt --all -- --check
cargo clippy --locked --offline -p forge-porttools --all-targets -- -D warnings
cargo test --locked --offline -p forge-porttools
cargo run --locked --offline -p forge-porttools -- \
  legacy parse \
  --root "$cards_root" \
  --metrics "$metrics" \
  --failures "$failures"
scripts/card_regression.sh

git ls-files --error-unmatch "$metrics" "$failures" >/dev/null
git diff --exit-code -- "$metrics" "$failures"

echo "PASS gate_T3_2.sh"
