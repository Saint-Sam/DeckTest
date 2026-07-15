# ADR 0029: Canonical Activated-Ability Extra Costs

Status: accepted for local T4 diagnostics on 2026-07-14.

## Context

The card compiler already emitted typed non-mana activated-ability costs for
literal life payments and exact sacrifices of matching controlled permanents.
The live runner rejected every program carrying either field, so compiler-valid
cards such as Polluted Delta and Zuran Orb had no human, AI, search, telemetry,
or replay activation path.

Adding cost fields to the established flat activation descriptor would change
ordinary activation IDs. Flattening target, optional, sacrifice, and mana-plan
products would also enlarge the root legal set and permit stale cost objects to
reach mutation code without a distinct canonical choice.

## Decision

- `ActivationCost` carries an exact life amount and matching-permanent
  sacrifice count. The activated-ability definition carries the corresponding
  closed predicate. All three values participate in deterministic state and
  clone-surface bytes.
- Existing no-extra-cost activation descriptors and IDs are unchanged. A
  compiler-declared extra-cost activation begins with append-only
  `BeginActivateProgramAbilityWithCosts` descriptor tag 30.
- Matching permanent selection uses append-only
  `ChooseActivationCostObjects` descriptor tag 31. The existing exact
  `ChoosePayment` descriptor then selects the mana plan, including the
  singleton zero-mana plan.
- Sacrifice candidates must be controlled battlefield permanents satisfying
  the compiler's closed predicate. Enumeration is canonical, exact, bounded,
  and never truncated. A source separately sacrificed by the printed cost
  cannot also pay the matching-permanent cost.
- Every offered root and partial selection must admit a complete legal kernel
  action. Human play, heuristic and random policy, determinized search,
  telemetry, and exact replay consume the same scoped contexts and typed
  mappings.
- The kernel independently validates current life, exact object count,
  uniqueness, zone, controller, predicate, and source reuse before mutating
  mana, life, tap state, loyalty, or zones. A player may pay down to zero life;
  state-based actions run when priority would next be granted.
- Successful payment follows one deterministic internal order after complete
  validation. Selected objects move to their owners' graveyards as costs, and
  the program-bound ability remains on the stack even when its source leaves.

## Consequences

The literal life and matching-permanent sacrifice families currently emitted
for non-mana activated programs are now playable through the production
decision surface. Focused regressions cover Polluted Delta, Zuran Orb, combined
life/tap/sacrifice payment, and atomic rejection of an opponent-controlled cost
object.

This does not authorize discard, exile, reveal, counter removal, variable life,
multiple independent matching-sacrifice groups, source-exclusion predicates
not represented by the compiler contract, or arbitrary legacy cost text. Those
families remain fail closed. It also does not establish AI strength or pass the
T4 promotion gate.
