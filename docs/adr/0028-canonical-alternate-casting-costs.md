# ADR 0028: Canonical Alternate Casting Costs

Status: accepted for local T4 diagnostics on 2026-07-14.

## Context

The card compiler already emitted typed programs for four alternate casting
costs: commander-presence costs, flashback, evoke, and overload. The live game
runner offered only printed mana costs, so these compiled branches were absent
from human play, AI, search, telemetry, and exact replay. Evoke also carried a
conditional sacrifice trigger that the runner previously registered without
proving the spell had been evoked.

Adding an alternate flag to the existing ordinary cast descriptor would change
every established canonical action ID. Flattening alternate selection together
with targets, additional costs, X, and mana plans would also recreate the cast
Cartesian product that the bounded hierarchy is designed to avoid.

## Decision

- `SpellAlternateCost` is the closed kernel meaning for Commander, Flashback,
  Evoke, and Overload. The selected value is stored in
  `StackDecisionBindings`, canonical state bytes, hashes, stack copies, and
  resolution records.
- Existing ordinary cast descriptors and IDs are unchanged. Alternate casts
  enter through append-only `BeginCastSpellAlternate` descriptor tag 29, then
  use the same scoped additional-cost, X, and exact-payment contexts as printed
  costs. The selected alternate participates in every scoped path identity.
- The runner offers only compiler-declared alternate programs whose closed
  state condition is currently true. Commander, evoke, and overload sources
  must be in the caster's hand; flashback sources must be in that player's
  graveyard. Normal timing rules still apply.
- `GameState::cast_spell` independently rejects invalid generic source
  conditions. Commander costs require a controlled designated commander,
  flashback requires a matching graveyard cast and flashback flag, and evoke or
  overload requires a hand-zone source. The adapter remains responsible for
  proving that the selected card program actually declares the alternate.
- Overload announces no targets and binds the compiler's each-object effect at
  resolution. Flashback uses the alternate mana cost and the kernel's existing
  exile-after-resolution rule. Commander and evoke use their compiled exact
  mana costs.
- Triggers with `required_alternate_cost` are never registered as ordinary
  persistent triggers. A matching cast registers a delayed-once subscription.
  Successful entry consumes it; a countered or otherwise non-entering spell
  unregisters it. This prevents normal casts and later unrelated entries from
  inheriting an evoke sacrifice.
- Human, heuristic, random, search, telemetry, and replay consumers all receive
  the same canonical root and scoped payment contexts. No consumer may infer an
  alternate from card text or display labels.

## Consequences

The four alternate-cost families currently emitted by the compiler are now
playable through the production decision surface and remain distinguishable in
state keys and exact replay. Focused real-definition tests cover conditional
Commander availability, graveyard flashback and exile, overload target removal
and each-object resolution, normal versus evoke trigger registration, and
countered-evoke cleanup.

This does not authorize arbitrary alternative costs, casting without paying a
mana cost, non-mana evoke costs, optional alternate-cost riders, or any value
outside the compiler's closed programs. Those cases remain fail closed. It also
does not establish AI strength or pass the T4 promotion gate.
