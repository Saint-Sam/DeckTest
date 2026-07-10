# CP-DSL Independent Gate Review

Date: 2026-07-09

Reviewed commit: `90f5ce69e085fb270bc1b2e8c7cb253009f6547f`

Reviewer: `gpt-5.6-luna`, xhigh reasoning, independent read-only Gate Reviewer

Result: FAIL

## Confirmed Evidence

- The exact clean detached local gate passed without GitHub Actions or network
  use.
- The packet binds 16 command logs and 13 tracked artifacts to the reviewed
  commit and tree.
- Four actual linked target artifacts were present and matched their recorded
  sizes, magic values, and SHA-256 hashes.
- The unmutated control passed and 28/28 curated mutants were killed.
- Five fuzz targets passed 2,408 verified address-sanitizer worker-seconds.
- All 1,200 semantic scenarios passed, including 133 hand-authored scenarios.
- Coverage passed at 17,798/22,161 lines, or 80.31%.

## Blocking Findings

### P1: Semantic status is unsupported

`CardClassification::VerifiedPlayable` means that mechanics have semantic
verification, but `tools/generate_cp_dsl.py` assigns that status to all 100
recipes unconditionally. The packet proves typed compilation and round-trip
structure, not card-by-card semantic equivalence. Sampled discrepancies include
`choose_up_to(2)` for cards whose Oracle text says "Choose two," an incorrect
front-face flying keyword on Delver of Secrets, omitted entry behavior on
Emeria, Shattered Skyclave, and partial Valki/Tibalt effects.

This blocks a one-way language freeze until the owner chooses whether semantic
fidelity belongs in T3.1 or is explicitly deferred with honest classifications.

### P1: Grammar changes did not invalidate evidence

Mutation and fuzz source hashes omitted `card_dsl.pest`, allowing a grammar
change to leave prior evidence apparently current.

Remediation implemented after review: relevant crate source trees, including
the Pest grammar, are now included in both bindings.

### P1: Platform packet trusted metadata

The packet checker retained platform log hashes but did not rehash linked
artifacts or enforce unique exact target identities.

Remediation implemented after review: direct artifact path/hash/size checks,
exact target-set validation, and full CP-DSL validator replay are required.

### P2: Fuzz recheck did not enforce address sanitizer

The supplied logs used address sanitizer, but the checker did not require the
report, worker command, and retained log to agree on `--sanitizer address`.

Remediation implemented after review: address sanitizer, clean exit, timeout
state, final-stat flag, and exact logged command are now validated.

## Next Review

After the owner semantic-scope decision is implemented, regenerate all affected
artifacts in a new clean detached worktree, rerun the exact local packet, and
submit the new commit to an independent Gate Reviewer.

Owner decision on 2026-07-10: Option 2. PC-0004 defines the 100-card packet as
an `unverified_playable` language-stress corpus and requires card-specific
semantic evidence before any later promotion to `verified_playable`.
