#!/usr/bin/env bash
set -euo pipefail

export LC_ALL=C
export LANG=C

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ "$#" -eq 0 ]]; then
  echo "usage: scripts/verify_lfs_hydration.sh FILE [...]" >&2
  exit 2
fi

if command -v shasum >/dev/null 2>&1; then
  sha256_file() { shasum -a 256 "$1" | awk '{print $1}'; }
elif command -v sha256sum >/dev/null 2>&1; then
  sha256_file() { sha256sum "$1" | awk '{print $1}'; }
else
  echo "ERROR: replay integrity requires shasum or sha256sum" >&2
  exit 1
fi

for file in "$@"; do
  [[ -f "$file" ]] || {
    echo "ERROR: missing Git LFS artifact: $file" >&2
    exit 1
  }
  pointer="$(git show "HEAD:$file" 2>/dev/null)" || {
    echo "ERROR: $file is not committed at HEAD" >&2
    exit 1
  }
  expected_oid="$(printf '%s\n' "$pointer" | awk -F: '/^oid sha256:/{print $2}')"
  expected_size="$(printf '%s\n' "$pointer" | awk '/^size [0-9]+$/{print $2}')"
  if [[ "$(printf '%s\n' "$pointer" | head -n 1)" != "version https://git-lfs.github.com/spec/v1" \
      || ! "$expected_oid" =~ ^[0-9a-f]{64}$ \
      || ! "$expected_size" =~ ^[0-9]+$ ]]; then
    echo "ERROR: committed $file is not a valid Git LFS pointer" >&2
    exit 1
  fi
  if [[ "$(head -n 1 "$file")" == "version https://git-lfs.github.com/spec/v1" ]]; then
    echo "ERROR: $file is an unhydrated Git LFS pointer" >&2
    echo "Run: git lfs install && git lfs pull && git lfs fsck" >&2
    exit 1
  fi
  actual_size="$(wc -c < "$file" | tr -d '[:space:]')"
  actual_oid="$(sha256_file "$file")"
  if [[ "$actual_size" != "$expected_size" || "$actual_oid" != "$expected_oid" ]]; then
    echo "ERROR: Git LFS integrity mismatch for $file" >&2
    echo "Expected sha256:$expected_oid size $expected_size; found sha256:$actual_oid size $actual_size" >&2
    exit 1
  fi
  echo "PASS LFS $file sha256:$actual_oid size $actual_size"
done
