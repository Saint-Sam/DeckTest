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
source_root="$archive_root/source"
source_tar="$archive_root/source.tar"
reviewed_commit="$(git -C "$ROOT" rev-parse HEAD)"
reviewed_tree="$(git -C "$ROOT" rev-parse 'HEAD^{tree}')"
mkdir -p "$source_root"
git -C "$ROOT" archive --format=tar --output="$source_tar" HEAD
tar -xf "$source_tar" -C "$source_root"

if [[ -e "$source_root/.git" ]]; then
  echo "ERROR: archive simulation unexpectedly contains .git" >&2
  exit 1
fi

FORGE_ROOT="$source_root" "$source_root/scripts/bootstrap_toolchain.sh" --check
FORGE_ROOT="$source_root" \
FORGE_ARCHIVE_SOURCE_COMMIT="$reviewed_commit" \
FORGE_ARCHIVE_SOURCE_TREE="$reviewed_tree" \
CARGO_NET_OFFLINE=true \
  "$source_root/scripts/local_verify.sh" task
echo "PASS source-archive bootstrap and local verification root=$source_root commit=$reviewed_commit"
