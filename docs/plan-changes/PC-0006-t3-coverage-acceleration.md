# PC-0006: T3 all-blocker batching and local validation acceleration

Date: 2026-07-10

Status: Accepted and incorporated in Master Plan v1.7.

## Motivation

The previous mapper loop exposed one failure at a time and paid for repeated
full-corpus scans. Running more simultaneous translators did not solve that:
measurement showed that two materializing sweeps contend on memory and thousands
of small output files. The Owner requested 2-3x faster card-coverage work,
all-blocker batching, 20-40% less validation time, local hardware use, and no
GitHub Actions, duplicate caches, stale subagents, network access, or installs.

## Exact Plan Change

1. Add a deterministic full-corpus blocker planner. It evaluates every root
   ability, every reachable SVar ability, and every keyword independently.
2. When a node reports an unknown parameter, remove that parameter from a
   temporary expression and re-evaluate until all confirmed unknown parameters
   and the first remaining non-parameter blocker are recorded.
3. Aggregate per-card blocker sets, global card impact, Owner-priority impact,
   linked-root fan-out, and a transparent effort prior (parameter < value <
   keyword < new API). Recommend multi-family batches by effort-normalized
   greedy marginal card completion, rather than fixing the first histogram row
   in isolation.
4. Keep projections explicitly non-authoritative. A later value blocker can be
   revealed after the current non-parameter failure is fixed; only actual
   translator emission and compiler roundtrip count as coverage.
5. Add one local runner with two modes. Development mode overlaps translation,
   planning, mapping audit, and focused tests before compiler validation.
   Checkpoint mode performs a 24-worker materializing sweep, then overlaps a
   12-worker hash-only replay with compiler, 6-worker planner, and mapping audit,
   followed by full workspace verification.
6. Compare deterministic output fingerprints and byte-identical quarantine and
   priority reports. Worker-count metadata may be normalized for comparison;
   semantic or output fields may not.
7. Use one Cargo target/cache and one generated output tree. No hosted CI,
   network, duplicate worktree, or duplicate output tree is part of this loop.
8. Track translated cards per engineering hour for three landed batches. The
   2-3x improvement is a throughput target. Retune batching if the observed gain
   is below 1.5x; do not manufacture the target by weakening gates.

## Measured Evidence

The checkpoint analyzed 33,290 scripts and recorded 63,859 confirmed blocker
observations across 4,877 normalized families, including linked-root fan-out of
64,887. The staged core validation completed in 37 seconds against a measured
56-second comparable serial baseline, saving 33.93%. Full workspace fmt,
clippy, tests, deterministic replay, compiler validation, and 80% coverage still
passed locally. `metrics/blocker_plan.json` and
`metrics/t3_parallel_validation.json` are the authoritative generated records.

## Risks And Mitigations

- Risk: projected completed cards overstate real unlocks.
  Mitigation: label them confirmed-set estimates and count only emitted,
  compiler-roundtripped cards in coverage.
- Risk: parallel full sweeps slow down through I/O contention.
  Mitigation: saturate the primary sweep alone, use a hash-only replay, and
  parallelize the smaller independent checks afterward.
- Risk: faster inner loops weaken correctness.
  Mitigation: development mode is advisory; every integration uses checkpoint
  mode and the unchanged full workspace gates.
- Risk: one large batch obscures regressions.
  Mitigation: keep default batches to five reusable families with one structural
  test pack per family and before/after metrics.

## Approval

In the Codex thread on 2026-07-10, the Owner explicitly approved incorporating
all-blocker batching and a 20-40% validation-time reduction into the wider plan
and instructed Codex to implement it using local hardware.
