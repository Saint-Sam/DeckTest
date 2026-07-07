# CP-LAYERS Scenario Interview

Date: 2026-07-07

Purpose: collect owner-authored scenario intent, then convert it into reviewer
scenarios for owner approval before any CP-LAYERS pass.

## Interview Prompts

Answer these in plain English. Short answers are fine.

1. Which three interactions feel most important to stress: Humility-style
   ability removal, Opalescence-style type plus P/T, Blood Moon/Song of the
   Dryads land typing, Clone/copy values, control-changing effects, timestamp
   ties, same-layer dependency, or something else?
2. For a Humility-style case, what outcome should be considered correct when a
   creature both gains an ability and later loses all abilities?
3. For an Opalescence-style case, should an enchantment becoming a creature keep
   its existing abilities unless a separate layer-6 effect removes them?
4. For a copy case, should the copy take only copiable/base values, or should it
   include later buffs and debuffs?
5. For a land-type replacement case, should nonbasic lands lose old land types
   and gain the new mana ability behavior, or only show the new type line?
6. For timestamp ties, should Forge 2.0 use deterministic id order as a
   tie-breaker when timestamps are equal?
7. For same-layer dependency, should dependency beat timestamp when one effect
   changes whether another effect exists, applies, or changes what it does?
8. Are any outcomes above intentionally different from legacy Forge behavior?

## Owner Answers

Source: Codex thread owner response on 2026-07-07.

1. Layer interactions: cover literally anything possible, the same way legacy
   Forge handles the layer system.
2. Card representation: the owner wants the layer system to be designed toward
   representing all roughly 100k cards, not only a small named-card sample.
3. Scenario style: synthetic rules stress tests are preferred; the owner asked
   why the reviewer packet should stop at 15 and requested 100 scenarios for a
   closer-to-bulletproof review.
4. Bug tolerance: ideally zero bugs. If any issue is tolerated, it should be
   text-only or a visual coloring artifact, not a rules-observable layer,
   targeting, state-based-action, or gameplay bug.
5. Difficulty: brutal.

## Approved Scenario Allocation

The original CP-LAYERS minimum was 15 novel reviewer scenarios with at least 14
passing. The owner requested a stricter packet: 100 brutal scenarios and no
rules-observable failures. Text-only or visual coloring artifacts are
non-blocking only if they cannot change game rules, legal actions, targets,
state-based actions, combat, zones, controller, or deterministic replay.

The owner approved the 100-scenario packet in the Codex thread on 2026-07-07
with `approve 100 scenarios`. The approved packet is recorded in:

- `reports/gates/CP-LAYERS/reviewer-scenarios-2026-07-07.md`

The 100 scenarios are not a claim that 100 examples literally cover all roughly
100k cards. They are a reviewer-level rules-family stress packet. Full
all-card confidence requires a later corpus-driven generation/differential pass
that maps imported card text/IR patterns onto these layer families.
