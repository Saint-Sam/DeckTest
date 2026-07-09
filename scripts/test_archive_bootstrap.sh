#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

if [[ "${1:-}" == "--self-test" ]]; then
  [[ -x "$ROOT/scripts/bootstrap_toolchain.sh" ]]
  [[ -x "$ROOT/scripts/check_toolchain.sh" ]]
  [[ -s "$ROOT/vendor/legacy-forge.reference.json" ]]
  echo "PASS test_archive_bootstrap.sh self-test"
  exit 0
fi
if [[ -n "${1:-}" ]]; then
  echo "usage: scripts/test_archive_bootstrap.sh [--self-test]" >&2
  exit 2
fi

archive_root="$(mktemp -d "${TMPDIR:-/tmp}/forge-archive-bootstrap.XXXXXX")"
mkdir -p "$archive_root/scripts" "$archive_root/vendor"
cp "$ROOT/scripts/bootstrap_toolchain.sh" "$archive_root/scripts/"
cp "$ROOT/scripts/check_toolchain.sh" "$archive_root/scripts/"
cp "$ROOT/vendor/legacy-forge.reference.json" "$archive_root/vendor/"

if [[ -e "$archive_root/.git" ]]; then
  echo "ERROR: archive simulation unexpectedly contains .git" >&2
  exit 1
fi

FORGE_ROOT="$archive_root" "$archive_root/scripts/bootstrap_toolchain.sh" --check
echo "PASS source-archive bootstrap simulation root=$archive_root"
