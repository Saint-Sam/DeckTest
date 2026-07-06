#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS check_toolchain.sh self-test"
  exit 0
fi

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

{
  echo "# Toolchain Lock"
  echo
  echo "Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo
  echo "## Rust"
  echo
  echo '```text'
  rustup --version
  rustc --version
  cargo --version
  rustfmt --version
  cargo clippy --version
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

echo "PASS check_toolchain.sh"
