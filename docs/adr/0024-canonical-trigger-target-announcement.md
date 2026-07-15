# ADR 0024: Canonical Trigger Target Announcement

Status: accepted for local T4 diagnostics on 2026-07-14. The no-legal-target
limitation is resolved for the current typed target model by ADR 0025.

## Context

Compiled triggered abilities could carry typed target requirements, but the
runner rejected them because pending triggers entered the stack with no target
snapshots. Choosing a target at resolution would be rules-incorrect: targets
must be announced when the triggered ability is put on the stack, remain
visible while players receive priority, and participate in resolution-time
legality checks.

Complete target products also scale poorly when an ability has multiple target
slots. Human, AI, telemetry, and replay consumers need one canonical boundary
without bypassing the kernel's target predicates or protection rules.

## Decision

- The append-only `TriggerStackBinding` and
  `PutPendingTriggeredAbilitiesOnStackWithChoices` action carry one ordered
  binding per pending trigger from stack bottom to top.
- The kernel validates APNAP order, target count, predicates, restrictions, and
  target snapshots for every binding before consuming any pending trigger.
  One invalid binding therefore leaves the entire operation unchanged.
- The runner exposes one scoped `Target` context per compiled target slot.
  Each selected target is retained in the typed path discriminator, and only
  the complete binding is dispatched to the kernel.
- Human, random, heuristic, search-fallback, autonomous, telemetry, and replay
  paths consume the same contexts and canonical IDs.
- Resolution receives the stack's exact target binding. The kernel continues
  to counter an ability whose targets are all illegal on resolution.

## Consequences

Ordinary targeted triggers now use the same announcement and resolution
boundary as spells and program-bound activated abilities without materializing
a cross-slot Cartesian product. Existing untargeted triggers retain their
previous action stream.

ADR 0025 adds the explicit kernel no-stack disposition for a required target
slot with no legal choice. Target distribution, same-batch inter-trigger stack
targeting, per-target partial-illegality effect filtering, deferred trigger
optionals, and sealed benchmark labels remain open. This ADR does not declare
the target decision family promotion-complete.
