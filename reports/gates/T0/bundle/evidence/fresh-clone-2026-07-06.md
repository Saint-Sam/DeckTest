# Fresh Clone Gate Evidence - T0

Date: 2026-07-06

Commit under test: `93cad32` (`T0.0: initialize Forge 2.0 foundation`)

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
