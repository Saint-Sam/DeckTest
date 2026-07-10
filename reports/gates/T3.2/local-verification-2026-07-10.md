# T3.2 Local Verification

Date: 2026-07-10

Implementation commit: `3b4bbd38ff6b79f6e84b6419eedfc570f77f05dd`

Mode: local-only, forced Cargo offline, no GitHub Actions, network, download,
installation, or push.

## Result

`scripts/gates/gate_T3_2.sh` passed.

- Pinned legacy revision: `1f0a3e0815822d8f58f798e0304b33d4534248b1`.
- Scripts discovered: 33,290.
- Scripts parsed: 33,290.
- Failed scripts: 0.
- Parse-only coverage: 100.0000%, above the 99.5% floor.
- Positioned AST lines: 296,879.
- Ability lines: 18,268 activated, 16,708 triggered, 1,673 replacement,
  and 7,000 static.
- Keyword lines: 18,042.
- SVar lines: 58,592.
- Multi-face boundaries: 857.
- Lossy UTF-8 files: 0.

The gate also passed formatter, clippy, parser tests, deterministic full-corpus
metric replay, workspace coverage at 18,143/22,586 lines (80.3285%), three
deterministic card-database builds, catalog validation, compiled nightmare
integration, and all 1,200 semantic Oracle scenarios.

The signed CP-DSL mutation, sanitizer, and cross-platform evidence remains
bound to its exact checkpoint commit. Recurring T3 integration does not
recreate that expensive checkpoint packet on every parser or mapper change.
