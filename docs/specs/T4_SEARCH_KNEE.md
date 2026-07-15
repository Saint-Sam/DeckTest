# T4 Search Knee And Adaptive Stopping

Status: Owner-approved provisional thresholds, 2026-07-14. A later threshold
change requires measured evidence and an approved plan change.

Search cost may scale roughly with budget; playing strength is expected to show
diminishing returns. Every `B` versus `2B` comparison uses identical benchmark
states, decks, seats, hidden-information determinizations, seeds, legal-action
sets, hardware, and thread configuration.

A provisional knee requires two consecutive budget increases where all of the
following hold:

1. Paired win-rate gain is below one percentage point, or estimated gain is
   below 10-15 Elo.
2. The confidence interval includes no product-useful improvement.
3. Catastrophic-blunder rate does not materially fall.
4. Missed deterministic wins or required defenses do not materially fall.
5. At least 95% of ordinary states select the same acceptable action.
6. p95 latency and measured report cost continue to rise materially.

The Standard tier uses the smallest budget that clears its competence bar
before the plateau. The largest tested budget has no privileged status.

At fixed visit checkpoints, search records the leader, visit share, value and
visit gaps, ranking stability, policy uncertainty, bounded-solver state, and
stop reason. The complete checkpoint sequence is retained in each genuine
searched-decision record rather than reducing the run to its final sample.
Singleton and forced actions, routine priority passes, forced
trigger handling, and obvious mana production receive no full MCTS budget.

Every adaptive-stop rule is ablated against the paired fixed-budget baseline.
It may ship only when CPU cost falls without a statistically or practically
meaningful decline across Tracks A, B, and C.

The illustrative cost table supplied with the clarification is retained in
`metrics/ai/search_budget_knee.json`, clearly marked non-authoritative. It must
be replaced by measured searched decisions per pod, CPU milliseconds per
decision, worker price, replacement overhead, and utilization.

## Local Campaign

`forge-arena --search-knee` runs every adjacent exact `B`/`2B` pair with the
same compiled pod, physical seats, paired seeds, seat-tied policy seeds,
determinizations, legal-action generator, hardware, and thread configuration.
It also runs fixed/adaptive seat-rotation ablations unless explicitly skipped
for a non-gate diagnostic. The report preserves per-budget wall-latency samples
and calculates campaign p50/p95/p99 values without pooling `B` and `2B`.

The arena outcome track cannot by itself prove identical legal-action sets
after policies diverge. Exact legal-set and acceptable-action agreement must
come from the immutable decision-state benchmark. Catastrophic-blunder,
missed-win/required-defense, acceptable-action, CPU-cost, and material-cost
criteria remain null until their authoritative adapters or labels exist. The
harness therefore cannot manufacture a knee or update `ai_tiers.ron` from an
incomplete campaign.

The material latency/cost rise criterion intentionally has no invented numeric
threshold. A measured threshold requires Owner-approved plan change before it
can become promotion-authoritative.

The product-bound local 1/2/4 ms, two-game smoke completed both B/2B
comparisons and all three fixed/adaptive ablations. The first artifact exposed
per-tree deadline multiplication. Product 316d9fd corrected that contract and
edge accounting, then measured approximately 240-266 ms p95 because eager
high-branching adapter work and duplicate full-state construction remained
non-preemptible. Product 3b1fd6d replaced those paths with hierarchical combat
subcontexts and one determinized state per sample; its exact rerun measures
approximately 1.5-4.7 ms p95 across the same nominal budgets. That is a bounded
timing diagnostic, not a knee. Safe Linux/Android CPU and memory adapters are
implemented in the following source wave, but a measured supported-platform
campaign, Track B labels, reference devices, and useful confidence intervals
still prohibit a strength, cost, or plateau claim.

The supporting design reference is *Learning to Stop: Dynamic Simulation
Monte-Carlo Tree Search* (arXiv:2012.07910). DeckTest's own paired arena,
decision benchmark, human trace review, latency, and cost measurements remain
the promotion authority.
