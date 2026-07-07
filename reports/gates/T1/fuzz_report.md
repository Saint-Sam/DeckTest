# T1 Fuzz Report

Date: 2026-07-07

Source report: `reports/fuzz/T1.12-2026-07-07.md`

## Scope

- `fuzz_apply`: byte-driven kernel action sequences through `legal_actions`
  and `apply`, plus setup/life/poison/SBA/view/hash probes.
- `fuzz_scenarioparse`: UTF-8 bounded scenario parser and runner fuzzing for
  the dependency-free RON-compatible testkit schema.

## Gate Result

PASS:

- `fuzz_apply`: 6,530,854 runs in 10,801 seconds.
- `fuzz_scenarioparse`: 895,666,102 runs in 10,801 seconds.

No `ERROR`, `CRASH`, `SUMMARY`, `panic`, `AddressSanitizer`, libFuzzer error,
or crash artifact marker was found in the long gate log.

## Command

```bash
scripts/fuzz_nightly.sh --t1-gate
```
