# OWNER BRIEF - T2 Full Rules Gate

Date: 2026-07-09

## 1. WHAT JUST HAPPENED

Tier 2 now has the full rules-engine layer needed before the card factory:
events, triggers, replacement/prevention, continuous effects, activated
abilities, targeting/restrictions, counters, tokens, copy semantics,
multiplayer/Commander hooks, keyword wave one, and the nightmare deck
integration suite.

The T2 exit gate passed locally after adding the generated T2 gate oracle pack.
That brought the checked oracle corpus to 1,200 scenarios. The evidence commit
also passed remote GitHub Actions CI.

## 2. WHAT YOU SHOULD SEE - TRY IT YOURSELF

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && scripts/run_oracle.sh --all`
- EXPECT: `oracle scenarios: 1200 passed, 0 failed`.
- RED FLAG: fewer than 1,200 scenarios, skipped scenarios, or any failure.

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && scripts/run_nightmare_suite.sh 1000 6`
- EXPECT: `PASS nightmare suite: 1000 game(s), 10 fixture(s), 0 invariant violations`.
- RED FLAG: any invariant violation or panic.

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && FORGE_T2_RUN_FUZZ=1 scripts/gates/gate_T2.sh`
- EXPECT: this is the full gate and takes roughly 12 hours; it ends with `PASS gate_T2.sh`.
- RED FLAG: any sanitizer summary, crash artifact, oracle failure, or missing fuzz run.

## 3. NUMBERS THAT MATTER

- 1,200 oracle scenarios passed, 0 failed.
- 622 generated T2 gate scenarios were added to hit the exit target.
- Nightmare suite passed 1,000 games across 10 curated layer-heavy fixtures.
- T2 gate fuzz passed all three targets:
  - `fuzz_apply`: 5,469,499 runs.
  - `fuzz_characteristics`: 1,770,840 runs.
  - `fuzz_scenarioparse`: 1,559,087,053 runs.
- Current coverage report after the gate shows 81.98% line coverage and 80.60%
  region coverage.
- Remote CI: GitHub Actions `ci #44`, run ID `28993856797`, passed all required
  jobs for commit `0fdc23dea157ee55226eae24d8d4d817c46b5d59`.
- Clean-checkout local gate: commit `0fdc23dea157ee55226eae24d8d4d817c46b5d59`
  passed `FORGE_T2_RUN_FUZZ=1 scripts/gates/gate_T2.sh` from
  `/private/tmp/forge-t2-clean-gate/forge-t2-0fdc23d-clean`.
- Reviewer hardening: 8 reviewer-only T2 oracles passed, 3 mutation checks
  caught their injected failures, and 3 spot replays round-tripped with
  deterministic replay checks.

## 4. KNOWN ROUGH EDGES

T2 proves the rules machinery and representative integration pressure. It is
not yet the full 100k-card product. T3 is where the card DSL, legacy parser,
translation factory, smoke harness, and coverage dashboard turn this engine
into broad card coverage.

## 5. WHAT YOU SHOULD EXPECT NEXT

We stop at the T3 boundary per your instruction. The next work, when you say
go, is T3.1: freeze-review the DSL design before mass translation.

## 6. WHAT WE NEED FROM YOU

Nothing right now. T2 is being recorded as passed, and work pauses before T3.
