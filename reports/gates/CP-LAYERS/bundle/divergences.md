# CP-LAYERS Divergences

Date: 2026-07-07

Status: REMEDIATED AND OWNER-SIGNED-OFF

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
- True importer differential:
  `reports/gates/CP-LAYERS/legacy-true-importer-diff-2026-07-08.md`
- Machine-readable true importer differential:
  `metrics/cp_layers_true_importer_diff.json`
- True importer predicted snapshots:
  `metrics/cp_layers_true_importer_diff_predicted.jsonl`

## Result

The 100-card legacy layered subset is selected and the legacy Forge Java engine
runs locally for all 100 cards: 100 snapshots emitted, 100 OK, 0 legacy harness
errors. The supplemental bridge still emits 53 executable Forge 2.0
legacy-fragment scenarios, all passing.

The true importer differential now remediates the previous blocker for this
checkpoint: it parses the active face of each selected legacy script, recreates
the Java harness fixture with stable object roles, instantiates 186
layer-ordered operations from 117 active-face continuous lines, and matches the
legacy Java snapshots for 100/100 selected scripts with 0 role-field
mismatches.

## Adjudicated Diagnostic Categories

| Category | Count |
| --- | ---: |
| Non-snapshot-visible ability text imported as diagnostics | 13 |
| Empty fixture selector matched the legacy Java harness result | 39 |
| Condition false in the CP-LAYERS fixture | 5 |
| Stable-role snapshot mismatches after true importer remediation | 0 |

## Gate Consequence

The Section 15.4 legacy differential clause has PASS evidence. The owner/human
reviewer signed off in the Codex thread on 2026-07-08, so CP-LAYERS is unblocked
and T2.5 may start.
