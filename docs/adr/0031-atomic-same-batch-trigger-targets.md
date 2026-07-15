# ADR 0031: Atomic Same-Batch Trigger Targets

- Status: accepted
- Date: 2026-07-15
- Scope: T4 canonical target and trigger-order adapters

## Context

Simultaneous triggered abilities are ordered in APNAP order and then placed on
the stack bottom to top. Choices for each ability are made as that ability is
placed on the stack. A later ability in the same batch can therefore target an
earlier ability that is already below it.

The prior runner selected every target against the pre-batch state, and the
kernel validated every binding before creating any stack entry. That was
atomic, but it made legal same-batch stack targets invisible. Silently removing
such a trigger or choosing a different target would violate the shared human,
AI, benchmark, and replay contract.

## Decision

For a trigger batch containing a stack-target requirement:

1. The runner predicts only the deterministic IDs that preceding, non-removed
   bindings will receive in the same atomic action.
2. Canonical target contexts include those prior prospective entries when the
   requirement accepts any stack entry. Forward entries are never offered.
3. The kernel clones the state, clears the original pending batch in the staged
   state, validates and pushes bindings sequentially, and commits the staged
   state only after every binding succeeds.
4. No priority window or external mutation occurs between staged entries.
5. A forward reference, mismatched binding, or illegal no-target disposition
   rejects the entire action and leaves the original state unchanged.

The common trigger path without stack-target requirements retains the existing
prepare-then-commit implementation and does not pay the staged-clone cost.

## Consequences

- Human and AI controllers see the same legal same-batch target context.
- Exact actions and replays retain concrete `StackEntryId` targets.
- The kernel, not the controller, remains the final legality authority.
- Predicted IDs are scoped to one immediate atomic batch; they do not reserve an
  ID or authorize arbitrary raw handle construction.
- Target distribution, modal triggers, and sealed benchmark labels remain
  separate T4 work and receive no completion claim from this decision.

## Verification

- A kernel regression rejects a forward reference without mutating state and
  accepts a later trigger targeting the prior staged entry.
- A production runner regression exercises shared human and AI ordering and
  target contexts over two real compiled triggered abilities.
- Workspace format, strict lint, tests, and exact replay gates remain required
  before the product checkpoint advances.
