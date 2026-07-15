# ADR 0027: Canonical Additional Spell Costs

Status: accepted for local T4 diagnostics on 2026-07-14.

## Context

The card compiler already emitted typed additional spell costs for discarding
cards and sacrificing matching permanents. The live runner rejected every spell
carrying either requirement, so those cards could compile without presenting a
real casting decision to humans, AI, search, telemetry, or exact replay.

Flattening every target, mode, optional answer, additional-cost selection, X
value, and mana plan into one root action would create an impractical Cartesian
product. Paying costs in the adapter before the kernel accepted the complete
cast would also permit partial state mutation after an invalid announcement.

## Decision

- A spell with a compiled discard or sacrifice cost exposes one deferred cast
  option at the ordinary priority or main-phase surface.
- The runner opens one scoped `Payment` context per additional-cost group in
  printed order. Each option uses the append-only `ChooseAdditionalCost`
  descriptor and contains the exact selected object IDs.
- Candidate enumeration is typed and fail closed. Discard candidates come from
  the caster's hand and exclude the spell. Sacrifice candidates are controlled
  battlefield permanents satisfying the compiler's closed predicate. An object
  cannot pay more than one cost group.
- Each offered partial selection must admit at least one complete remaining
  payment. This removes dead-end branches without materializing the product of
  all cost groups.
- After additional costs, the existing hierarchy chooses printed X when needed
  and then an exact mana payment. Human, heuristic, random, search, telemetry,
  and decision replay consume the same scoped contexts and typed mappings.
- `GameState::cast_spell` is authoritative. It validates targets, mana, every
  selected additional-cost object, exact zone/controller predicates, and
  duplicate use before mutating state. It then pays additional costs in the
  declared order, pays mana, and moves the spell to the stack.
- Existing canonical IDs for spells without additional costs remain unchanged.
  Additional selections enter only the hierarchical path identity for spells
  that actually carry those costs.

## Consequences

The currently compiled discard-card and sacrifice-permanent spell costs are no
longer silent or structurally unavailable. Their choices are inspectable,
replayable, searchable, and rejected atomically when stale or invalid.

The bounded option cap remains fail closed; it never truncates a legal set.
Alternate costs and uncompiled life, tap, reveal, exile, optional, or other
non-mana cost families remain open. Legacy autonomous compatibility helpers do
not guess additional-cost payments. No AI-strength or T4 promotion claim
follows from this diagnostic adapter.
