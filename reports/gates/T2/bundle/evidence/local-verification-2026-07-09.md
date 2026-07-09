# T2 Local Verification

Date: 2026-07-09

Scope: local T2 exit-gate tree after T2.10 and the generated T2 oracle pack.

## Commands

```bash
python3 tools/generate_t2_gate_oracle_pack.py
cargo run -p forge-testkit -- lint tests/oracle/generated_t2_gate_622
cargo run -p forge-testkit -- oracle --path tests/oracle/generated_t2_gate_622 --no-junit
scripts/run_oracle.sh --all
FORGE_T2_RUN_FUZZ=1 scripts/gates/gate_T2.sh
```

## Results

- Generator wrote 622 scenarios; total checked oracle corpus is 1,200 `.ron`
  files.
- Generated T2 gate pack lint: PASS.
- Generated T2 gate pack oracle run: 622 passed, 0 failed.
- Full oracle run: 1,200 passed, 0 failed.
- `scripts/vl.sh`, as invoked by the gate, passed with 1,200 oracle scenarios,
  nightmare-suite smoke, coverage, clone-surface, and perf checks.
- Nightmare suite: 1,000 game(s), 10 fixture(s), 0 invariant violations.
- T2 fuzz gate: 3 targets, 14,400 seconds each, all clean.
- Overall T2 gate: `PASS gate_T2.sh`.

