# T1 Combat/Lifelink Oracle Schema Gap

Date: 2026-07-07

Status: closed

Priority: P0

Source:
- `reports/gates/T1/SIGNOFF.md` G5, G8, and T1.R6
- `docs/t1_10_legacy_test_oracle_mapping.md` row S8
- `FORGE_REBUILD_MASTER_PLAN.md` Section 6 T1.6

Blocker:
The T1 scenario/oracle surface does not yet expose the combat setup and combat
actions needed to express the legacy combat plus lifelink case, and the T1.6
gate requirement for at least 60 combat oracle scenarios is not yet proven by
oracle files.

Impact:
T1 gate signoff remains blocked. Existing Rust combat unit tests are useful
supporting evidence, but they do not close the plan-required combat oracle gap.

Closure:
Closed 2026-07-07 by T1.R5/T1.R6 remediation.

Evidence:
- `crates/forge-testkit/src/lib.rs` exposes combat RON actions:
  `declare_attackers`, `declare_blockers`, and `assign_combat_damage`.
- `tests/oracle/generated_t1_300/t1_14_223_combat_unblocked_vigilance_000.ron`
  through `tests/oracle/generated_t1_300/t1_14_282_combat_lifelink_unblocked_009.ron`
  provide 60 passing combat oracle scenarios.
- `tests/oracle/generated_t1_300/t1_14_273_combat_lifelink_unblocked_000.ron`
  covers the row S8 lifelink-before-loss-SBA case by setting active life to 0
  before lifelink combat damage is assigned and then expecting the game to
  remain in progress.
- `scripts/gates/gate_T1.sh` now fails if the T1.6 combat oracle surface drops
  below 60 scenarios or loses required combat feature coverage.
- `reports/gates/T1/test_log.txt` ends with `PASS gate_T1.sh` and includes
  `PASS combat oracle surface: 60 scenario(s) cover T1.6 combat feature
  requirements`.
