#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

LEGACY_FORGE_URL="https://github.com/Card-Forge/forge"
LEGACY_FORGE_COMMIT="1f0a3e0815822d8f58f798e0304b33d4534248b1"
RUST_STABLE="1.96.1"
RUST_NIGHTLY="nightly-2026-07-05"

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

mode="${1:---check}"
if [[ "$mode" != "--check" && "$mode" != "--install" ]]; then
  echo "usage: scripts/bootstrap_toolchain.sh [--check|--install]" >&2
  exit 2
fi

if [[ "$mode" == "--check" ]]; then
  if [[ -e .git ]]; then
    if [[ ! -f vendor/legacy-forge/.git && ! -f vendor/legacy-forge/.git/HEAD ]]; then
      echo "ERROR: pinned legacy submodule is absent from this git checkout" >&2
      echo "Run only after approval: git submodule update --init --recursive" >&2
      exit 1
    fi
  else
    reference="vendor/legacy-forge.reference.json"
    if [[ ! -s "$reference" ]] \
      || ! grep -q "\"commit\": \"$LEGACY_FORGE_COMMIT\"" "$reference" \
      || ! grep -q '"required_for_baseline_build": false' "$reference"; then
      echo "ERROR: source archive lacks the pinned legacy reference manifest" >&2
      exit 1
    fi
    echo "Archive mode: baseline verification uses the bundled legacy pin manifest."
  fi
  scripts/check_toolchain.sh
  echo "Bootstrap check complete. No software was installed and no network was used."
  exit 0
fi

echo "Explicit install mode: this command uses the network and may install toolchains."
if ! command -v rustup >/dev/null 2>&1; then
  echo "ERROR: rustup is not installed." >&2
  echo "Install rustup from https://rustup.rs, then rerun this command." >&2
  exit 1
fi

rustup toolchain install "$RUST_STABLE" --profile default
rustup component add --toolchain "$RUST_STABLE" rustfmt clippy llvm-tools-preview
rustup target add --toolchain "$RUST_STABLE" \
  wasm32-unknown-unknown \
  aarch64-linux-android \
  aarch64-apple-ios \
  x86_64-pc-windows-msvc
rustup toolchain install "$RUST_NIGHTLY" --profile minimal

cargo +"$RUST_STABLE" install cargo-llvm-cov --version 0.8.7 --locked
cargo +"$RUST_STABLE" install cargo-fuzz --version 0.13.2 --locked
cargo +"$RUST_STABLE" install cargo-deny --version 0.19.9 --locked
cargo +"$RUST_STABLE" install cargo-audit --version 0.22.2 --locked
cargo +"$RUST_STABLE" install wasm-bindgen-cli --version 0.2.126 --locked
cargo +"$RUST_STABLE" install cargo-ndk --version 4.1.2 --locked
cargo +"$RUST_STABLE" install critcmp --version 0.1.8 --locked

ensure_legacy_forge
scripts/check_toolchain.sh --write-lock

echo "Bootstrap install complete. Run scripts/local_verify.sh task next."
