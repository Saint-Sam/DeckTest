# Open Questions

# Question Queue

Append-only queue for ambiguity that survives the plan, specs, official rules,
and legacy differential evidence.

## Q-2026-07-07-T1.10

Priority: P1

Status: resolved 2026-07-07

Question: T1.10 asks for the top 100 legacy `forge-game` unit tests, but the
vendored checkout contains only 3 `@Test` methods under
`vendor/legacy-forge/forge-game/src/test/java` and no top-100 ranking metadata.
Should T1.10 use a broader source set, fetch/provide a fuller upstream legacy
test corpus, or keep the exact path and mark the missing 97 rows as blocked by
source evidence?

Resolution: Owner chose Option 1 in the Codex thread: use the broader local
`forge-gui-desktop` game-simulation tests as the T1.10 source set.

Current evidence: `docs/t1_10_legacy_test_oracle_mapping.md`

## Q-2026-07-07-CP-LAYERS-LEGACY-DIFF

Priority: P0

Status: open, partially remediated 2026-07-07

Question: CP-LAYERS requires a legacy engine differential on a 100-card layered
subset. Local-only evidence selected the subset and adjudicated script-level
divergence categories. The legacy Java engine side has now been remediated with
repo-local Corretto/Maven tooling and executed for all 100 selected cards: 100
legacy snapshots emitted, 100 OK, 0 harness errors. A narrow Forge 2.0 bridge
now parses all 100 scripts and executes 53 representable layer-fragment
scenarios with 53/53 pass; 43 generated fragments match legacy modeled fields
and 10 expose fixture/model divergence. A true end-to-end engine-vs-engine run
is still blocked because Forge 2.0 has no full legacy card-script
importer/card compiler yet. Should CP-LAYERS continue remediation before T2.5,
explicitly de-scope the true engine differential for this checkpoint, or
fail/reopen?

Current evidence:
`reports/gates/CP-LAYERS/legacy-100-layered-subset-2026-07-07.md`
`reports/gates/CP-LAYERS/legacy-engine-snapshot-2026-07-07.md`
`reports/gates/CP-LAYERS/legacy-script-bridge-2026-07-07.md`
