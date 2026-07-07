# T1 Gate Signoff

Date: 2026-07-07

Reviewer: Codex Gate Reviewer for T1. I did not build this tier.

Verdict: FAIL

## Reviewed Tree State

- Repository: `/Users/juanlopez2016/Desktop/Forge 2.0`
- Reviewed HEAD: `c9e333e` (`T1 perf: inline small legal action lists`)
- Working tree state reviewed: dirty local T1 gate-prep tree on top of `c9e333e`.
- Local modifications observed before this signoff: `PLAN_STATE.json`, `crates/forge-core/src/lib.rs`, `metrics/coverage.json`, `reports/perf/T1.13-2026-07-07.md`, `reports/status/2026-W28.md`, `scripts/gates/make_bundle.sh`, plus untracked `reports/gates/T1/` and `reports/owner/brief-T1-gate.md`.
- Latest remote CI evidence in bundle: GitHub Actions run `28872149729` passed for commit `c9e333e`.
- Exact reviewed final local tree remote CI: pending. The user explicitly instructed no network egress and no remote fresh clone.
- Product-code edits by this reviewer: none. Evidence edits by this reviewer: none. The only file I wrote is this `SIGNOFF.md`.

## Evidence Reviewed

- Plan sections reviewed: Section 15, Section 17.2, T1 exit gate, and relevant T1 task rows.
- State/evidence reviewed: `PLAN_STATE.json`, `reports/gates/T1/bundle/*`, `reports/owner/brief-T1-gate.md`, `reports/questions/QUEUE.md`, `docs/t1_10_legacy_test_oracle_mapping.md`, current git status/diffs, T1 gate script, perf scripts, and sampled code/tests/oracle scenarios.
- Local gate log evidence: `reports/gates/T1/bundle/test_log.txt` ends in `PASS gate_T1.sh`.
- Metrics evidence: 300 oracle scenarios passed, fuzz evidence for two 10,801-second target runs, clone budget 109.804 ns per 200-card state, perf diff compared 4 metrics at 5.0% threshold with 0 regressions, arena smoke 10,000 games with 0 invariant violations.
- Perf threshold/baseline review: I found no current diff to `metrics/perf_baseline.json`, `scripts/perf_smoke.sh`, `tools/perf_diff.py`, or `scripts/gates/gate_T1.sh` changing the 5.0% regression threshold. The final local perf diff in `crates/forge-core/src/lib.rs` is code-side hot-path remediation.

## Checklist

| Gate | Result | Review notes |
| --- | --- | --- |
| G1 Gate script green from scratch | FAIL | The provided `test_log.txt` is green, but it is not reviewer-executed from a fresh clone. The final reviewed tree is dirty/local-only and exact-tree remote CI is pending. User instructions prohibited network egress and remote fresh clone, so this cannot be passed. |
| G2 Exit metrics meet targets | PASS | T1 exit metrics meet the top-level gate targets in the bundle: 300 scenarios >=250, fuzz evidence >=6 h aggregate, clone 109.804 ns <=200 ns, CLI demo/roundtrip present. No perf baseline or threshold relaxation found. |
| G3 Test-quality audit | FAIL | Sampled unit tests are mostly meaningful, but `scripts/review/mutation_check.sh` was not run because the review was constrained to write only this file. More importantly, the oracle corpus has a major task-level coverage gap: see G8. |
| G4 Adversarial probe | FAIL | Section 15.3 requires the reviewer to author and run at least 5 novel oracle scenarios. I did not create or run new scenarios because the task forbade writes other than this signoff. This gate cannot pass without that probe. |
| G5 Blocker and quarantine hygiene | FAIL | Bundle says no blockers and no quarantines. However `docs/t1_10_legacy_test_oracle_mapping.md` records a combat/lifelink legacy row as `blocked_schema`, and the T1.6 plan row requires combat oracle scenarios. That should have remained visible as a blocker/QID/remediation item. |
| G6 Determinism and invariants | FAIL | Historical evidence is strong: fuzz report is clean and arena smoke passed 10,000 games. But I did not re-run `scripts/review/determinism.sh` or a live 1-hour fuzz session during review, both required by Section 15.3. |
| G7 Spot play | FAIL | The bundle contains 10 replay files for seeds 11-20 and a README with final hashes/outcomes. I did not step-replay 3 games during this review, so the required spot-play check is not satisfied. |
| G8 ADR/spec consistency | FAIL | T1.6 explicitly requires `>=60 combat oracle scenarios incl. double-block ordering, trample+deathtouch`. `find tests/oracle -name '*.ron'` reports 300 total scenarios, but grep found no oracle files exercising attack/block/combat-damage/trample/deathtouch/lifelink/double-strike actions. Combat exists in Rust unit tests, but the plan called for oracle scenarios. This is undocumented drift. |
| G9 Question Queue clear | PASS | `reports/questions/QUEUE.md` has only `Q-2026-07-07-T1.10`, resolved 2026-07-07 by owner choice. Caveat: the missing combat oracle/schema blocker should have been represented there or as a blocker. |
| G10 Scope integrity | PASS | Current diff is confined to T1 perf remediation, gate evidence/status, metrics coverage, and bundle assembly. I did not find stealth edits to earlier-tier gates/tests. The dirty local tree still blocks G1/exact-tree CI. |
| G11 Owner Brief delivered | PASS | `reports/owner/brief-T1-gate.md` exists, contains a TRY-IT section, includes `WHAT YOU SHOULD EXPECT NEXT` and `WHAT WE NEED FROM YOU`, and records the exact remote-CI-pending caveat. ADR-0008 names this Codex thread as the owner channel. |

## Remediation Tickets

T1.R5 - Restore T1.6 oracle coverage. Add and pass at least 60 combat oracle scenarios, including double-block ordering and trample plus deathtouch, as required by the T1.6 plan row. If the scenario DSL cannot express combat setup/actions, extend `forge-testkit` first and include parser/runner tests.

T1.R6 - Convert the hidden combat/schema gap into visible project hygiene. Record the current `blocked_schema` combat/lifelink gap as a blocker or Question Queue item, then close it only with linked code/scenario evidence.

T1.R7 - Produce exact-tree gate evidence. Commit the final local remediation/evidence tree, run `scripts/gates/gate_T1.sh` from a fresh clone or otherwise reviewer-commanded clean checkout, and archive that log in the T1 bundle.

T1.R8 - Complete the Section 15 reviewer live checks. In re-review, run the G3 mutation checks on three sampled tests, author and run five novel Gate Reviewer oracle scenarios with at least four passing, re-run `scripts/review/determinism.sh`, perform the required 1-hour fuzz live check, and step-replay at least three bundle replays.

T1.R9 - Obtain exact final-tree remote CI. After the final local tree is pushed, obtain a green GitHub Actions run for that exact commit and add the run id/result to T1 evidence.

## Closing Note

This is a gate failure, not a rollback request. The local implementation has substantial green evidence, and the perf remediation appears real rather than threshold-based. The blocking issue is that the gate bundle proves the top-level count, but not the plan-required combat oracle surface, and the Section 15 reviewer procedures were not completed under the constraints of this review.
