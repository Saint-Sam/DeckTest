# OWNER BRIEF - T1 Kernel Start

Date: 2026-07-06

## 1. WHAT THIS TIER WILL BUILD

T1 turns the empty workspace into the first playable rules kernel: state arenas,
turn structure, priority/stack, mana and casting, combat, state-based actions,
mulligans, oracle scenario running, a terminal demo game, fuzz targets, and
baseline performance benches.

Expected duration: T1 remains open until `gate_T1.sh` proves at least 250
oracle scenarios, a terminal demo game, fuzz evidence, and clone/perf budgets.

## 2. WHAT YOU SHOULD SEE - TRY IT YOURSELF

- DO: `cargo test -p forge-core`
- EXPECT: T1.1 kernel-state tests pass.
- RED FLAG: any test failure or panic text. Reply with the output; an agent
  investigates within 24 hours.

- DO LATER AT T1 GATE: `cargo run -p forge-cli -- play --demo`
- EXPECT: a simplified terminal Magic game with numbered choices, a winner
  banner, and a saved replay path.
- RED FLAG: hangs, illegal moves, or crash text.

## 3. NUMBERS THAT MATTER

- 1 T1 task is locally verified: T1.1 core state.
- 5 `forge-core` unit tests cover typed zones, movement, conservation,
  snapshots, and deterministic hashing.
- 0 T1 oracle scenarios exist yet; they start once the scenario runner and
  early turn/priority rules land.
- 250+ oracle scenarios are required before the T1 gate can pass.
- T1 gate status is not passed.

## 4. KNOWN ROUGH EDGES

The kernel can store and hash objects, but it cannot yet play Magic. There is
no turn machine, priority, mana, casting, combat, oracle runner, or demo CLI
game. T0 is passed conditional; before collaboration branches or release,
GitHub `main` branch protection still needs to require the T0.3 CI jobs.

## 5. WHAT YOU SHOULD EXPECT NEXT

Next implementation target is T1.2: explicit turn structure and steps/phases
from CR 5, with cleanup and end-of-turn timing scenarios.

## 6. WHAT WE NEED FROM YOU

No immediate input. The next human-heavy checkpoint remains CP-LAYERS in T2.
