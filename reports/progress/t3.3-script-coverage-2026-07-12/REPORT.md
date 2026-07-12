# Forge 2.0 T3.3 Script Coverage Progress

Generated from committed local `reports/gates/T3.3` evidence and local git history only. No live metrics, network access, installs, GitHub Actions, or pushes were used.

## Snapshot

- Latest committed checkpoint: **13,368/33,290 complete scripts (40.16%)**.
- Mapped ability uses: **22,164/43,649 (50.78%)**.
- Owner-priority scripts complete: **200/365**.
- Across this evidence series: **+4,366 complete scripts** and **+6,144 mapped uses** over 41 monotonic committed checkpoints.
- T3.3 60% floor remaining: **6,606 scripts**.
- Mechanical 100% remaining: **19,922 scripts**.

## Projection

The recent campaign median is **836 complete scripts/hour** across the latest campaign intervals; the whole-series wall-clock rate is **159/hour**. Linear projection gives:

| Target | Recent campaign rate | Whole-history wall-clock rate |
|---|---:|---:|
| 60% T3.3 floor | 7.9 h | 41.6 h |
| 100% mechanical coverage | 23.8 h | 125.6 h |

These are trend scenarios, not delivery promises. The recent rate assumes compatible high-yield families remain available and work continues in dense batches. The historical rate includes inactive gaps and small experimental batches. The 100% estimate is especially optimistic because the tail shifts toward linked abilities, open selectors, unsupported values, replacement effects, and rules requiring new semantic design; actual time can be several times the linear estimate, and some items may appropriately remain quarantined until later runtime work.

## How Coverage Is Being Built

1. **Structural typed lowering.** Legacy Forge script forms are parsed into explicit typed CardDef operations, selectors, conditions, costs, events, and values. Unsupported or ambiguous forms remain quarantined instead of being guessed.
2. **Blocker histograms.** Development sweeps regenerate exact blocker families and counts. This exposes concentration in unsupported parameters, values, and unmapped APIs.
3. **Singleton and high-yield ranking.** Candidate families are ranked by scripts that would become fully translatable if that family were closed, not merely by raw occurrence count.
4. **Fanout and priority weighting.** Linked sub-abilities are weighted by how many parent triggers depend on them, while the Owner priority list raises strategically important cards.
5. **Compatible-family batching.** Several families sharing a representation or parser path are implemented together before validation, reducing repetitive compile and sweep overhead.
6. **24-worker shared-cache local sweeps.** Development and exact checkpoint sweeps use up to 24 local workers and one Cargo cache. This avoids duplicate caches and GitHub Actions consumption.
7. **Exact deterministic checkpoints.** Source is committed before the checkpoint. Workspace tests, clippy with warnings denied, compiler round trips, database validation, oracle/nightmare/smoke suites, coverage floors, parallel replay, and blocker-plan replay bind evidence to an exact product commit and tree.

## Reading The Evidence

- `coverage_over_time.png` shows complete-script and mapped-use percentages at committed checkpoints.
- `checkpoint_throughput.png` shows the gain associated with each evidence checkpoint; the sequence maps to `checkpoint_series.csv`.
- `coverage_projection.png` compares recent-campaign and whole-history linear scenarios.
- `checkpoint_series.csv` is the machine-readable series, including evidence and mapper commits.

## Important Boundary

This is **structural mapping evidence**, not proof that every translated card behaves correctly in a complete game runtime. T3.5/T3.6 semantic, differential, oracle, and card-specific verification remain necessary. A rising mapped-use percentage means fewer scripts are rejected by the typed importer; it does not by itself establish visual correctness, gameplay completeness, or zero bugs.
