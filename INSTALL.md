# Install Forge 2.0 From GitHub

Forge 2.0 is bootstrapped so a fresh checkout can install the same local
toolchain used by this repository.

## Recommended: Git Clone

```bash
git clone --recurse-submodules https://github.com/Saint-Sam/DeckTest.git DeckTest
cd DeckTest
bash scripts/bootstrap_toolchain.sh --check
bash scripts/local_verify.sh task
```

## GitHub Download ZIP

GitHub source ZIP files do not include git submodule contents. The ZIP includes
the pinned legacy-reference manifest, and the baseline build and local task
verification do not require the legacy source tree.

```bash
cd DeckTest-main
bash scripts/bootstrap_toolchain.sh --check
bash scripts/local_verify.sh task
```

Legacy mining and differential tools are optional development workflows. Run
`bash scripts/bootstrap_toolchain.sh --install` only after approving network
access if those workflows need to materialize `vendor/legacy-forge`.

## What The Bootstrap Installs

`bash scripts/bootstrap_toolchain.sh --check` performs no installation or
network access. If it reports missing tools, the explicit
`bash scripts/bootstrap_toolchain.sh --install` command may install:

- rustup and the stable Rust toolchain
- rustfmt, clippy, and llvm-tools-preview
- Rust targets for wasm, Android, iOS, and Windows smoke builds
- cargo helper tools: cargo-llvm-cov, cargo-fuzz, cargo-deny, cargo-audit,
  wasm-bindgen-cli, cargo-ndk, and critcmp
- the pinned legacy Forge reference under `vendor/legacy-forge`

The repository itself includes the GPL-3.0-only license, Rust workspace
metadata, official comprehensive rules text snapshot, gate scripts, validation
scripts, and Tier 0 reports needed to verify the bootstrap.
