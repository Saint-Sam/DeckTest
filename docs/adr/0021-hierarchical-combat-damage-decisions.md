# ADR 0021: Hierarchical Combat-Damage Decisions

Status: accepted for local T4 diagnostics on 2026-07-14.

## Context

The production runner automatically assigned all combat damage to the first
legal target. `DecisionKind::CombatDamage` and an amount descriptor existed,
but neither human nor AI controllers received a live ordering or assignment
prompt. Complete damage distributions form a combinatorial product, and a
single creature can also have a very large numeric damage range.

The kernel already enforces the current rules model's lethal-to-advance and
trample constraints. The controller needs those constraints through a typed
read-only boundary, while the kernel must remain the final legality authority.

## Decision

- The kernel exposes a read-only `CombatDamageChoiceProfile` containing the
  eligible source, exact legal targets, total damage, and per-position minimum
  needed before a later target may receive damage.
- The ordered assignments in `CombatDamageAssignmentRequest` define the
  selected blocker order. The kernel validates their exact legal target set,
  keeps a defending player last, and rejects illegal lethal-to-advance or
  trample distributions before mutation.
- Human and AI controllers choose one next blocker at a time in scoped
  canonical contexts. The defending player, when legal through trample,
  remains last automatically.
- Damage amounts are chosen one target at a time. Each later prompt carries
  the selected order and prior amounts in its typed path identity; the final
  target receives the exact remainder.
- Direct amount contexts contain at most 64 values. Larger inclusive ranges
  are split into two canonical subranges until a bounded direct context is
  reached, preserving every `u32` outcome without a large allocation.
- Forced values and singleton targets bypass policy work. Autonomous legacy
  play retains deterministic first-target assignment.
- One final typed `AssignCombatDamage` action carries every source request and
  is revalidated by the kernel before damage is dealt.

## Consequences

Every legal order and amount remains reachable without materializing complete
distributions. Order contexts shrink linearly with remaining blockers, while
large numeric ranges require logarithmic bounded prompts. Human and AI paths
share canonical membership, replay identity, telemetry, and fail-closed kernel
validation.

Exact AI replays must be regenerated because games with meaningful multi-block
damage choices now contain canonical decisions. Sealed benchmark fixtures and
independent acceptable-action labels remain open, so this decision completes
the live adapters but not CP-AI-BENCH promotion evidence.
