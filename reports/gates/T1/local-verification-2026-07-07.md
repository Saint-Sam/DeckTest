# T1 Local Verification

Date: 2026-07-07

Scope: local gate-prep working tree after the T1 oracle expansion, combat
oracle remediation, clone surface remediation, small legal-action list
optimization, and the final T1 perf hot-path fixes.

## Commands

- `cargo test -p forge-core`: PASS, 72 tests.
- `cargo test -p forge-testkit`: PASS, 10 library tests and 8 CLI tests.
- `scripts/run_oracle.sh --all`: PASS, 300 scenarios including 60 combat
  scenarios.
- `scripts/perf_smoke.sh`: PASS after the T1 perf hot-path fixes.
- `scripts/gates/gate_T1.sh > reports/gates/T1/test_log.txt 2>&1`: PASS.

## Gate Output Highlights

- `scripts/gates/gate_T1.sh`: PASS combat oracle surface, 60 scenarios cover
  T1.6 combat feature requirements.
- `scripts/vl.sh`: PASS inside the T1 gate log.
- `scripts/run_oracle.sh --all`: 300 scenarios passed, 0 failed.
- `scripts/perf_smoke.sh`: PASS, 4 metrics compared at 5.0% threshold.
- `cargo run -p forge-arena -- --smoke 10000 --random`: PASS, 10,000 games,
  0 invariant violations.
- `forge-cli play --demo --seed 11`: PASS, final hash
  `17913199206715572167`, outcome `won player 0`.
- `forge-cli roundtrip target/gates/T1/replays/demo-seed-11.frsreplay`: PASS,
  final hash preserved.
- Clone budget: PASS at 112.292 ns per 200-card state.

## Combat Oracle Remediation Notes

The T1 Gate Reviewer failed G5/G8 because the earlier 300-scenario bundle did
not prove the T1.6 combat oracle surface. The remediation added combat actions
to the dependency-free `forge-testkit` RON runner and regenerated the bounded
300-scenario pack so 60 scenarios now exercise:

- attack/block legality, flying/reach, menace, and vigilance
- first-strike and double-strike combat damage steps
- double-block damage assignment order
- trample plus deathtouch assignment
- lifelink, including the row S8 lifelink-before-loss-SBA case

The gate script now checks this combat oracle surface before running the rest
of the T1 gate.

## Perf Remediation Notes

The first T1 gate capture failed on `kernel_full_playout_four_turns` after the
shallow clone change made mutation-heavy cloned playouts pay avoidable work.
The fix was code-side, not a threshold or baseline relaxation:

- Creature state-based actions now iterate the battlefield zone directly rather
  than scanning all object records and searching zones for each record.
- `pass_priority` avoids an extra player lookup on the hot priority-rotation
  path.
- The public `legal_actions`/`apply` hot boundary, private `pass_priority`, and
  small `ActionList` helpers are marked inline.

The final T1 gate log ends with `PASS gate_T1.sh`.
