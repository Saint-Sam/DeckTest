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

Status: open

Question: CP-LAYERS requires a legacy engine differential on a 100-card layered
subset. Local-only evidence selected the subset and adjudicated script-level
divergence categories, but a true engine-vs-engine run is blocked because Forge
2.0 has no real legacy card-script importer/card compiler yet, and the local
legacy Java engine is not runnable as-is without JDK17/Maven/dependencies.
Should CP-LAYERS remediate this before T2.5, explicitly de-scope the true
engine differential for this checkpoint, or fail/reopen?

Current evidence:
`reports/gates/CP-LAYERS/legacy-100-layered-subset-2026-07-07.md`
