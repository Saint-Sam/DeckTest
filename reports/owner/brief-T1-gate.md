# OWNER BRIEF - T1 Rules Kernel V1 Gate

Date: 2026-07-07

## 1. WHAT JUST HAPPENED

Tier 1 now has a working deterministic rules-kernel slice with setup, opening
hands, priority, stack, mana, combat, state-based actions, a CLI demo/replay
path, fuzz targets, performance benches, and 300 oracle scenarios. The T1 gate
reviewer found that the first 300-scenario packet did not prove the required
combat oracle surface, so I added combat actions to the scenario runner and
regenerated the bounded oracle pack with 60 combat scenarios. The final local
T1 gate passed after real code-side fixes; I did not relax the perf threshold
or recalibrate the baseline. Remote CI was green at `c9e333e` before this final
local gate packet, so GitHub Actions still needs to verify the exact commit
after push.

## 2. WHAT YOU SHOULD SEE - TRY IT YOURSELF

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && scripts/gates/gate_T1.sh`
- EXPECT: the command includes `PASS combat oracle surface: 60 scenario(s) cover
  T1.6 combat feature requirements`, ends with
  `PASS clone budget: 112.292 ns per 200-card state`,
  and `PASS gate_T1.sh`.
- RED FLAG: any `ERROR`, failed oracle, perf regression, replay mismatch, or
  arena invariant violation; reply with the failing tail and I will investigate
  within 24 hours.

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && scripts/run_oracle.sh --all`
- EXPECT: `oracle scenarios: 300 passed, 0 failed`.
- RED FLAG: fewer than 300 scenarios or any failed `.ron` file.

- DO: open `reports/gates/T1/replays/README.md`
- EXPECT: 10 seeded demo replays, seeds `11` through `20`, each with a final
  hash and `won player 0` outcome.
- RED FLAG: missing replay files or a `forge-cli roundtrip` mismatch.

## 3. NUMBERS THAT MATTER

- 300 oracle scenarios pass: these are the ground-truth T1 rules exam set.
- 60 combat oracle scenarios cover T1.6, including double-block ordering,
  trample plus deathtouch, first/double strike, flying/reach, menace,
  vigilance, and lifelink before loss SBAs.
- 10,000 random arena smoke games pass with 0 invariant violations.
- Clone budget is 112.292 ns per 200-card state, below the 200 ns target.
- Coverage is 83.73% lines and 82.53% regions.
- Fuzzing completed 6,530,854 `fuzz_apply` runs and 895,666,102
  `fuzz_scenarioparse` runs in the T1 six-hour aggregate gate.

## 4. KNOWN ROUGH EDGES

This is still a T1 kernel, not a complete card engine. Layers, real card IR,
replacement/prevention effects, full UI flows, and production-grade AI/search
come later. The current replay demo is intentionally tiny and deterministic.
Remote CI and exact-tree gate review must still run against the final local
remediation commit after you push it.

## 5. WHAT YOU SHOULD EXPECT NEXT

A Gate Reviewer agent needs to re-review the final T1 bundle against
T1.R5-T1.R9. After local review and commit, GitHub Actions should run on the
pushed commit; a green run plus signoff will let us mark T1 passed and open
Tier 2.

## 6. WHAT WE NEED FROM YOU

After I create the local gate packet commit, please push it through GitHub
Desktop so remote CI can validate the exact T1 gate state.
