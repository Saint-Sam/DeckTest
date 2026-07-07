# OWNER BRIEF - T1 Rules Kernel V1 Gate

Date: 2026-07-07

## 1. WHAT JUST HAPPENED

Tier 1 now has a working deterministic rules-kernel slice with setup, opening
hands, priority, stack, mana, combat, state-based actions, a CLI demo/replay
path, fuzz targets, performance benches, and 300 oracle scenarios. The T1 gate
reviewer originally failed the packet because the 300-scenario corpus did not
prove the required combat oracle surface. That gap is now locally remediated:
the scenario runner supports combat actions, the bounded oracle pack includes
60 combat scenarios, the live reviewer checks passed, the clean-checkout gate
passed, and GitHub Actions passed for the exact T1 gate evidence commit
`2198493`.

## 2. WHAT YOU SHOULD SEE - TRY IT YOURSELF

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && scripts/gates/gate_T1.sh`
- EXPECT: the command includes `PASS combat oracle surface: 60 scenario(s) cover
  T1.6 combat feature requirements`, ends with
  `PASS clone budget: 112.292 ns per 200-card state`, and `PASS gate_T1.sh`.
- RED FLAG: any `ERROR`, failed oracle, perf regression, replay mismatch, or
  arena invariant violation; reply with the failing tail and I will investigate
  within 24 hours.

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && scripts/run_oracle.sh --all`
- EXPECT: `oracle scenarios: 300 passed, 0 failed`.
- RED FLAG: fewer than 300 scenarios or any failed `.ron` file.

- DO: open `reports/gates/T1/reviewer-live-checks-2026-07-07.md`
- EXPECT: three mutation checks caught mutants, five reviewer oracle scenarios
  passed, determinism replay passed, live sanitizer fuzz passed, and three spot
  replays matched recorded hashes.

- DO: open `reports/gates/T1/remote-ci-2026-07-07.md`
- EXPECT: GitHub Actions `ci #18`, run ID `28883217715`, passed for commit
  `2198493284299d9721d59ab3a23e3b2a2ab71f56`.

## 3. NUMBERS THAT MATTER

- 300 oracle scenarios pass.
- 60 combat oracle scenarios cover T1.6, including double-block ordering,
  trample plus deathtouch, first/double strike, flying/reach, menace,
  vigilance, and lifelink before loss SBAs.
- 10,000 random arena smoke games pass with 0 invariant violations.
- Clone budget is 112.292 ns per 200-card state, below the 200 ns target.
- Coverage is 83.76% lines and 82.57% regions.
- T1 gate fuzzing completed 6,530,854 `fuzz_apply` runs and 895,666,102
  `fuzz_scenarioparse` runs in the six-hour aggregate gate.
- Live reviewer fuzz completed 1,845,413 `fuzz_apply` runs and 219,668,185
  `fuzz_scenarioparse` runs with address sanitizer enabled and no crashes.

## 4. KNOWN ROUGH EDGES

This is still a T1 kernel, not a complete card engine. Layers, real card IR,
replacement/prevention effects, full UI flows, and production-grade AI/search
come later. The current replay demo is intentionally tiny and deterministic.

## 5. WHAT YOU SHOULD EXPECT NEXT

Tier 1 is pass-recorded locally. The next plan work is Tier 2, with CP-LAYERS
remaining the next human-heavy checkpoint.

## 6. WHAT WE NEED FROM YOU

Push this small pass-record commit through GitHub Desktop so the remote repo has
the same T1 PASS state as the local repo.
