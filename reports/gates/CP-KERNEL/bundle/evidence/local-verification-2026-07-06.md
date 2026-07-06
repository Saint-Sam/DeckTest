# CP-KERNEL Local Verification Evidence - 2026-07-06

Reviewed head: `6491d5f` (`T1.7: add state-based actions`)

Local verification completed before the T1.7 commit:

- `cargo fmt --all -- --check`: PASS
- `cargo test -p forge-core`: PASS, 60 tests
- `cargo clippy -p forge-core --all-targets -- -D warnings`: PASS
- `scripts/vl.sh`: PASS, ended with `ALL CHECKS PASSED`
- `scripts/review/no_unwrap.sh`: PASS
- `scripts/review/no_card_names.sh`: SKIP, no card-name source present yet
- `scripts/review/determinism.sh`: SKIP, replay corpus absent at this tier
- `git diff --check`: PASS

Notes:

- The local shell prints conda and macOS `xcrun` cache warnings under the Codex
  sandbox. They have not corresponded to Rust, verification-loop, or CI
  failures.
- The Gate Reviewer should rerun any checks they rely on from a fresh clone per
  plan Section 15.
