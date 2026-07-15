# ADR 0032: Resolution Target Legality Mask

- Status: accepted
- Date: 2026-07-15
- Scope: T4 canonical target adapter and interpreter resolution

## Context

The kernel already snapshots every announced target and records a per-slot
legality mask when a stack entry resolves. It counters the entry when every
target is illegal and otherwise resolves it. The interpreter previously
revalidated the complete target vector after that point. One illegal target
therefore rejected the whole compiled effect program even when the kernel had
correctly allowed the stack entry to resolve through a different legal target.

That fail-closed behavior prevented illegal effects, but it also suppressed
independent instructions that Magic rules require to continue. Recomputing or
guessing legality in the controller would create a second rules path.

## Decision

1. The production runner copies the target choices and exact legality mask from
   the kernel `ResolutionRecord` into every pending spell, activated-ability,
   and triggered-ability interpreter binding.
2. Announcement-time bindings carry no mask and continue to validate every
   target normally.
3. A resolution mask must have exactly one entry per target. A nonempty
   all-illegal mask is rejected because the kernel must counter that stack
   object before interpreter execution.
4. Every compiled effect declares its target dependencies structurally through
   its typed player, object-set, amount, or direct-target bindings. An effect
   depending on an illegal slot is skipped; effects depending only on legal
   targets or no targets continue in source order.
5. The kernel remains the authority for the original legality snapshot. The
   interpreter does not turn an illegal target into a legal one or invent last
   known information.

## Consequences

- Multi-target spells and abilities can resolve their legal and untargeted
  instructions without applying an effect to an illegal target.
- All-target-illegal countering remains unchanged in the kernel.
- New `EffectProgram` variants must be added to the exhaustive dependency
  match, so an unclassified target dependency cannot silently compile.
- Cross-target last-known-information rules that are not represented by the
  current typed amount grammar remain fail closed and require a separate ADR.
- Target distribution, modal activated/triggered abilities, and sealed
  benchmark labels remain separate T4 work.

## Verification

- A runtime regression supplies a two-slot kernel legality mask, proves only
  the legal target's instruction binds, and rejects truncated or all-illegal
  masks.
- A production runner regression casts a real compiled two-target spell,
  removes one target before resolution, observes the kernel mask
  `[false, true]`, skips damage to the illegal target, and still applies the
  independent legal-player instruction.
- Workspace format, strict lint, tests, and exact replay gates remain required
  before the product checkpoint advances.
