# PC-0005: Corpus-driven DSL amendment and priority coverage

Date: 2026-07-10

Status: Accepted and incorporated in Master Plan v1.6.

## Motivation

The first broad translation campaigns proved that the largest remaining card
gaps are concentrated in a small set of corpus-demonstrated value, condition,
binding, and event shapes. The Owner also supplied a 365-card Commander-focused
coverage list whose cards should steer implementation order without replacing
the global card-coverage contract.

## Exact Plan Change

1. Reopen the frozen CP-DSL operation surface only for exact, corpus-proven
   representation gaps in these bounded families:
   - parameterized keyword costs and values;
   - activation zones and general phase events;
   - conditional and unless-paid costs;
   - card-choice bindings needed by dig and reveal effects; and
   - explicit fight and regenerate operations.
2. Require each addition to remain closed and typed, round-trip canonically,
   reject malformed or approximate forms, and include structural tests before
   CP-DSL is re-frozen.
3. Add the Owner-supplied priority list as a deterministic translation input
   and generate tier-aware per-card results with exact quarantine reasons.
4. Use priority coverage to order mapper batches. It does not weaken the global
   playable-card coverage, semantic verification, quarantine, or release gates.
5. Rank linked-ability work by dependent fan-out as well as direct use count so
   one exact sub-ability lowering can unblock every trigger that executes it.
6. Keep translation local-only and resource-aware. Use one Cargo target/cache,
   cap a single translation pool at useful hardware parallelism, and use spare
   capacity for independent audits and tests instead of duplicate worktrees or
   oversubscribed worker pools.

## Affected Tasks

T3.1, T3.3-T3.7, CP-DSL, CP-PORT-20, translation metrics, and the T3 coverage
dashboard.

## Risks And Mitigations

- Risk: priority cards distort global implementation work.
  Mitigation: report priority and corpus-wide coverage separately; both gates
  remain binding.
- Risk: reopening CP-DSL creates open-ended or card-specific primitives.
  Mitigation: only the five approved families may change, and every operation
  remains typed, corpus-backed, reusable, and fail-closed.
- Risk: more workers create duplicate caches or I/O contention.
  Mitigation: retain one cache and one output tree, cap translation workers at
  useful core parallelism, and batch mapper changes between full sweeps.

## Approval

In the Codex thread on 2026-07-10, the Owner approved the bounded amendment and
asked that the supplied Commander coverage list be included to maximize
coverage. The Owner also approved using additional local hardware while
avoiding GitHub Actions, duplicate caches, and wasteful subagents.

## Implemented Amendment Wave

The first incorporated wave adds typed presence predicates and count
comparisons across activated, spell, static, replacement, and intervening-if
trigger contexts. It also adds typed paid-X, counter-count, devotion,
distinct-count, and history-count values, and an optional numeric amount on
`add_mana`. New operation variants are appended after the original frozen
registry to preserve every existing serialized discriminant. Structural tests,
the existing card-database compatibility suite, canonical compiler round-trips,
and the workspace coverage gate pass locally.
