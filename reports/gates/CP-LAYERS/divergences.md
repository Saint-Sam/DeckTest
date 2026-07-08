# CP-LAYERS Divergences

Date: 2026-07-07

Status: PARTIALLY REMEDIATED; TRUE ENGINE DIFFERENTIAL STILL BLOCKED

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
- Legacy engine snapshot:
  `reports/gates/CP-LAYERS/legacy-engine-snapshot-2026-07-07.md`
- Machine-readable legacy snapshot:
  `metrics/cp_layers_legacy_engine_snapshot.jsonl`
- Legacy-script bridge:
  `reports/gates/CP-LAYERS/legacy-script-bridge-2026-07-07.md`
- Generated Forge 2.0 fragment oracles:
  `tests/oracle/legacy_layers`
- Machine-readable bridge summary:
  `metrics/cp_layers_legacy_script_bridge.json`

## Result

The 100-card legacy layered subset is selected and the legacy Forge Java engine
now runs locally for all 100 cards: 100 snapshots emitted, 100 OK, 0 legacy
harness errors. A narrow Forge 2.0 bridge now parses all 100 selected scripts
and emits the representable fragments as executable RON scenarios: 53 scenarios
generated, 53 passed, 0 failed. Of those generated fragments, 43 match the
legacy snapshot on currently modeled fields and 10 differ because the bridge
fixture/model does not yet capture the full legacy card behavior.

A true engine-vs-engine differential still cannot honestly run yet because
Forge 2.0 has no full legacy card-script importer/card compiler capable of
executing those real legacy scripts end to end in the new engine.

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
review or further new-engine importer/compiler remediation. The available
choices are to continue remediation for missing layer/card-import semantics,
explicitly de-scope the real 100-card engine-vs-engine differential from this
checkpoint, or fail CP-LAYERS and trigger the plan's remediation path.
