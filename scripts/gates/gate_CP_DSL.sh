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
  [[ -x tools/cp_dsl_evidence_packet.py ]]
  [[ -x scripts/test_archive_bootstrap.sh ]]
  echo "PASS gate_CP_DSL.sh self-test"
  exit 0
fi
if [[ -n "$mode" \
  && "$mode" != "--reuse-current-evidence" \
  && "$mode" != "--exact-packet" ]]; then
  echo "usage: scripts/gates/gate_CP_DSL.sh [--reuse-current-evidence|--exact-packet]" >&2
  exit 2
fi

export CARGO_NET_OFFLINE="${CARGO_NET_OFFLINE:-true}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(scripts/local_workers.sh)}"

if [[ "$mode" == "--exact-packet" ]]; then
  reviewed_commit="$(git rev-parse HEAD)"
  reviewed_tree="$(git rev-parse 'HEAD^{tree}')"
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  if git symbolic-ref -q HEAD >/dev/null 2>&1; then
    echo "ERROR: exact CP-DSL packet requires detached HEAD" >&2
    exit 1
  fi
  if [[ -n "$(git status --porcelain)" ]]; then
    echo "ERROR: exact CP-DSL packet requires a clean worktree" >&2
    exit 1
  fi

  evidence_dir="${FORGE_CP_DSL_EVIDENCE_DIR:-$ROOT/reports/gates/CP-DSL/evidence}"
  commands_dir="$evidence_dir/commands"
  mkdir -p "$commands_dir"
  export FORGE_CP_DSL_EVIDENCE_DIR="$evidence_dir"
  {
    echo "started_at=$started_at"
    echo "reviewed_commit=$reviewed_commit"
    echo "reviewed_tree=$reviewed_tree"
    echo "clean=true"
    echo "detached=true"
    echo "github_actions_used=false"
  } >"$commands_dir/00-preflight.log"

  run_logged() {
    local label="$1"
    shift
    local log="$commands_dir/$label.log"
    local command_started
    local command_finished
    local exit_code
    command_started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    {
      echo "started_at=$command_started"
      printf 'command='
      printf '%q ' "$@"
      printf '\n--- output ---\n'
    } >"$log"
    set +e
    "$@" >>"$log" 2>&1
    exit_code=$?
    set -e
    command_finished="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    {
      echo
      echo "--- result ---"
      echo "finished_at=$command_finished"
      echo "exit_code=$exit_code"
    } >>"$log"
    if ((exit_code != 0)); then
      tail -n 80 "$log" >&2
      return "$exit_code"
    fi
  }

  run_logged 01-fmt-workspace cargo fmt --all -- --check
  run_logged 02-fmt-fuzz cargo fmt --manifest-path fuzz/Cargo.toml --all -- --check
  run_logged 03-clippy cargo clippy --locked --offline \
    -p forge-carddef -p forge-cardc -p forge-cards -p forge-porttools -p forge-arena \
    --all-targets --all-features -- -D warnings
  run_logged 04-tests cargo test --locked --offline --quiet \
    -p forge-carddef -p forge-cardc -p forge-cards -p forge-porttools -p forge-arena
  run_logged 05-deny cargo deny --offline --locked check licenses bans sources
  run_logged 06-platforms python3 tools/local_platform_metrics.py
  run_logged 07-fuzz scripts/fuzz_local_parallel.sh gate
  run_logged 08-mutation python3 tools/run_cp_dsl_mutation.py
  run_logged 09-card-regression scripts/card_regression.sh --gate
  run_logged 10-platform-validate python3 tools/local_platform_metrics.py --validate-only
  run_logged 11-oracle-semantics python3 tools/oracle_semantic_metrics.py --check
  run_logged 15-local-verify scripts/local_verify.sh task
  run_logged 12-cp-dsl-metrics python3 tools/cp_dsl_metrics.py --check
  run_logged 13-bootstrap scripts/bootstrap_toolchain.sh --check
  run_logged 14-archive-bootstrap scripts/test_archive_bootstrap.sh
  python3 tools/cp_dsl_evidence_packet.py --create \
    --evidence-dir "$evidence_dir" \
    --reviewed-commit "$reviewed_commit" \
    --started-at "$started_at"
  python3 tools/cp_dsl_evidence_packet.py --check --evidence-dir "$evidence_dir"
  echo "PASS CP-DSL exact local evidence packet commit=$reviewed_commit"
  exit 0
fi

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
scripts/check_coverage.sh
python3 tools/cp_dsl_metrics.py --check
scripts/bootstrap_toolchain.sh --check
scripts/test_archive_bootstrap.sh
if [[ "$mode" == "--reuse-current-evidence" ]]; then
  python3 tools/cp_dsl_evidence_packet.py --check
fi

echo "PASS CP-DSL local gate"
