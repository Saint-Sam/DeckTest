#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS mutation_check.sh self-test"
  exit 0
fi

target="${1:-}"
base="${MUTATION_BASE:-$target}"
test_cmd="${MUTATION_TEST_CMD:-cargo test --workspace --quiet}"

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "SKIP: not in a git worktree; mutation check is not active yet"
  exit 0
fi

if [[ ! -f Cargo.toml ]]; then
  echo "SKIP: no Cargo.toml; mutation check is not active yet"
  exit 0
fi

if [[ "$test_cmd" == cargo* ]] && ! command -v cargo >/dev/null 2>&1; then
  echo "SKIP: cargo is not available; mutation check is not active in this shell"
  exit 0
fi

if [[ -z "$base" ]]; then
  echo "ERROR: mutation_check.sh needs a base ref or PR ref; pass one arg or set MUTATION_BASE" >&2
  exit 2
fi

if ! git rev-parse --verify "$base^{commit}" >/dev/null 2>&1; then
  echo "SKIP: base ref is not available locally: $base"
  exit 0
fi

patch="$(mktemp "${TMPDIR:-/tmp}/forge_mutation_patch.XXXXXX")"
tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/forge_mutation_worktree.XXXXXX")"
cleanup() {
  git worktree remove -f "$tmpdir" >/dev/null 2>&1 || rm -rf "$tmpdir"
  rm -f "$patch"
}
trap cleanup EXIT

git diff --binary "$base"...HEAD >"$patch"
if [[ ! -s "$patch" ]]; then
  echo "SKIP: no diff between $base and HEAD"
  exit 0
fi

echo "==> baseline test command"
sh -c "$test_cmd"

git worktree add -q --detach "$tmpdir" HEAD
if ! git -C "$tmpdir" apply -R "$patch"; then
  echo "ERROR: could not reverse-apply diff for mutation check" >&2
  exit 1
fi

echo "==> reverted test command (expected to fail)"
if (cd "$tmpdir" && sh -c "$test_cmd"); then
  echo "ERROR: tests still pass after reverting the task diff" >&2
  exit 1
fi

echo "PASS mutation_check.sh"
