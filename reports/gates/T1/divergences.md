# T1 Divergences

No T1 plan divergences are open.

Owner decision `Q-2026-07-07-T1.10` resolved the legacy-oracle source scope:
use the broader local `forge-gui-desktop` game-simulation tests as the T1.10
source set. The 300-scenario expansion was implemented as additional oracle
coverage and does not de-scope any T1 exit criterion.

The T1.R5 remediation keeps the oracle pack bounded at 300 total scenarios
while reserving 60 of them for T1.6 combat coverage. This is not a de-scope:
the gate now explicitly checks the 60 combat scenarios and required combat
feature surface before it can pass.

The final T1 perf fix did not relax thresholds or recalibrate
`metrics/perf_baseline.json`; it made the kernel hot path faster.
