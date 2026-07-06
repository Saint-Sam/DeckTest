# Fresh Clone Gate Evidence - T0

Date: 2026-07-06

Initial commit under test: `93cad32`
(`T0.0: initialize Forge 2.0 foundation`)

Re-review commit under test:
`3ee6166a6a969207088c46beda5e7c29a914ad48`
(`T0.3: record remote CI remediation evidence`)

Command shape:

```bash
set -euo pipefail
git clone --recurse-submodules "/Users/juanlopez2016/Desktop/Forge 2.0" "$clone_dir"
cd "$clone_dir"
bash scripts/gates/gate_T0.sh
```

Result: PASS.

Evidence:

- Recursive clone completed after network approval.
- `vendor/legacy-forge` checked out pinned commit
  `1f0a3e0815822d8f58f798e0304b33d4534248b1`.
- `scripts/gates/gate_T0.sh` ended with `PASS gate_T0.sh`.

Note: an earlier sandboxed attempt could not resolve `github.com`; the strict
rerun with approved network access completed successfully.

## Re-review Clean Clone

Date: 2026-07-06

Clone directory:
`/private/tmp/forge_t0_fresh_3ee6166_20260706_1918`

Result: PASS.

Evidence:

- Recursive clone completed.
- `git rev-parse HEAD` returned
  `3ee6166a6a969207088c46beda5e7c29a914ad48`.
- `vendor/legacy-forge` checked out pinned commit
  `1f0a3e0815822d8f58f798e0304b33d4534248b1`.
- `bash scripts/gates/gate_T0.sh` ended with `ALL CHECKS PASSED` and
  `PASS gate_T0.sh`.
- The clean clone was dirty after the gate run only because the gate generated
  local evidence outputs (`docs/toolchain.lock.md` and `metrics/coverage.json`).
