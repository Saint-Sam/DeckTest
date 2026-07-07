#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

TARGET="${FORGE_CORE_LIB:-crates/forge-core/src/lib.rs}"
TAB="$(printf '\t')"
TMPDIRS=()

cleanup() {
  local dir
  for dir in "${TMPDIRS[@]}"; do
    rm -rf "$dir"
  done
}
trap cleanup EXIT

make_tmpdir() {
  REPLY="$(mktemp -d "${TMPDIR:-/tmp}/forge_public_mutating_gamestate.XXXXXX")"
  TMPDIRS+=("$REPLY")
}

allowed_methods() {
  # T1.R1 seals public mutation behind the action surface. Read-only queries are
  # outside this check because they do not take &mut self.
  printf '%s\n' apply
}

allowed_free_functions() {
  printf '%s\n' apply
}

scan_public_mutating_methods() {
  local file="$1"
  awk -v tab="$TAB" '
    function brace_delta(s,    t, opens, closes) {
      t = s
      opens = gsub(/\{/, "{", t)
      t = s
      closes = gsub(/\}/, "}", t)
      return opens - closes
    }

    function normalize(s) {
      gsub(/[[:space:]]+/, " ", s)
      sub(/^[[:space:]]+/, "", s)
      sub(/[[:space:]]+$/, "", s)
      return s
    }

    function method_name(s,    name) {
      name = s
      sub(/^.*pub[[:space:]]+/, "", name)
      sub(/^((const|async|unsafe)[[:space:]]+)*/, "", name)
      sub(/^fn[[:space:]]+/, "", name)
      sub(/\(.*/, "", name)
      sub(/[[:space:]].*/, "", name)
      return name
    }

    function report_if_mutating(name, line, sig,    normalized) {
      normalized = normalize(sig)
      if (normalized ~ /&[[:space:]]*mut[[:space:]]+self/) {
        print line tab name tab normalized
      }
    }

    /^[[:space:]]*impl[[:space:]]+GameState[[:space:]]*\{/ {
      in_impl = 1
      depth = brace_delta($0)
      next
    }

    in_impl {
      if (!in_sig && $0 ~ /^[[:space:]]*pub[[:space:]]+((const|async|unsafe)[[:space:]]+)*fn[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\(/) {
        in_sig = 1
        sig = $0
        start_line = NR
        name = method_name($0)
      } else if (in_sig) {
        sig = sig " " $0
      }

      if (in_sig && $0 ~ /\{/) {
        report_if_mutating(name, start_line, sig)
        in_sig = 0
        sig = ""
      }

      depth += brace_delta($0)
      if (depth <= 0) {
        in_impl = 0
        in_sig = 0
        sig = ""
      }
    }
  ' "$file"
}

scan_public_mutating_free_functions() {
  local file="$1"
  awk -v tab="$TAB" '
    function brace_delta(s,    t, opens, closes) {
      t = s
      opens = gsub(/\{/, "{", t)
      t = s
      closes = gsub(/\}/, "}", t)
      return opens - closes
    }

    function normalize(s) {
      gsub(/[[:space:]]+/, " ", s)
      sub(/^[[:space:]]+/, "", s)
      sub(/[[:space:]]+$/, "", s)
      return s
    }

    function function_name(s,    name) {
      name = s
      sub(/^.*pub[[:space:]]+/, "", name)
      sub(/^((const|async|unsafe)[[:space:]]+)*/, "", name)
      sub(/^fn[[:space:]]+/, "", name)
      sub(/\(.*/, "", name)
      sub(/[[:space:]].*/, "", name)
      return name
    }

    function report_if_mutating(name, line, sig,    normalized) {
      normalized = normalize(sig)
      if (normalized ~ /&[[:space:]]*mut[[:space:]]+GameState/) {
        print line tab name tab normalized
      }
    }

    {
      if (!in_sig && depth == 0 && $0 ~ /^[[:space:]]*pub[[:space:]]+((const|async|unsafe)[[:space:]]+)*fn[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\(/) {
        in_sig = 1
        sig = $0
        start_line = NR
        name = function_name($0)
      } else if (in_sig) {
        sig = sig " " $0
      }

      if (in_sig && $0 ~ /\{/) {
        report_if_mutating(name, start_line, sig)
        in_sig = 0
        sig = ""
      }

      depth += brace_delta($0)
    }
  ' "$file"
}

run_check() {
  local file="$1"
  local quiet="${2:-}"
  local tmpdir allowed_methods_file allowed_functions_file method_matches method_violations function_matches function_violations

  if [[ ! -f "$file" ]]; then
    echo "SKIP: $file is absent; GameState mutator review is not active yet"
    return 0
  fi

  make_tmpdir
  tmpdir="$REPLY"
  allowed_methods_file="$tmpdir/allowed-methods"
  allowed_functions_file="$tmpdir/allowed-functions"
  method_matches="$tmpdir/method-matches"
  method_violations="$tmpdir/method-violations"
  function_matches="$tmpdir/function-matches"
  function_violations="$tmpdir/function-violations"

  allowed_methods | sort -u >"$allowed_methods_file"
  allowed_free_functions | sort -u >"$allowed_functions_file"
  scan_public_mutating_methods "$file" >"$method_matches"
  scan_public_mutating_free_functions "$file" >"$function_matches"
  awk -F "$TAB" 'NR == FNR { allowed[$1] = 1; next } !($2 in allowed) { print }' \
    "$allowed_methods_file" "$method_matches" >"$method_violations"
  awk -F "$TAB" 'NR == FNR { allowed[$1] = 1; next } !($2 in allowed) { print }' \
    "$allowed_functions_file" "$function_matches" >"$function_violations"

  failed=0
  if [[ -s "$method_violations" ]]; then
    echo "ERROR: public GameState methods taking &mut self outside the action-surface allowlist:" >&2
    while IFS="$TAB" read -r line name signature; do
      printf '  %s:%s: %s\n' "$file" "$line" "$name" >&2
      printf '    %s\n' "$signature" >&2
    done <"$method_violations"
    echo "Allowed public mutating GameState methods:" >&2
    sed 's/^/  /' "$allowed_methods_file" >&2
    echo "Move low-level mutators behind private/pub(crate)/test-only APIs or route them through apply." >&2
    failed=1
  fi

  if [[ -s "$function_violations" ]]; then
    echo "ERROR: public free functions taking &mut GameState outside the action-surface allowlist:" >&2
    while IFS="$TAB" read -r line name signature; do
      printf '  %s:%s: %s\n' "$file" "$line" "$name" >&2
      printf '    %s\n' "$signature" >&2
    done <"$function_violations"
    echo "Allowed public free functions taking &mut GameState:" >&2
    sed 's/^/  /' "$allowed_functions_file" >&2
    echo "Route external mutation through apply only." >&2
    failed=1
  fi

  if [[ "$failed" -ne 0 ]]; then
    return 1
  fi

  if [[ "$quiet" != "--quiet" ]]; then
    echo "PASS no_public_mutating_gamestate.sh"
  fi
}

self_test() {
  local tmpdir ok_fixture bad_fixture bad_output
  make_tmpdir
  tmpdir="$REPLY"
  ok_fixture="$tmpdir/ok.rs"
  bad_fixture="$tmpdir/bad.rs"
  bad_output="$tmpdir/bad.out"

  cat >"$ok_fixture" <<'RS'
pub struct GameState;

pub fn apply(_state: &mut GameState, _action: ()) -> Result<(), ()> {
    Ok(())
}

pub fn query(_state: &GameState) -> bool {
    true
}

impl GameState {
    pub(crate) fn setup_only(&mut self) {}

    fn private_helper(&mut self) {}

    pub fn query(&self) -> bool {
        true
    }
}

impl NotGameState {
    pub fn public_mutator(&mut self) {}
}
RS

  cat >"$bad_fixture" <<'RS'
pub struct GameState;

pub fn mutate_state(_state: &mut GameState) {}

impl GameState {
    pub fn apply(&mut self, _action: ()) -> Result<(), ()> {
        Ok(())
    }

    pub fn set_life(
        &mut self,
        _life: i32,
    ) -> Result<(), ()> {
        Ok(())
    }
}
RS

  run_check "$ok_fixture" --quiet
  if run_check "$bad_fixture" --quiet >"$bad_output" 2>&1; then
    echo "ERROR: self-test fixture with set_life unexpectedly passed" >&2
    return 1
  fi
  if ! grep -q 'set_life' "$bad_output"; then
    echo "ERROR: self-test did not report the disallowed method name" >&2
    cat "$bad_output" >&2
    return 1
  fi
  if ! grep -q 'mutate_state' "$bad_output"; then
    echo "ERROR: self-test did not report the disallowed free function name" >&2
    cat "$bad_output" >&2
    return 1
  fi

  echo "PASS no_public_mutating_gamestate.sh self-test"
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit 0
fi

run_check "$TARGET"
