# ADR 0030: Bounded Meaningful Search Transitions

Status: accepted for local T4 diagnostics on 2026-07-14.

## Context

The UCT implementation searched real canonical actions, but its production
game domains stopped after an immediate main-phase action or completed combat
declaration. A cast could therefore be evaluated while still on the stack,
before opponents received priority, and an attack could be evaluated before
unopposed combat damage. More iterations over those leaves did not provide a
deeper game search.

Multiplayer nodes also require an explicit backup convention. Maximizing the
root player's scalar value at every node would make opponents silently
cooperative.

## Decision

- Main and priority search share one typed state carrying the current actor,
  decision window, canonical context, mappings, and a 12-decision bound.
- A completed cast or activation advances through production priority. A
  singleton pass is automatic; a player with a legal response becomes a real
  search node. Consecutive passes resolve the actual stack through the kernel
  and runner interpreter before evaluation.
- Forced runtime-backed trigger placement with no target or distinct ordering
  choice is traversed through the production trigger action. Choice-bearing
  trigger placement, unknown trigger runtimes, and deferred resolution prompts
  remain fail-closed leaves until they receive typed search-state adapters.
- The product backup rule is a documented paranoid-coalition scalarization:
  the root actor maximizes root value and every opponent minimizes it. The
  former root-max-at-every-node behavior remains only as an explicit ablation
  input to the backup-sign function.
- The frozen opponent-response regression applies both conventions to the
  same canonical state and verifies opposite backup signs without changing
  its legal actions or state. This is structural evidence, not a strength
  result; paired arena ablation remains required before promotion.
- Completed attacker declarations may advance through forced priority, empty
  blocker declarations, and actual combat-damage assignment only when no
  defender has a legal blocker and no priority response exists. Otherwise the
  combat domain stops at the unresolved meaningful boundary.
- State keys include actor, window, and consumed decision count. Search never
  shares states with different backup roles or remaining bounded horizons.

## Consequences

Focused regressions now include an opponent stack response, a forced triggered
ability, and an unopposed combat-damage line. A fixed-iteration production
domain test reaches depth greater than one.

This ADR does not claim full-turn search, MaxN, choice-bearing trigger search,
blocked-combat continuation, hidden-information policy quality, calibrated
playing strength, or T4 promotion. Those remain measured follow-on work. The
paranoid model must be compared with at least one alternative on paired frozen
decision benchmarks before any strength claim.
