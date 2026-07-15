# ADR 0025: No-Legal-Target Trigger Disposition

Status: accepted for local T4 diagnostics on 2026-07-14.

## Context

A required targeted triggered ability cannot be put on the stack when a legal
target cannot be chosen. The first canonical trigger-target adapter failed
closed at the runner when this occurred. That preserved safety, but it also
left the pending-trigger queue blocked and made an otherwise legal game unable
to continue.

Silently deleting the pending instance in the runner would bypass the kernel,
hide the disposition from the typed action stream, and make a forged
no-target claim impossible to reject atomically.

## Decision

- `TriggerStackDisposition` is the typed kernel authority for putting a pending
  trigger on the stack or removing it for no legal targets.
- `TriggerStackBinding::no_legal_targets` carries the trigger ID and its closed
  required target slots. It carries no target choices or non-target decisions.
- The kernel accepts that disposition only when the requirement list is
  nonempty and at least one required slot has no legal player, object, or stack
  choice under the same target predicates and restrictions used at
  announcement.
- Every binding in the APNAP batch is validated before the pending queue is
  consumed. A forged removal or any invalid sibling binding leaves both the
  pending queue and stack unchanged.
- Removed instances create no stack entry and no human or AI prompt. Other
  valid instances in the same batch retain their APNAP order and enter the
  stack normally.

## Consequences

Ordinary required targeted triggers no longer block the game merely because
their current closed target domain is empty. The disposition remains visible
in the typed action stream and exact replay path.

This decision does not approximate unmodeled rules. Same-batch targeting of a
triggered ability that is itself being placed on the stack, target
distribution/distinctness, per-target partial-illegality filtering, and
deferred trigger optionals remain fail closed and require separate typed
designs. No benchmark or product-strength promotion follows from this ADR.
