# T2 Clean-Checkout Gate Evidence

Date: 2026-07-09

Reviewed commit: `0fdc23dea157ee55226eae24d8d4d817c46b5d59`
(`T2 gate: add exit evidence packet`)

Clean checkout:
`/private/tmp/forge-t2-clean-gate/forge-t2-0fdc23d-clean`

Command:

```bash
FORGE_T2_RUN_FUZZ=1 scripts/gates/gate_T2.sh
```

Result: PASS. Full log:
`reports/gates/T2/clean-checkout-gate-0fdc23d-2026-07-09.log`

## Key Results

- Oracle gate: 1,200 scenarios passed, 0 failed.
- Nightmare suite: 1,000 game(s), 10 fixture(s), 0 invariant violations.
- `fuzz_apply`: 4,790,289 runs in 14,401 seconds.
- `fuzz_characteristics`: 3,203,178 runs in 14,401 seconds.
- `fuzz_scenarioparse`: 796,343,177 runs in 14,401 seconds.
- Overall gate result: `PASS gate_T2.sh`.

## Local-Only CI Budget Note

This clean-checkout gate was run locally to avoid spending additional GitHub
Actions minutes. GitHub Actions `ci #44`, run ID `28993856797`, already passed
for the reviewed commit before the budget constraint was raised. Later
T2-closeout evidence/status commits are intentionally local-only unless the
owner explicitly approves a push.
