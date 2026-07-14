# CP-HUMAN-PLAY-CLI Owner Brief

## What You Do

From the repository root, run:

```bash
cargo run -p forge-cli --locked --offline -- play --human --seat 1 --replay-out reports/gates/T1.R10/owner-game.frsreplay
```

Choose numbered actions until the game reports a winner. `q` stops without
passing the checkpoint. Then verify the saved game:

```bash
cargo run -p forge-cli --locked --offline -- replay reports/gates/T1.R10/owner-game.frsreplay
```

## Expect

- Prompts show four life totals and only legal choices for your seat.
- Your hand cards may be named; opponent hidden cards and all library cards are
  never named.
- The game reaches a normal winner without a hang or rejected displayed action.
- Replay ends with `decisions and typed actions verified` and the same winner
  and final hash.

## Red Flags

- A prompt exposes an opponent hand or any library card identity.
- A displayed option is rejected, the game cannot advance, or direct state
  mutation is used to force a result.
- Replay reports a decision, action, state-hash, winner, or summary mismatch.

Scripted choices are development evidence only and do not count. The Owner's
real play and successful replay are the CP-HUMAN-PLAY-CLI decision.
