# T1.R10 Implementation Evidence

Date: 2026-07-14

Status: implementation and CP-HUMAN-PLAY-CLI verified locally.

## Product Path

- Reuses the exact T3.9 four-player deck loader, card runtime, controller, and
  `forge_core::apply` mutation boundary.
- Human seat decisions receive a redacted `PlayerView` plus prevalidated
  numbered options.
- `forge-human-play-replay-v1` binds every prompt, selection, human/baseline
  action, transition hash, final summary, and direct typed-action replay.

## Local Results

- Prompt/replay and hidden-information unit tests: 5 passed, 0 failed.
- Scripted readiness game: 82 turns, 93 decisions, 5,634 typed actions, winner
  seat 1, final hash `7518639965893281228`.
- Saved decision replay: exact match.
- Direct typed-action replay: exact match.
- Existing T3.9 regression: 10/10 deterministic four-player games replayed
  exactly.
- GitHub Actions, network access, downloads, installs, pushes, and PRs: none.

## Owner Checkpoint

- Real Owner game completed on 2026-07-14.
- Seed: `17306488943054686736`.
- Human decisions: 56.
- Typed actions: 4,901.
- Winner: seat 1.
- Final hash: `7878648484579518403`.
- Decision and typed-action replay: exact match.
- Replay SHA-256:
  `8d639f5794bff1dafcd53f6e327bd1fd9fa30670b8a874f90c8766659e0f1ae0`.

CP-HUMAN-PLAY-CLI passes. T4.1-T4.3 may begin.
