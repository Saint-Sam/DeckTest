# ADR 0026: Canonical Trigger Resolution Choices

Status: accepted for local T4 diagnostics on 2026-07-14.

## Context

Compiled triggered abilities already represented optional effect groups and
`unless_paid` branches, but the game runner silently accepted every triggered
optional and silently declined every unless payment. Those defaults let games
continue while hiding real player decisions from human play, AI, telemetry,
and exact decision replay.

An unless payment also belongs to the player carried by the triggering event,
not necessarily the trigger controller. A subscription ID can produce several
simultaneous instances, so recovering the payer from only the trigger ID would
merge distinct events and become incorrect in multiplayer games.

## Decision

- Triggered optional effects are chosen while the ability resolves. One scoped
  `Optional` context is emitted per compiled optional group, and prior answers
  participate in the path discriminator. The adapter does not materialize a
  Cartesian product.
- Trigger optionals are no longer announced in `StackDecisionBindings`. A
  nonempty announcement-time optional payload now fails closed in the runtime
  adapter.
- The queued trigger's event sequence and turn identify its exact source event.
  For the currently compiled `OpponentDrawsCard` family, the runner recovers
  the `CardDrawn` player before the pending batch is consumed and binds that
  player to the exact stack-entry ID returned by the kernel. Search clones keep
  the same typed association; resolution or countering removes it.
- An `unless_paid` resolution first exposes a canonical pay-or-decline
  `Optional` context to that event-bound player. If the cost is unaffordable,
  decline is the sole legal action and receives forced-action telemetry.
- Accepting payment opens a separate `Payment` context containing every exact
  plan enumerated from the payer's current mana pool. The selected production
  `PayMana` action suppresses the wrapped effect. Declining continues through
  any trigger optionals and resolution-time object choices before binding the
  wrapped effect.
- Human, random, heuristic, autonomous compatibility, telemetry, and replay
  consumers use the same canonical contexts and membership validation. No AI
  receives the trigger controller's view when another player is the payer.

## Consequences

Triggered "may" effects and the currently compiled draw-trigger unless costs
are no longer hidden defaults. Payment legality is evaluated at resolution,
after intervening priority, and colored mana can satisfy a generic unless cost
through the kernel's ordinary payment enumerator.

This ADR does not generalize event-bound players beyond the closed compiled
draw family. New trigger event families must expose exact event provenance and
fail closed until their payer binding is implemented. Modal triggered
abilities, optional costs, same-batch inter-trigger targeting, target
distribution, and sealed benchmark labels remain open. No AI-strength or T4
promotion claim follows from these diagnostic adapters.
