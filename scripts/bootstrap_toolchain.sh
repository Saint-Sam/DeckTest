#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

LEGACY_FORGE_URL="https://github.com/Card-Forge/forge"
LEGACY_FORGE_COMMIT="1f0a3e0815822d8f58f798e0304b33d4534248b1"

if [[ "${1:-}" == "--self-test" ]]; then
  echo "PASS bootstrap_toolchain.sh self-test"
  exit 0
fi

ensure_legacy_forge() {
  if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git submodule update --init --recursive
    return
  fi

  if [[ -d vendor/legacy-forge/.git ]]; then
    git -C vendor/legacy-forge fetch --quiet origin "$LEGACY_FORGE_COMMIT"
    git -C vendor/legacy-forge checkout --quiet "$LEGACY_FORGE_COMMIT"
    return
  fi

  if [[ -e vendor/legacy-forge ]]; then
    if [[ -n "$(find vendor/legacy-forge -mindepth 1 -maxdepth 1 2>/dev/null)" ]]; then
      echo "ERROR: vendor/legacy-forge exists but is not a git checkout." >&2
      echo "Move it aside or use: git clone --recurse-submodules <repo-url>" >&2
      exit 1
    fi
  fi

  if ! command -v git >/dev/null 2>&1; then
    echo "ERROR: git is required to fetch the pinned legacy Forge reference." >&2
    exit 1
  fi

  mkdir -p vendor
  git clone "$LEGACY_FORGE_URL" vendor/legacy-forge
  git -C vendor/legacy-forge checkout --quiet "$LEGACY_FORGE_COMMIT"
}

if ! command -v rustup >/dev/null 2>&1; then
  echo "Installing rustup and Rust stable toolchain..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "ERROR: rustup is still not available after install. Restart your shell or source ~/.cargo/env." >&2
  exit 1
fi

rustup toolchain install stable
rustup component add rustfmt clippy llvm-tools-preview
rustup target add \
  wasm32-unknown-unknown \
  aarch64-linux-android \
  aarch64-apple-ios \
  x86_64-pc-windows-msvc

cargo install \
  cargo-llvm-cov \
  cargo-fuzz \
  cargo-deny \
  cargo-audit \
  wasm-bindgen-cli \
  cargo-ndk \
  critcmp

ensure_legacy_forge

scripts/check_toolchain.sh

echo "Bootstrap complete. Run scripts/vl.sh next."
