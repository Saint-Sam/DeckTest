# CP-LAYERS Reviewer Oracle Execution

Date: 2026-07-07

Status: PASS FOR MODELED SUBSET

Owner approval source: Codex thread response `approve 100 scenarios`.

## Artifacts

- `tools/generate_cp_layers_reviewer_oracles.py`
- `tests/oracle/reviewer_layers/MANIFEST.md`
- `tests/oracle/reviewer_layers/cp_layers_reviewer_001_copy_ignores_source_modifier.ron`
  through `tests/oracle/reviewer_layers/cp_layers_reviewer_100_full_brutal_stack.ron`
- `reports/gates/CP-LAYERS/reviewer_oracles/MANIFEST.md`

## Commands

- `python3 tools/generate_cp_layers_reviewer_oracles.py`: PASS, wrote 100
  reviewer oracles plus evidence mirror.
- `cargo run -p forge-testkit -- lint tests/oracle/reviewer_layers`: PASS, 100
  reviewer scenarios parsed.
- `cargo run -p forge-testkit -- oracle --path tests/oracle/reviewer_layers --no-junit`:
  PASS, 100 scenarios passed and 0 failed.
- `scripts/vl.sh`: PASS after the reviewer pack was added; 482 oracle
  scenarios passed and 0 failed, coverage was 81.82% lines, and perf smoke
  reported 0 regressions.

## Coverage

The executable reviewer pack covers modeled copy/base-creature behavior,
controller changes, text markers, type add/set/remove, color set, combat-keyword
add/remove, numeric 7a-7d P/T operations, explicit same-layer dependencies,
timestamp ties, selected combat/SBA consequences, mutation/query interleaving,
and hash determinism.

## Limits

This evidence does not prove true CDA/copiable-CDA semantics, land subtypes or
intrinsic mana abilities, supertypes, all-abilities removal beyond modeled
combat keywords, legal target enumeration, automatic semantic dependency
detection, or the pending 100-card legacy differential.
