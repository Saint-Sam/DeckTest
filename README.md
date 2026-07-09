# Forge 2.0 / forge-rs

Forge 2.0 is a GPL-3.0-only Rust rebuild of Forge, guided by
`FORGE_REBUILD_MASTER_PLAN.md`.

This repository has completed T0-T2 and is currently in Tier 3. The Orchestrator-owned state
is `PLAN_STATE.json`; Owner-facing updates live under `reports/owner/`.

## Fresh Checkout Setup

See `INSTALL.md` for the full install request and ZIP fallback.

Use a recursive clone so the pinned legacy Forge reference is available:

```bash
git clone --recurse-submodules https://github.com/Saint-Sam/Forge-2.0.git forge-2.0
cd forge-2.0
bash scripts/bootstrap_toolchain.sh --check
bash scripts/local_verify.sh task
```

If you downloaded the repository before initializing submodules, run:

```bash
git submodule update --init --recursive
bash scripts/bootstrap_toolchain.sh --check
```

The repository includes the project Rust pin in `rust-toolchain.toml`, the
explicit check/install helper in `scripts/bootstrap_toolchain.sh`, the official rules text
in `docs/vendor/comprehensive-rules.txt`, and a pinned legacy Forge submodule in
`vendor/legacy-forge`.

Magic: The Gathering is owned by Wizards of the Coast. This project is not
affiliated with, endorsed, sponsored, or specifically approved by Wizards of the
Coast. It does not ship official card art, official set symbols, official mana
symbol fonts, or copied official visual assets.

GitHub Actions are intentionally disabled. Verification runs locally through
`scripts/local_verify.sh`; archived workflow definitions cannot trigger on
pushes or schedules.
