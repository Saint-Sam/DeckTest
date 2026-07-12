# Forge 2.0 T3.3 Script Coverage Progress

Generated from committed local `reports/gates/T3.3` evidence and local git history only. No live metrics, network access, installs, GitHub Actions, or pushes were used.

## Snapshot

- Latest committed checkpoint: **14,179/33,290 complete scripts (42.59%)**.
- Mapped ability uses: **22,322/43,649 (51.14%)**.
- Owner-priority scripts complete: **200/365**.
- Across this evidence series: **+5,177 complete scripts** and **+6,302 mapped uses** over 44 committed checkpoints, including **1 reviewed downward correction(s)**.
- T3.3 60% floor remaining: **5,795 scripts**.
- Mechanical 100% remaining: **19,111 scripts**.

## Projection

The recent six-checkpoint net cadence is **842 complete scripts/hour**; the whole-series wall-clock rate is **181/hour**. These rates use elapsed time between evidence timestamps, not measured hands-on work time. Linear projection gives:

| Target | Faster observed checkpoint cadence | Whole-history wall-clock rate |
|---|---:|---:|
| 60% T3.3 floor | 6.9 h | 32.0 h |
| 100% mechanical coverage | 22.7 h | 105.5 h |

These are trend scenarios, not delivery promises. Neither rate measures active engineering time. The faster rate assumes compatible high-yield families remain available and work continues in dense batches. The historical rate includes inactive gaps and small experimental batches. The 100% estimate is especially optimistic because the tail shifts toward linked abilities, open selectors, unsupported values, replacement effects, and rules requiring new semantic design; actual time can be several times the linear estimate, and some items may appropriately remain quarantined until later runtime work.

## Independent Review Corrections

Ampere's review found two P1 fail-open paths and one P2 test gap in an earlier mapper batch. The source-zone issue now lowers closed source moves through `MoveZoneFrom(source, origin, destination)`, retaining the legacy no-op guard when the source is not in the declared origin. `RestrictValid` now checks each complete branch against an exact closed vocabulary, rejecting approved-prefix extensions such as `Spell.Runtime.Arbitrary`. Focused regressions now cover dynamic restricted-mana amounts, both dynamic Dig counts, and subtype removal across Continuous, Animate, and AnimateAll. These findings temporarily invalidated the older false-positive checkpoint; the current exact checkpoint was regenerated after remediation.

Downward corrections are retained in the CSV and plots. Projections use net changes rather than deleting regressions or ignoring nonpositive intervals.

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
- `coverage_projection.png` compares faster observed-checkpoint and whole-history linear scenarios.
- `checkpoint_series.csv` is the machine-readable series, including evidence and mapper commits.

## Important Boundary

This is **structural mapping evidence**, not proof that every translated card behaves correctly in a complete game runtime. T3.5/T3.6 semantic, differential, oracle, and card-specific verification remain necessary. A rising mapped-use percentage means fewer scripts are rejected by the typed importer; it does not by itself establish visual correctness, gameplay completeness, or zero bugs.
