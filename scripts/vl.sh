#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  [[ -x "$ROOT/scripts/check_coverage.sh" ]]
  [[ -x "$ROOT/scripts/run_oracle_subset.sh" ]]
  [[ -x "$ROOT/scripts/perf_smoke.sh" ]]
  echo "PASS vl.sh self-test"
  exit 0
fi

step() {
  local name="$1"
  shift
  echo "==> $name"
  "$@"
}

skip() {
  echo "SKIP: $*"
}

if [[ ! -f Cargo.toml ]]; then
  skip "no Cargo.toml; Rust workspace checks are not active yet"
elif ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: Cargo.toml exists but cargo is not available" >&2
  exit 1
else
  step "cargo fmt" cargo fmt --all -- --check
  step "cargo clippy" cargo clippy --workspace --all-targets --all-features -- -D warnings
  step "cargo build" cargo build --workspace --all-targets
  step "cargo test" cargo test --workspace --quiet
  step "cargo slow tests" cargo test --workspace --quiet --release -- --ignored slow
fi

step "coverage floor" "$ROOT/scripts/check_coverage.sh" 80
step "oracle subset" "$ROOT/scripts/run_oracle_subset.sh"
step "perf smoke" "$ROOT/scripts/perf_smoke.sh"
echo "ALL CHECKS PASSED"
