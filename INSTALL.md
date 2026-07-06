# Install Forge 2.0 From GitHub

Forge 2.0 is bootstrapped so a fresh checkout can install the same local
toolchain used by this repository.

## Recommended: Git Clone

```bash
git clone --recurse-submodules <repo-url> forge-rs
cd forge-rs
bash scripts/bootstrap_toolchain.sh
bash scripts/vl.sh
```

## GitHub Download ZIP

GitHub source ZIP files do not include git submodule contents. This repository's
bootstrap script handles that by fetching the pinned legacy Forge reference.

```bash
cd forge-rs
bash scripts/bootstrap_toolchain.sh
bash scripts/vl.sh
```

## What The Bootstrap Installs

Running `bash scripts/bootstrap_toolchain.sh` may install or update:

- rustup and the stable Rust toolchain
- rustfmt, clippy, and llvm-tools-preview
- Rust targets for wasm, Android, iOS, and Windows smoke builds
- cargo helper tools: cargo-llvm-cov, cargo-fuzz, cargo-deny, cargo-audit,
  wasm-bindgen-cli, cargo-ndk, and critcmp
- the pinned legacy Forge reference under `vendor/legacy-forge`

The repository itself includes the GPL-3.0-only license, Rust workspace
metadata, official comprehensive rules text snapshot, gate scripts, validation
scripts, and Tier 0 reports needed to verify the bootstrap.
