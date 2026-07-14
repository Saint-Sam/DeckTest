#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  [[ -f docs/specs/T1.R10.md ]]
  [[ -f reports/owner/brief-CP-HUMAN-PLAY-CLI.md ]]
  rg -q 'pub fn run_prompted_game' crates/forge-game-runner/src/lib.rs
  echo "PASS gate_T1_R10.sh self-test"
  exit 0
fi

if [[ -n "${1:-}" ]]; then
  echo "usage: scripts/gates/gate_T1_R10.sh [--self-test]" >&2
  exit 2
fi

for command in cargo jq; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "ERROR: $command is required" >&2
    exit 1
  fi
done

if [[ ! -f target/translated-cards/i/isamaru_hound_of_konda.frs ]]; then
  echo "ERROR: local T3 translated cards are absent" >&2
  echo "Run: scripts/t3_parallel_sweep.sh development" >&2
  exit 1
fi

export CARGO_NET_OFFLINE=true
cargo fmt --all -- --check
cargo test -p forge-game-runner --lib tests --locked --offline
cargo test -p forge-game-runner --lib \
  tests::scripted_human_game_completes_and_replays_exactly \
  --locked --offline -- --ignored --exact
cargo run -p forge-cli --locked --offline -- \
  replay target/t1-r10/scripted-human.frsreplay

jq -e '
  .format == "forge-human-play-replay-v1" and
  .human_seat == 0 and
  (.decisions | length) > 0 and
  (.actions | length) > 0 and
  .expected.turns > 0 and
  .expected.final_hash > 0
' target/t1-r10/scripted-human.frsreplay >/dev/null

echo "PASS T1.R10 implementation: prompted game and exact replay are locally green"
echo "OWNER INPUT REQUIRED: complete CP-HUMAN-PLAY-CLI before T4 starts"
