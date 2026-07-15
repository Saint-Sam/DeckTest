# ADR 0034: Canonical Target Groups and Distribution

- Status: accepted
- Date: 2026-07-15
- Scope: spell, activated-ability, and triggered-ability target announcements

## Context

One logical Magic target declaration may choose a bounded set rather than one
object, and a divided effect assigns a positive portion of a fixed total to
each chosen target. Flattening that declaration into anonymous target slots
loses its printed grouping, permits duplicate targets within one declaration,
and cannot preserve the announced division through replay and resolution.

Comprehensive Rules 601.2c and 601.2d require targets and divisions to be
announced before payment. Each recipient of a divided amount must receive at
least one. A target that later becomes illegal loses only its assigned effect;
the amount is not redistributed at resolution.

## Decision

1. `TargetRequirement` carries an append-only group identity, minimum and
   maximum cardinality, optional allocation total, and per-selection assigned
   allocation.
2. `AnnouncedTarget` is the canonical pre-kernel representation of a group,
   typed target, and optional positive allocation.
3. The interpreter accepts only statically bounded `target_range` and
   `target_allocation` expressions. Dynamic bounds, dynamic totals, invalid
   ranges, impossible positive divisions, and unsupported cross-target
   constraints fail closed.
4. The kernel validates group metadata, cardinality, distinctness within each
   printed group, exact allocation totals, and ordinary target legality before
   mutation. The same object may be selected by separate printed groups unless
   another closed predicate forbids it.
5. Human and AI trigger announcements use scoped hierarchical contexts: add
   one distinct target or finish an eligible group, then assign one bounded
   positive amount at a time. The final expanded binding is validated
   atomically by the kernel.
6. Spell and activation enumeration remains bounded by the canonical option
   cap. Grouped and divided actions use append-only descriptor codes 32-37;
   existing fixed-target descriptor encodings remain unchanged.
7. Resolution consults the kernel legality mask per selected member. Legal
   members retain their announced amounts, illegal members are skipped, and an
   optional empty group makes only its dependent effect a no-op.

## Consequences

- Range and divided semantics survive state hashing, replay, human prompts,
  AI telemetry, stack snapshots, copies, and partial target illegality.
- No card identity branch or oracle-text heuristic is introduced.
- Positive divided totals imply an effective minimum of one selected target
  even when the printed range says "up to" and carries a numeric minimum of
  zero.
- Dynamic distributions and other untyped target relationships remain explicit
  T4 adapter blockers rather than silently approximated behavior.

## Verification

- Kernel tests cover cardinality, duplicate rejection, exact allocation, and
  reuse across separate groups.
- Interpreter tests cover optional ranges, multi-member effects, partial
  legality, divided amounts, and mismatched-total rejection.
- Runner tests cover bounded combinations, positive compositions, canonical
  action identity, and hierarchical triggered distributions.
- Runtime smoke synthesizes expanded group metadata and positive allocations.
