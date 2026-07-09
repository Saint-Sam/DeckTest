# PC-0002: Card identity and coverage contract

Date: 2026-07-09

Status: Accepted and incorporated in Master Plan v1.3.

## Motivation

The original plan measures only 33,290 legacy Forge scripts, while the Owner
wants all roughly 100,000 cards represented. The local Scryfall snapshot has
113,234 English printing records, 38,225 unique English Oracle identities, and
37,790 unique English names. A printing and a rules identity are different
things and must not be counted as though each needs duplicate mechanics code.

## Exact Plan Change

1. Add a versioned `PrintingRecord` catalog keyed by Scryfall printing id.
2. Add one mechanics `CardDefinition` per Oracle identity; every printing
   references its Oracle identity.
3. Import and index 100% of English printing records in the pinned metadata
   snapshot. Non-game objects and out-of-v1 mechanics remain visible and carry
   an explicit status instead of disappearing.
4. Report separate generated metrics:
   - catalog coverage: imported English printings / source English printings;
   - identity coverage: classified Oracle identities / source identities;
   - playable coverage: verified playable identities / in-scope identities;
   - legacy parity: verified playable legacy scripts / legacy scripts.
5. Preserve the existing T3 milestone of at least 60% verified legacy-script
   translation and GA target of at least 95% playable legacy scripts, while
   requiring 100% catalog coverage and 100% identity classification.
6. Catalog silver-border, joke, token, emblem, art-series, and other special
   records even when their mechanics are out of v1 scope; label the reason.
7. Expand CP-DSL from 20 hand translations to 100 stratified examples selected
   across layouts, ability APIs, selectors, costs, zones, triggers,
   replacements, continuous effects, multiplayer choices, linked abilities,
   and unusual mana symbols.
8. An Oracle identity is the stable Scryfall `oracle_id` when present. Records
   without one use a deterministic namespaced identity derived from layout and
   source id. Multi-face, split, adventure, transform, modal DFC, flip, meld,
   and reversible cards keep ordered face records under one identity when the
   source says they are one game object. Tokens, emblems, art-series, and other
   non-game records remain printing/catalog records with an explicit
   non-playable classification.
9. Every generated coverage metric records source path, retrieval/source
   timestamp, SHA-256, and generator version. No coverage value is hand typed.
10. Catalog import remains metadata/text only and may not introduce official
    art, set symbols, or mana-symbol fonts. GPL, Fan Content, and Scryfall
    attribution requirements remain unchanged.

## Affected Tasks

T3.1-T3.8, CP-DSL, CP-PORT-20, T5.6-T5.7, T6.1, T7 packaging, card coverage
metrics, and the v1 acceptance test.

## Risks And Mitigations

- Risk: printing count falsely inflates rules coverage.
  Mitigation: printing, identity, playability, and legacy parity are separate.
- Risk: the long tail blocks the entire application.
  Mitigation: every identity is classified and visible; quarantine is explicit
  and cannot count as playable.
- Risk: current bulk metadata changes over time.
  Mitigation: source timestamp and SHA-256 are pinned in generated provenance.

## Approval

The Owner requested all roughly 100,000 cards be represented and approved this
review's identity/coverage recommendation in the Codex thread on 2026-07-09.

Gate Reviewer recommendation: RECOMMENDED on 2026-07-09 after requiring precise
identity, provenance, IP, and non-playable classification rules. The final text
incorporates those requirements.
