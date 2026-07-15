# ADR 0019: Hierarchical Resolution Object Choices

Status: accepted for local T4 diagnostics on 2026-07-14; amended on 2026-07-15
to include spell resolution.

## Context

Spell, activated, and triggered interpreter effects may ask for several
ordered object choices while resolving. The production runner previously enumerated each
slot and multiplied the results into one complete Cartesian product before it
could present any decision. The fail-closed option cap was honest, but a legal
effect with several broad searches could exceed it even when each individual
prompt was modest.

The canonical decision protocol requires complete options, exact replay, and
hidden-safe path identity without silently dropping legal selections.

## Decision

Resolution object choices are represented as one scoped canonical context per
compiled choice slot:

- each context contains every legal selection for exactly one requirement;
- prior selections remain typed in every subsequent descriptor;
- a visible-path discriminator binds the slot and prior object selections into
  the context and benchmark state keys;
- interpreter actions are bound only on the final slot, after every required
  selection is complete;
- spell choices are deferred until successful resolution just like activated
  and triggered choices, so countered spells never expose or consume them;
- human, AI, autonomous, telemetry, and replay paths consume the same contexts;
- authoritative interpreter predicates and legal fail-to-find branches remain
  unchanged.

## Consequences

Context width is the largest individual slot rather than the product of all
slots. A regression with two nine-option searches now presents two nine-option
contexts instead of materializing 81 complete branches. Exact replays must be
regenerated because scoped context and state keys intentionally change.

This decision does not by itself complete spell-announcement choices, trigger
targets, deferred optionals, or multi-zone benchmark fixtures. Those remain
explicit T4 blockers.
