# T1.R10 Implementation Evidence

Date: 2026-07-14

Status: implementation verified locally; CP-HUMAN-PLAY-CLI Owner game pending.

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

The scripted run proves implementation readiness only. It does not satisfy the
Owner checkpoint. T4 remains blocked until the Owner completes the command in
`reports/owner/brief-CP-HUMAN-PLAY-CLI.md` and its replay verifies exactly.
