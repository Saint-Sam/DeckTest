# ADR-0011: Card DSL parser and database encoding

Date: 2026-07-09

## Status

Proposed under T3.1 and the dependency budget in Master Plan §5.3. Requires
independent Review Agent approval with the T3.1 packet.

## Context

The line-oriented T3 spike cannot distinguish comments from `//` inside quoted
split-card names, cannot parse nested compositional rules reliably, and writes a
custom text record while the plan requires a versioned binary database.

## Decision

Use `pest`/`pest_derive` for the source grammar, `serde` for the validated card
IR, `bincode` v2's serde adapter for the versioned binary payload, and
`serde_json` for the human-inspectable index and local Scryfall catalog input.

The database has an explicit Forge magic header and schema version before the
bincode payload. Compiler output is sorted by stable identity and contains no
absolute source paths, making clean builds byte-deterministic.

## Consequences

Quoted strings and recursive expressions are parsed by a real grammar with
source spans. Unknown fields, operations, keywords, selectors, and symbols fail
validation. Runtime loading can reject incompatible/corrupt databases before
decoding their payload.

These crates are already in the master plan's approved dependency budget, but
Cargo must download `pest`, `pest_derive`, and `bincode` once on this machine.

## Alternatives Considered

- Continue the hand-written line parser: rejected because lexical context,
  nested expressions, and diagnostics would become a second parser project.
- Store JSON as the runtime database: rejected because size/load-time and schema
  ambiguity are worse than a versioned binary plus generated JSON index.
- Couple card definitions directly to `forge-core`: rejected because runtime
  object ids and game state do not belong in reusable printed-card mechanics.
