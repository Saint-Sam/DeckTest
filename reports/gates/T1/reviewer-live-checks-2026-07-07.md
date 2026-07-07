# T1 Reviewer Live Checks

Date: 2026-07-07

Reviewer: Codex Gate Reviewer for T1 re-review

Result: PASS local reviewer checks. Exact final-tree remote CI later passed as
GitHub Actions `ci #18`, run ID `28883217715`, for commit
`2198493284299d9721d59ab3a23e3b2a2ab71f56`.

## Mutation Checks

Three sampled tests were mutation-checked with `scripts/review/mutation_check.sh`
using `MUTATION_EDIT_CMD`; each baseline test passed, each mutant failed, and
the mutation runner reported `PASS mutation_check.sh`.

- `cargo test -p forge-core double_block_damage_must_follow_blocker_order`
  caught removal of `validate_blocker_assignment_order`.
  Evidence: `reports/gates/T1/mutation-double-block-order-2026-07-07.log`
- `cargo test -p forge-core lifelink_is_applied_before_loss_state_based_actions`
  caught disabling lifelink life gain before loss SBAs.
  Evidence: `reports/gates/T1/mutation-lifelink-before-sba-2026-07-07.log`
- `cargo test -p forge-testkit ron_scenario_covers_combat_actions` caught
  removal of the `assign_combat_damage` scenario action parser arm.
  Evidence: `reports/gates/T1/mutation-combat-scenario-dsl-2026-07-07.log`

## Novel Reviewer Oracles

The reviewer authored five novel scenarios outside the production 300-scenario
pack under `reports/gates/T1/reviewer_oracles/`.

Command:

```text
cargo run -p forge-testkit -- oracle --path reports/gates/T1/reviewer_oracles --no-junit
```

Result: 5 passed, 0 failed.

Evidence: `reports/gates/T1/reviewer-oracles-2026-07-07.log`

Scenarios:

- `reviewer_t1_menace_two_blockers.ron`
- `reviewer_t1_first_strike_removes_blocker.ron`
- `reviewer_t1_double_strike_unblocked_twice.ron`
- `reviewer_t1_trample_lethal_without_deathtouch.ron`
- `reviewer_t1_lifelink_zero_life_attacker_wins.ron`

## Determinism Replay

Command:

```text
scripts/review/determinism.sh
```

Result: PASS, 10 archived T1 replays checked.

Evidence: `reports/gates/T1/determinism-review-2026-07-07.log`

## Live Fuzz

Command:

```text
FORGE_FUZZ_SECONDS=1800 FORGE_FUZZ_SANITIZER=address scripts/fuzz_nightly.sh review
```

Result: PASS, no crashes.

Targets:

- `fuzz_apply`
  - Seed: `445226698`
  - Runs: `1845413`
  - Duration: `1801` seconds
  - Result: no crash
- `fuzz_scenarioparse`
  - Seed: `2248302062`
  - Runs: `219668185`
  - Duration: `1801` seconds
  - Result: no crash

## Spot Replays

Three archived T1 bundle replays were step-replayed with `forge-cli replay`.

- Seed 11: final hash `17913199206715572167`, outcome `won player 0`.
  Evidence: `reports/gates/T1/spot-replay-seed-11-2026-07-07.log`
- Seed 14: final hash `11845241288509108833`, outcome `won player 0`.
  Evidence: `reports/gates/T1/spot-replay-seed-14-2026-07-07.log`
- Seed 20: final hash `713360490662108421`, outcome `won player 0`.
  Evidence: `reports/gates/T1/spot-replay-seed-20-2026-07-07.log`

## Review Tooling Fixes

- `scripts/review/determinism.sh` now finds the archived T1 replay corpus when
  the repository root has no `replays/` directory and uses the supported
  `forge-cli replay` command.
- `scripts/review/mutation_check.sh` now supports targeted edit-command
  mutation checks, avoiding the false pass observed when reversing an entire
  evidence diff removed the tests under review.
- `forge-testkit oracle --path PATH` allows reviewer-only oracle probes without
  changing the production 300-scenario corpus.
