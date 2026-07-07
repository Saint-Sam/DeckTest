#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS selftest.sh self-test"
  exit 0
fi

scripts=(
  scripts/check_toolchain.sh
  scripts/bootstrap_toolchain.sh
  scripts/selftest.sh
  scripts/vl.sh
  scripts/check_coverage.sh
  scripts/run_oracle.sh
  scripts/run_oracle_subset.sh
  scripts/fuzz_nightly.sh
  scripts/card_regression.sh
  scripts/perf_smoke.sh
  scripts/gates/make_bundle.sh
  scripts/gates/gate_T0.sh
  scripts/gates/gate_T1.sh
  scripts/gates/gate_T2.sh
  scripts/gates/gate_T3.sh
  scripts/gates/gate_T4.sh
  scripts/gates/gate_T5.sh
  scripts/gates/gate_T6.sh
  scripts/gates/gate_T7.sh
  scripts/gates/gate_T8.sh
  scripts/review/mutation_check.sh
  scripts/review/no_public_mutating_gamestate.sh
  scripts/review/clone_surface_guard.sh
  scripts/review/no_unwrap.sh
  scripts/review/no_card_names.sh
  scripts/review/determinism.sh
)

tools=(
  tools/perf_diff.py
  tools/criterion_to_perf.py
  tools/metrics_write.py
)

for script in "${scripts[@]}"; do
  if [[ ! -x "$script" ]]; then
    echo "ERROR: $script is missing or not executable" >&2
    exit 1
  fi
  "$ROOT/$script" --self-test
done

for tool in "${tools[@]}"; do
  if [[ ! -f "$tool" ]]; then
    echo "ERROR: $tool is missing" >&2
    exit 1
  fi
  python3 "$ROOT/$tool" --self-test
done

echo "PASS scripts/selftest.sh"
