# PC-0004: Honest CP-DSL classification

Date: 2026-07-10

Status: Accepted and incorporated in Master Plan v1.5.

## Motivation

The CP-DSL packet proves language structure, closed recursive typing,
round-tripping, deterministic compilation, and broad engine behavior. It does
not prove that every one of its 100 card recipes exactly implements retained
Oracle text. Labeling those recipes `verified_playable` would therefore make a
false semantic claim and weaken later translation metrics.

## Exact Plan Change

1. Classify all 100 CP-DSL review definitions as `unverified_playable`.
2. Define the packet as a reviewer-authored language-stress corpus across the
   exact 25 mandatory strata, not a card-fidelity acceptance sample.
3. Reserve `verified_playable` for definitions with card-specific semantic
   evidence. Parsing, round-tripping, compilation, and generic engine oracles
   are necessary but insufficient for promotion.
4. Keep O4 as a one-way freeze of identity, grammar, closed argument typing,
   canonical emission, and database contracts only.
5. Require T3.6 semantic tests and CP-PORT-20 fidelity review to promote card
   definitions. Systematic mismatches reopen the mapper or DSL as already
   required by the plan.
6. Correct obvious language gaps found during CP-DSL review before O4. The
   initial remediation adds exact modal-count representation separately from
   "up to" modal selection and prevents cross-face keyword leakage.
7. Generated CP-DSL metrics must prove that all 100 review definitions remain
   honestly classified and that zero are counted as semantically verified.

## Affected Tasks

T3.1, CP-DSL, T3.2-T3.6, CP-PORT-20, playable-coverage metrics, and O4 wording.

## Risks And Mitigations

- Risk: unverified recipes could accidentally ship as playable.
  Mitigation: classification is compiled into the database and promotion is
  checked by later semantic gates.
- Risk: a language gap is discovered after O4.
  Mitigation: T3.6 and CP-PORT-20 systematic failures reopen the DSL or mapper;
  O4 does not suppress evidence-driven remediation.

## Approval

The independent CP-DSL Gate Reviewer rejected unconditional
`verified_playable` status. In the Codex thread on 2026-07-10, the Owner chose
Option 2: honest staged verification with card fidelity deferred to T3.6 and
CP-PORT-20.
