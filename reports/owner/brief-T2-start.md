# OWNER BRIEF - T2 Full Rules Layer Start

Date: 2026-07-07

## 1. WHAT THIS TIER WILL BUILD

T2 is where Forge 2.0 starts handling the rules machinery that makes real Magic
cards difficult: events, triggers, replacement/prevention effects, continuous
layers, activated abilities, targeting, counters, tokens, multiplayer, and a
nightmare-deck integration suite. The first task, T2.1, creates the event
stream that later trigger and replacement code will listen to.

Expected duration: T2 is a hard multi-week tier. The risky midpoint is T2.4
layers, followed by your CP-LAYERS human checkpoint before any T2.5+ work can
build on it.

## 2. WHAT YOU SHOULD SEE - TRY IT YOURSELF

- DO: `cargo test -p forge-core event --quiet`
- EXPECT: event-system tests pass, including event ordering, failed-action
  cleanliness, bounded event retention, and turn reset behavior.
- RED FLAG: any failed test or panic text. Reply with the output; an agent
  investigates within 24 hours.

- DO LATER AT T2 GATE: `scripts/run_oracle.sh --all`
- EXPECT: at least 1,200 oracle scenarios pass with 0 failed and 0 skipped.
- RED FLAG: skipped scenarios or any failure count above 0.

## 3. NUMBERS THAT MATTER

- T1 is passed and remote-green.
- T2 starts with 300 checked-in oracle scenarios from the T1 gate.
- T2 gate target is at least 1,200 green oracle scenarios.
- CP-LAYERS requires 15 novel reviewer layer scenarios, a legacy differential
  run on 100 layered cards, and a memoization-invalidation audit after T2.4.

## 4. KNOWN ROUGH EDGES

T2.1 creates the event stream only. It does not yet make triggered abilities
fire, prevent damage, replace draws, or apply continuous effects. That is
expected: events are the substrate those systems need before they can be built
correctly.

## 5. WHAT YOU SHOULD EXPECT NEXT

Next visible milestone is T2.1 verification: event records in `forge-core`, a
T2.1 spec/ticket, green local tests, and then a small commit for you to push
through GitHub Desktop.

## 6. WHAT WE NEED FROM YOU

Nothing right now. Your next required decision is still CP-LAYERS after T2.4.
