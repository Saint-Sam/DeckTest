# ADR-0015: Production Game Runner Extraction

Status: accepted and implemented locally, 2026-07-14.

## Context

The T3.9 four-player driver grew into the production execution path for human
play, AI decisions, arena campaigns, replay verification, and card-runtime
integration. Its implementation still lived at
`tests/t3_9/four_player_pod.rs` and was compiled into `forge-testkit` through a
path module. `forge-cli` and `forge-arena` therefore depended on a testing
crate for live product behavior.

That ownership obscured the production boundary and made it possible for test
support to become the only implementation of game behavior. The 2026-07-14 T4
engineering assessment classified this as a P1 architecture finding.

## Decision

The complete runner is now owned by the production crate
`forge-game-runner`. It provides:

- deterministic four-player game execution;
- human and AI decision adapters;
- arena and campaign entry points;
- typed-action capture and replay verification;
- the bridge from compiled card programs to the kernel's public mutation API.

`forge-cli` and `forge-arena` depend directly on `forge-game-runner`.
`forge-testkit` retains a compatibility re-export named `t3_9_pod`, but owns no
runner implementation. The `forge-t3-9-four-player-pod` binary is built from
the production crate. Coverage, T1.R10, T3.9, mutation, and T4 gate scripts
address the new owner directly.

## Boundaries

This extraction does not move rules authority out of `forge-core` or card
semantics out of `forge-cards::runtime`. Every game mutation still crosses
`forge_core::apply`, AI still consumes typed canonical decisions and redacted
views, and unsupported prompts still fail closed.

The move is behavior-preserving. It does not promote T4, expand the canonical
prompt surface, select a search budget, or authorize learned models.

## Compatibility

Downstream Rust code using `forge_testkit::t3_9_pod` continues to compile via
the compatibility re-export. Repository-owned production callers use
`forge_game_runner` directly. Existing replay formats and exact replay checks
are unchanged.

## Verification

The extraction must keep all of the following green before it can be included
in the next exact T4 product:

- workspace formatting and strict clippy;
- `forge-game-runner`, `forge-testkit`, `forge-cli`, and `forge-arena` tests;
- four-deck manifest validation through the production binary;
- T1.R10 self-test and exact human replay path;
- all three exact T4 baseline replays;
- local T4 preflight and cross-target compilation.

The final exact T4 packet will be regenerated once the related search
correctness wave is complete, so architecture and behavioral evidence share
one frozen product commit.

## Alternatives

- **Keep the path module in `forge-testkit`:** rejected because product code
  would continue to depend on test ownership.
- **Duplicate a smaller production runner:** rejected because two game drivers
  would drift and undermine exact replay authority.
- **Move only the CLI wrappers:** rejected because the live behavior, not the
  executable entry point, was the ownership problem.
