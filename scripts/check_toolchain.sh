#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS check_toolchain.sh self-test"
  exit 0
fi

write_lock=0
if [[ "${1:-}" == "--write-lock" ]]; then
  write_lock=1
elif [[ -n "${1:-}" ]]; then
  echo "usage: scripts/check_toolchain.sh [--write-lock|--self-test]" >&2
  exit 2
fi

stable_toolchain="1.96.1"
nightly_toolchain="nightly-2026-07-05"

required_commands=(rustup rustc cargo rustfmt cargo-clippy)
required_cargo_bins=(
  cargo-llvm-cov
  cargo-fuzz
  cargo-deny
  cargo-audit
  wasm-bindgen
  cargo-ndk
  critcmp
)
version_commands=(
  "cargo llvm-cov --version"
  "cargo fuzz --version"
  "cargo deny --version"
  "cargo audit --version"
  "wasm-bindgen --version"
  "cargo ndk --version"
  "critcmp --version"
)
required_targets=(
  wasm32-unknown-unknown
  aarch64-linux-android
  aarch64-apple-ios
  x86_64-pc-windows-msvc
)

required_versions=(
  "cargo llvm-cov --version|cargo-llvm-cov 0.8.7"
  "cargo fuzz --version|cargo-fuzz 0.13.2"
  "cargo deny --version|cargo-deny 0.19.9"
  "cargo audit --version|0.22.2"
  "wasm-bindgen --version|wasm-bindgen 0.2.126"
  "cargo ndk --version|cargo-ndk 4.1.2"
  "critcmp --version|critcmp 0.1.8"
)

missing=()
for command_name in "${required_commands[@]}"; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    missing+=("$command_name")
  fi
done

if ((${#missing[@]} > 0)); then
  printf 'ERROR: missing required Rust command(s): %s\n' "${missing[*]}" >&2
  echo "Install Rust with rustup, then rerun this script." >&2
  exit 1
fi

toolchains="$(rustup toolchain list)"
if ! grep -Eq "^${stable_toolchain}(-|[[:space:]])" <<<"$toolchains"; then
  echo "ERROR: pinned Rust toolchain $stable_toolchain is not installed" >&2
  exit 1
fi
if ! grep -Eq "^${nightly_toolchain}(-|[[:space:]])" <<<"$toolchains"; then
  echo "ERROR: pinned fuzz toolchain $nightly_toolchain is not installed" >&2
  exit 1
fi

target_list="$(rustup target list --installed)"
missing_targets=()
for target in "${required_targets[@]}"; do
  if ! grep -qx "$target" <<<"$target_list"; then
    missing_targets+=("$target")
  fi
done

missing_bins=()
for binary in "${required_cargo_bins[@]}"; do
  if ! command -v "$binary" >/dev/null 2>&1; then
    missing_bins+=("$binary")
  fi
done

if ((${#missing_targets[@]} > 0)); then
  printf 'ERROR: missing rustup target(s): %s\n' "${missing_targets[*]}" >&2
  echo "Run: rustup target add ${missing_targets[*]}" >&2
  exit 1
fi

if ((${#missing_bins[@]} > 0)); then
  printf 'ERROR: missing cargo-installed binary/binaries: %s\n' "${missing_bins[*]}" >&2
  echo "Run the cargo install command from the T0.1 ticket for the missing tools." >&2
  exit 1
fi

version_failures=0
for entry in "${required_versions[@]}"; do
  command_text="${entry%%|*}"
  expected="${entry#*|}"
  actual="$(bash -c "$command_text" 2>&1 || true)"
  if [[ "$actual" != *"$expected"* ]]; then
    echo "ERROR: expected '$expected' from '$command_text', got '$actual'" >&2
    version_failures=$((version_failures + 1))
  fi
done
if ((version_failures > 0)); then
  exit 1
fi

rustc_version="$(rustc +"$stable_toolchain" --version)"
if [[ "$rustc_version" != rustc\ 1.96.1* ]]; then
  echo "ERROR: pinned rustc mismatch: $rustc_version" >&2
  exit 1
fi

if ((write_lock == 1)); then
{
  echo "# Toolchain Lock"
  echo
  echo "Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo
  echo "## Rust"
  echo
  echo '```text'
  rustup --version
  rustc +"$stable_toolchain" --version
  cargo +"$stable_toolchain" --version
  rustfmt +"$stable_toolchain" --version
  cargo +"$stable_toolchain" clippy --version
  echo '```'
  echo
  echo "## Installed Targets"
  echo
  echo '```text'
  rustup target list --installed
  echo '```'
  echo
  echo "## Cargo Tools"
  echo
  echo '```text'
  for version_command in "${version_commands[@]}"; do
    printf '$ %s\n' "$version_command"
    $version_command 2>&1 || true
  done
  echo '```'
} > docs/toolchain.lock.md
fi

echo "PASS check_toolchain.sh"
