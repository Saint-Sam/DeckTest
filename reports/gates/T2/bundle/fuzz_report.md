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

## Clean-Checkout Repeat

The gate was repeated from a clean local clone at commit
`0fdc23dea157ee55226eae24d8d4d817c46b5d59`.

Evidence:
`reports/gates/T2/clean-checkout-gate-0fdc23d-2026-07-09.log`

| Target | Result | Runs | Duration |
| --- | --- | ---: | ---: |
| `fuzz_apply` | PASS | 4,790,289 | 14,401 s |
| `fuzz_characteristics` | PASS | 3,203,178 | 14,401 s |
| `fuzz_scenarioparse` | PASS | 796,343,177 | 14,401 s |

The clean-checkout repeat ended with `PASS gate_T2.sh`.
