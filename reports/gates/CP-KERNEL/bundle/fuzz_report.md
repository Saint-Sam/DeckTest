# Fuzz Report

Checkpoint: CP-KERNEL

Status: NOT YET APPLICABLE for full T1 fuzzing.

Rationale: `fuzz_apply` and `fuzz_scenarioparse` are planned in T1.12. This
checkpoint occurs immediately after T1.7 to audit the kernel API surface before
T1.8+ builds on it. The reviewer should treat the absence of a live fuzz corpus
as expected schedule state, not as evidence of kernel correctness.

Current risk control:

- `scripts/vl.sh` passes in `test_log.txt`.
- GitHub Actions `determinism-replay` passed for commit `6491d5f`.
- `reports/gates/CP-KERNEL/kernel-invariant-map-2026-07-06.md` calls out the
  API-surface and characteristic-storage risks for adversarial review.
