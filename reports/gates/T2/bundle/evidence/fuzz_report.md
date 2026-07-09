# T2 Gate Fuzz Report

Date: 2026-07-09

Command:

```bash
FORGE_T2_RUN_FUZZ=1 scripts/gates/gate_T2.sh
```

Result: PASS. Full log is archived in `reports/gates/T2/test_log.txt`.

The T2 gate ran the sanitizer-backed fuzz suite in `--t2-gate` mode with
address sanitizer enabled, nightly Rust, and `14400` seconds per target.

| Target | Result | Runs | Duration |
| --- | --- | ---: | ---: |
| `fuzz_apply` | PASS | 5,469,499 | 14,401 s |
| `fuzz_characteristics` | PASS | 1,770,840 | 14,401 s |
| `fuzz_scenarioparse` | PASS | 1,559,087,053 | 14,401 s |

The `fuzz_characteristics` run emitted a macOS external-symbolizer warning while
discovering new coverage. There was no sanitizer summary, crash artifact, panic,
or non-zero exit; the target continued and ended with `DONE`.

