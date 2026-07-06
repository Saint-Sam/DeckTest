# T0 Cross-Target Build Smoke

Date: 2026-07-06

Local target build smokes passed after `scripts/gates/gate_T0.sh`:

- `cargo build -p forge-app-wasm --target wasm32-unknown-unknown --release`
- `cargo build -p forge-app-android --target aarch64-linux-android --release`
- `cargo build -p forge-app-ios --target aarch64-apple-ios --release`
- `cargo build -p forge-app-desktop --target x86_64-pc-windows-msvc --release`

These are empty Tier 0 shell crates, so this does not replace GitHub Actions
matrix validation. It confirms that the Rust targets installed by T0.1 can
compile the bootstrap crates locally.

