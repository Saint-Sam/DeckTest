# CP-KERNEL Remediation Evidence - 2026-07-06

Checkpoint: `CP-KERNEL`

Original reviewed commit: `6491d5fd12887eb5f5b7ad8d57eda83e1ff17783`

This is Orchestrator evidence for scoped re-review of the failed CP-KERNEL
items. It does not edit or replace the original failed `SIGNOFF.md`.

## Remediation Summary

| Ticket | Status | Evidence |
| --- | --- | --- |
| T1.R1 action surface | Verified locally | Public mutation is routed through `legal_actions(&GameState) -> ActionList` and `apply(&mut GameState, Action) -> Outcome`. `scripts/review/no_public_mutating_gamestate.sh` is wired into `scripts/vl.sh`. |
| T1.R2 characteristic boundary | Verified locally | `ObjectRecord` stores `BaseCreatureCharacteristics`; rules paths derive current characteristics through `GameState::creature_characteristics`. |
| T1.R3 hidden information | Verified locally | `GameState::player_view(observer)` returns `PlayerView`; hand and library identities are redacted through `ObjectView::Hidden`; `forge-ai` accepts `PlayerView`; full-state `GameSnapshot` was removed; `canonical_bytes` is private diagnostics. |
| T1.R4 clone surface | Verified locally | `scripts/review/clone_surface_guard.sh` is wired into `scripts/vl.sh`; every persistent allocation-bearing clone field has a `clone_surface:` invariant; `metrics/clone_surface.json` records an 18-field baseline; `GameState` does not derive `Debug`; `GameSnapshot`/`StateSnapshot`/`snapshot()` are guarded against resurrection. |

## Clone Surface Baseline

Baseline file: `metrics/clone_surface.json`

- `persistent_allocation_field_count`: 18
- `full_state_snapshot_surface`: false
- `game_state_debug`: false
- Guard script: `scripts/review/clone_surface_guard.sh`
- VL step: `clone surface guard`

The guard also fails on new unallowlisted allocation-bearing `GameState`
wrappers, `StateSnapshot`/`GameSnapshot`, common alternate full-state clone API
names, `From<&GameState>`, and loss of `Copy` on the Copy-record structures
used in the clone-surface invariant.

## Local Verification

Final T1.R4 verification run:

- `scripts/selftest.sh`: PASS
- `cargo fmt --all -- --check`: PASS
- `cargo clippy -p forge-core --all-targets -- -D warnings`: PASS
- `cargo test -p forge-core`: PASS, 65 tests
- `scripts/review/clone_surface_guard.sh`: PASS, `persistent_allocation_field_count=18`
- `scripts/vl.sh`: PASS, ended with `ALL CHECKS PASSED`

Sandbox note: local macOS runs print conda and `xcrun` cache warnings under the
Codex sandbox. They did not correspond to Rust, verification-loop, or guard
failures.
