# Tier 3 Scenario Parser Fuzz Regression

## Discovery

The pre-final local AddressSanitizer campaign against product commit
`bfaea59c53fbb8988b71f150bc6a42ef2996540f` found an adversarially nested RON
value that exhausted the native stack in `RonParser::parse_value`.

- Target: `fuzz_scenarioparse`
- Failure: AddressSanitizer stack overflow
- Original input bytes: 11,491
- Preserved input: `reports/gates/T3/fuzz-regressions/scenario-parser-stack-overflow-2026-07-14.input`
- Input SHA-256: `e54c6c6752a71037033b56af0ac254888b7c028a95a905c16a32b42be5273d37`

## Remediation

Commit `4c18592` added a fail-closed maximum RON nesting depth of 128, a focused
boundary and adversarial-depth regression test, and a mutation that changes the
guard from `>=` to `>`. Commit `7bbbafa` made the regression test satisfy the
workspace's strict Clippy gate.

The preserved crashing input was then executed once against the repaired
AddressSanitizer target and returned normally. The final exact Tier 3 campaign
against `7bbbafa376a5222c3a335a744b5b942898c67a84` completed 3,608 verified
worker-seconds across eight workers with no artifacts. The scored mutation gate
killed all five declared mutants, including the nesting-guard mutant.
