#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--self-test" ]]; then
  workers="$("$0")"
  [[ "$workers" =~ ^[1-9][0-9]*$ ]]
  echo "PASS local_workers.sh self-test: workers=$workers"
  exit 0
fi

logical_cores=""
if command -v sysctl >/dev/null 2>&1; then
  logical_cores="$(sysctl -n hw.logicalcpu 2>/dev/null || true)"
fi
if [[ ! "$logical_cores" =~ ^[1-9][0-9]*$ ]] && command -v getconf >/dev/null 2>&1; then
  logical_cores="$(getconf _NPROCESSORS_ONLN 2>/dev/null || true)"
fi
if [[ ! "$logical_cores" =~ ^[1-9][0-9]*$ ]]; then
  logical_cores=2
fi

reserve_by_percent=$(((logical_cores + 9) / 10))
reserve=2
if ((reserve_by_percent > reserve)); then
  reserve="$reserve_by_percent"
fi

workers=$((logical_cores - reserve))
if ((workers < 1)); then
  workers=1
fi
if [[ -n "${FORGE_LOCAL_WORKERS:-}" ]]; then
  if [[ ! "$FORGE_LOCAL_WORKERS" =~ ^[1-9][0-9]*$ ]]; then
    echo "ERROR: FORGE_LOCAL_WORKERS must be a positive integer" >&2
    exit 2
  fi
  workers="$FORGE_LOCAL_WORKERS"
fi

printf '%s\n' "$workers"
