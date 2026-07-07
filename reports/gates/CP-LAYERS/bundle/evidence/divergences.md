# CP-LAYERS Divergences

Date: 2026-07-07

Status: BLOCKED FOR TRUE ENGINE DIFFERENTIAL

Owner decision on 2026-07-07: use local-only search for a legacy Forge/layered
subset first, and ask before any network/download. No network access, clone,
download, or upstream fetch was used.

## Local Evidence

- Local-only subset search:
  `reports/gates/CP-LAYERS/legacy-local-search-2026-07-07.md`
- Selected 100-card layered subset and script-level adjudication:
  `reports/gates/CP-LAYERS/legacy-100-layered-subset-2026-07-07.md`
- Machine-readable subset CSV:
  `reports/gates/CP-LAYERS/legacy-100-layered-subset-2026-07-07.csv`

## Result

The 100-card legacy layered subset is selected, but a true engine-vs-engine
differential cannot honestly run yet. Forge 2.0 currently has a data-only layer
substrate and RON oracle harness, but no legacy card-script importer/card
compiler capable of executing those real legacy scripts in the new engine.

## Adjudicated Divergence Categories

| Category | Count |
| --- | ---: |
| Legacy predicate targets are not modeled | 100 |
| Subtypes/supertypes beyond `ObjectTypes` are not modeled | 65 |
| All-abilities removal is not modeled beyond explicit combat keywords | 36 |
| Keywords beyond the current combat-keyword subset are not modeled | 17 |
| Land subtypes/intrinsic mana abilities are not modeled | 6 |
| Dynamic P/T expressions are not modeled | 5 |
| "Can't-have" keyword suppression is not modeled | 3 |
| Dynamic P/T modifiers are not modeled | 2 |
| Legacy copy behavior is broader than current `CopyBaseCreature` | 1 |

## Gate Consequence

CP-LAYERS cannot pass the Section 15.4 legacy differential clause without owner
review. The available choices are to reopen T2.4/T2.x remediation for missing
layer/card-import semantics, explicitly de-scope the real 100-card differential
from this checkpoint, or fail CP-LAYERS and trigger the plan's remediation path.
