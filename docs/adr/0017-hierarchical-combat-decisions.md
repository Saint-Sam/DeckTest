# ADR 0017: Hierarchical Combat Decisions

Status: accepted for local T4 diagnostics on 2026-07-14.

## Context

Complete multiplayer attack and block declarations form a Cartesian product.
Materializing every product made canonical legality explicit, but decision
construction became non-preemptible and dominated small search budgets. It
also imposed a diagnostics-only option ceiling on otherwise legal combat.

The T4 engineering assessment requires hierarchical high-branching choices,
complete canonical legality, hidden-safe replay identity, and no silent option
loss.

## Decision

Combat is represented as bounded canonical subcontexts:

- each attack-capable creature chooses no attack or one legal player defender;
- each block-capable creature chooses no block or one legal attacker;
- partial declarations are retained as typed search state;
- menace prefixes are offered only when a distinct remaining-blocker matching
  can complete every currently single-blocked menace attacker;
- the kernel receives one complete `DeclareAttackers` or `DeclareBlockers`
  action after all subchoices;
- append-only descriptor tags represent attacker and blocker assignments;
- a typed-path discriminator binds each subcontext and benchmark state key to
  its prior visible choices;
- transposition equivalence includes the complete partial declaration.

The determinization adapter also constructs the sampled game state directly in
the search clone, removing an accidental first full-state clone that was
immediately discarded.

## Consequences

Every currently legal player-defender combat assignment remains reachable,
including split attacks and multiple blockers. Context width is linear in
opponents or attackers rather than exponential in creatures. Forced empty
combat choices no longer consume search work. Existing complete-declaration
descriptor tags and legacy human replay adapters remain unchanged.

Exact AI decision replays must be regenerated because canonical combat prompts
are intentionally finer grained. Typed kernel action replay, final game state,
and hidden-information invariants remain the semantic regression authority.

This decision does not complete non-player defenders, strategic damage order,
or the other open canonical prompt families.
