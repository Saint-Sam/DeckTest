# CP-LAYERS Reviewer Instructions

Date prepared: 2026-07-07

Reviewer role: Owner/human reviewer for CP-LAYERS under O1 Option C.

This checkpoint blocks all T2.5+ work. The reviewer should treat the layer
system as suspect until the evidence below passes.

## Required Review Items

1. Author 15 novel layer-interaction oracle scenarios that were not written by
   the implementer. At least 14 of 15 must pass.
2. Include cases for CR 613.8 dependency ordering, timestamp ties,
   characteristic-defining abilities, Humility-class type/ability/P-T stacking,
   copy effects, control effects, type/color/text/ability layers, and 7a-7d
   interactions.
3. Run a differential comparison versus the legacy engine on a 100-card layered
   subset. Every divergence must be adjudicated in `divergences.md` or a
   reviewer addendum.
4. Inspect the memoization/invalidation story. Current T2.4 has no
   characteristics memoization cache; queries fully recompute. Decide whether
   `fuzz_characteristics` is sufficient or demand a longer sanitizer fuzz run.
5. Write the required explicit signoff sentence in `SIGNOFF.md`:
   "I believe layer ordering is correct for the following reasons..."

## Stop Conditions

- If fewer than 14 of 15 reviewer scenarios pass, CP-LAYERS fails and T2.4
  reopens with P0 remediation.
- If the legacy differential produces unadjudicated divergences, CP-LAYERS
  fails pending written adjudication.
- If three CP-LAYERS attempts fail, Section 16.3 kill-criteria review applies
  to the layer design.
