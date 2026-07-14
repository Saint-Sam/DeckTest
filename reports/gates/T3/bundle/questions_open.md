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

Status: resolved/remediated 2026-07-08; owner signed off CP-LAYERS

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

Resolution: Owner chose remediation rather than de-scope. Codex added a
local true importer differential for the selected CP-LAYERS 100-card subset.
It parses the active legacy card face, recreates the Java snapshot fixture with
stable object roles, applies layer-ordered continuous effects, and matches the
vendored legacy Java engine snapshots for 100/100 selected scripts with 0
mismatches.

Current evidence:
`reports/gates/CP-LAYERS/legacy-100-layered-subset-2026-07-07.md`
`reports/gates/CP-LAYERS/legacy-engine-snapshot-2026-07-07.md`
`reports/gates/CP-LAYERS/legacy-script-bridge-2026-07-07.md`
`reports/gates/CP-LAYERS/legacy-true-importer-diff-2026-07-08.md`

Owner signoff: 2026-07-08, Codex thread:
`CP-LAYERS signoff: proceed. I approve CP-LAYERS based on the 100
owner-approved reviewer scenarios passing, the 100-card true importer legacy
differential matching 100/100 with 0 mismatches, fuzz/local verification
passing, and no current blocking divergences.`

## Q-2026-07-10-PC-0007

Priority: P0

Status: resolved 2026-07-10

Question: Should the project ratify the linked dual-track plan changes in
`docs/plan-changes/PC-0007-local-trainer-and-grand-plan-bridge.md` and Grand
Plan `GP-PC-0001-dual-track-engine-and-human-teacher.md`? The proposed change
preserves standalone Forge, keeps PodBench report-only, adds truthful card
maturity, semantic/four-player/human-play checkpoints, brings forward the
focused Trainer, and creates a governed human-teacher data bridge while
retaining no GitHub Actions and all existing GPL/IP/egress gates.

Companion Owner IP/repository-placement decision: should the full Grand Plan
package remain private/outside public DeckTest (recommended), or should its
business/research volumes be deliberately published? Intake alone does not
authorize copying those volumes into the public GPL repository.

Required evidence:

- `docs/transition/GRAND_PLAN_V2_INTAKE.json`
- `docs/transition/GRAND_PLAN_V2_CONFLICT_MATRIX.md`
- `docs/transition/PC-0007_IMPLEMENTATION_SEQUENCE.md`
- `reports/owner/brief-grand-plan-v2-intake.md`
- `reports/gates/ADR-0012/EVIDENCE.json`

Resolution: Owner approved exact PC-0007 SHA
`8a63ec707562fd50353b028c27152dd215186b014c35af919e7c870f2a02aed6`
and GP-PC-0001 SHA
`9972644f84fc2f8f4f501ec450d2ae9df5ed72560556b3fb521b7290a94322b9`.
The full Grand Plan/business/private research package remains private outside
public DeckTest; only sanitized bridge/intake/evidence records may enter the
public repository. The Owner performs or explicitly approves GitHub egress.
Master plan v1.8 incorporates the accepted bridge.
