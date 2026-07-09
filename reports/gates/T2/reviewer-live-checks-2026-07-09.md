# T2 Reviewer Live Checks

Date: 2026-07-09

Result: PASS local reviewer checks. Exact T2 evidence commit remote CI passed
separately as GitHub Actions `ci #44`, run ID `28993856797`, for commit
`0fdc23dea157ee55226eae24d8d4d817c46b5d59`.

## Mutation Checks

Three sampled reviewer probes were mutation-checked with
`scripts/review/mutation_check.sh`. Each baseline run passed and each mutated
run failed as expected; the mutation runner reported `PASS mutation_check.sh`.

- Copy characteristics: mutating expected source power from 7 to 6 caused
  `reviewer_t2_copy_ignores_counters_and_modifiers.ron` to fail.
  Evidence: `reports/gates/T2/mutation-copy-characteristics-2026-07-09.log`
- Protection targeting: mutating the protected red-source target expectation
  from false to true caused
  `reviewer_t2_protection_uses_effective_source_color.ron` to fail.
  Evidence: `reports/gates/T2/mutation-protection-targeting-2026-07-09.log`
- Activated life ability: mutating expected life from 24 to 23 caused
  `reviewer_t2_mana_then_stack_ability.ron` to fail.
  Evidence: `reports/gates/T2/mutation-activated-life-2026-07-09.log`

## Reviewer Oracle Probes

The reviewer authored eight additional scenarios outside the production
1,200-scenario corpus under `reports/gates/T2/reviewer_oracles/`.

Command:

```bash
cargo run -p forge-testkit -- oracle --path reports/gates/T2/reviewer_oracles --no-junit
```

Result: PASS; 8 passed, 0 failed.

Evidence: `reports/gates/T2/reviewer-oracles-2026-07-09.log`

Scenarios:

- `reviewer_t2_copy_ignores_counters_and_modifiers.ron`
- `reviewer_t2_token_copy_ceases_without_original.ron`
- `reviewer_t2_commander_tax_identity_combo.ron`
- `reviewer_t2_protection_uses_effective_source_color.ron`
- `reviewer_t2_haste_does_not_override_cannot_attack.ron`
- `reviewer_t2_scry_then_surveil_zone_order.ron`
- `reviewer_t2_mana_then_stack_ability.ron`
- `reviewer_t2_layer_counter_sba_lethal.ron`

## Determinism Replay

Command:

```bash
DETERMINISM_REPLAY_DIR=reports/gates/T2/replays scripts/review/determinism.sh
```

Result: PASS; 3 replay(s) were replayed twice with matching output.

Evidence: `reports/gates/T2/determinism-review-2026-07-09.log`

## Spot Play

Three demo seeds were spot-played, archived as replay files, and round-tripped
through `forge-cli roundtrip`.

- Seed 101: final hash `1566419436961389613`, 14 actions, outcome `won player 0`.
  Evidence: `reports/gates/T2/spot-replay-seed-101-2026-07-09.log`
- Seed 202: final hash `8677981376605351123`, 14 actions, outcome `won player 0`.
  Evidence: `reports/gates/T2/spot-replay-seed-202-2026-07-09.log`
- Seed 303: final hash `2598573421341448852`, 14 actions, outcome `won player 0`.
  Evidence: `reports/gates/T2/spot-replay-seed-303-2026-07-09.log`

Archived replay files:

- `reports/gates/T2/replays/spot-seed-101.frsreplay`
- `reports/gates/T2/replays/spot-seed-202.frsreplay`
- `reports/gates/T2/replays/spot-seed-303.frsreplay`

## Notes

The macOS sandbox emitted repeated `xcrun` cache warnings during local builds.
Those warnings did not affect command exit status or replay/oracle results.
