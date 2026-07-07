# CP-LAYERS Scenario Interview

Date: 2026-07-07

Purpose: collect owner-authored scenario intent, then convert it into 15
reviewer scenarios for owner approval before any CP-LAYERS pass.

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

## Draft Scenario Allocation

After the owner answers, Codex will draft 15 approvable scenarios in this mix:

- 3 ability-removal or ability-gain scenarios.
- 3 type/color/text scenarios.
- 3 P/T sublayer scenarios covering 7a, 7b, 7c, and 7d where applicable.
- 2 copy/copiable-value scenarios.
- 2 timestamp or deterministic tie scenarios.
- 2 same-layer dependency or dependency-cycle scenarios.

The allocation may change if the owner's answers point at a better risk mix.
