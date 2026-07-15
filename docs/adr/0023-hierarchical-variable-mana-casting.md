# ADR 0023: Hierarchical Variable-Mana Casting

Status: accepted for local T4 diagnostics on 2026-07-14.

## Context

Printed `{X}` costs compiled fail-closed, and the runner had no production
`NumericValue` adapter. A first implementation multiplied every X value by
every payment, target, mode, and optional branch in the root main or priority
context. That approach made legal-action size proportional to a Cartesian
product and imposed a hard cap on otherwise representable numeric values.

T4 requires human, heuristic, random, search, replay, and benchmark consumers
to see the same legal decisions. The final kernel action must still bind the
announced value and exact payment, including when a generic cost modifier
changes the payable amount.

## Decision

- `{X}` contributes zero to printed mana value and compiles into a typed
  `ManaCost` with its exact X-symbol count. Independent variable symbols remain
  fail-closed.
- Each legal target, mode, and optional binding for a printed-X spell enters a
  deferred `BeginCastSpell` option. This append-only descriptor uses canonical
  tag 26 and carries no executable action.
- The next scoped `NumericValue` context exposes every affordable announced X.
  Up to 64 values are direct `ChooseNumber` options. Larger inclusive ranges
  split into two `ChooseNumberRange` options, using append-only tag 27, until a
  bounded direct context is reached.
- The selected X leads to a scoped `Payment` context containing only legal
  `ChoosePayment` options. Each option maps to the complete typed `CastSpell`
  action with the prior target, mode, optional, X, and payment bindings.
- Affordable-X discovery is monotonic and uses bounded binary search over the
  representable `u32` range. Cost arithmetic overflow is an unaffordable bound,
  while unrelated state errors remain fail-closed.
- Human prompts, exact human replay, heuristic and random policy selection, and
  main-phase search all traverse these same contexts. Search treats the
  deferred stages as states in the existing domain instead of evaluating an
  empty root action as if casting had already occurred.
- A payment retains announced-X metadata even when a generic reducer consumes
  part of X's payable contribution. The kernel rejects a request whose cost and
  payment announce different X values and validates the final paid mana against
  the effective cost before mutation.
- Existing fixed-cost cast descriptors and the frozen legacy human replay path
  remain unchanged.

## Consequences

Printed-X casting no longer multiplies numeric values by payment plans in the
root action set, every legal value remains reachable with logarithmic range
narrowing, and the stack records the exact announcement. Canonical IDs, path
discriminators, telemetry, replay, and hidden-information behavior have focused
regression coverage.

This is a partial `NumericValue` and `Payment` adapter, not a claim that every
numeric rules prompt is complete. Variable activated abilities, target
distribution, additional or alternate costs, non-mana payments, and sealed
Track B benchmark labels remain open.
