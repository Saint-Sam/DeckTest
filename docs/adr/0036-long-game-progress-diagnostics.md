# ADR 0036: Long-Game Progress Diagnostics

## Status

Accepted for T4 diagnostics. No no-progress termination rule is approved.

## Context

Current exact heuristic and search baselines take roughly 243 turns, while the
small search-knee games take roughly 247. A winner proves completion but does
not explain whether those games contain real development, repeated states, or
long pass-only stretches. Strength and cost reports therefore need game length
and progress evidence alongside win rate.

## Decision

The production runner records additive, deterministic diagnostics without
changing legality, policy selection, state hashes, or termination:

- full-state observations and repeated full-state hashes;
- repeated recorded `DecisionStateKey` values;
- per-round player damage, absolute life movement, casts, meaningful actions,
  empty-stack pass-only priority cycles, active-player progress, and
  eliminations;
- no-progress round count and maximum consecutive no-progress rounds;
- elimination seat and turn; and
- an explicit termination reason and turn-cap flag.

A table round is four successive game turns. A meaningful action is a
successful typed action during the controlled game loop other than priority
passing, step advancement, a rules-only state check, turn start, or cleanup
priority request. A no-progress round has no player damage, life movement,
cast, meaningful action, or elimination. These definitions are diagnostic
contracts, not claims that every counted action is strategically useful.

The arena preserves each game's diagnostics and aggregates turn percentiles,
turn-cap rate, repetition rates, progress counters, and elimination timing.
All rates use integer parts per million to remain deterministic.

## Compatibility

Fields are additive and default when old T3 pod and human replays are read.
Their legacy verification adapter may ignore only these new instrumentation
fields. Current T4 AI evidence must be regenerated and match them exactly.

## Promotion Boundary

The detector is observation-only. It may not declare a draw, choose a winner,
alter policy rewards, or end a game. Any future no-progress termination or
policy penalty requires measured multi-pod evidence and an approved plan
change. Ordinary wins and diagnostic terminations must remain distinct.
