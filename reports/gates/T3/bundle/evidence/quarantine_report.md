# Tier 3 Quarantine Report

Date: 2026-07-14

The exact product emitted 20,082 of 33,290 complete legacy scripts
(60.3244%) and retained 13,208 scripts in fail-closed quarantine. The emitted
corpus fingerprint is `efb1d6c4d33d4d33fb43e1ec3282b061`. Owner-priority
coverage is 281 of 365 identities (76.9863%).

The largest current file-level quarantine classes are `UNSUPPORTED_VALUE`
(4,590), `UNSUPPORTED_PARAMETER` (2,371), `COMPILE_ERROR` (1,797),
`UNSUPPORTED_VALUE_SVAR` (1,241), `UNMAPPED_API` (1,028),
`MISSING_CATALOG_IDENTITY` (877), and `UNSUPPORTED_KEYWORD` (799).

These rows are not counted as translated, runtime-smoke passed, semantic
verified, or pod integrated. Tier 3 intentionally closes at the documented
60% complete-script floor; later mapper work remains targeted to vertical
product blockers instead of treating the fail-closed long tail as supported.
