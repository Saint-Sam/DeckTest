# ADR-0002: Legacy Forge Vendor Pin

Date: 2026-07-06

## Status

Accepted for T0.5.

## Context

The master plan requires the Card-Forge/forge repository to be vendored
read-only as the behavioral and data source for card scripts, tests, AI profile
inspiration, and legacy differentials. The vendored source must be pinned to a
specific commit so later mining and differential behavior are reproducible.

## Decision

Vendor `https://github.com/Card-Forge/forge` as a git submodule at
`vendor/legacy-forge`.

Pinned commit:

```text
1f0a3e0815822d8f58f798e0304b33d4534248b1
```

The legacy repository is mined and referenced as data/behavior. New Forge 2.0
code must not be a line-by-line port of legacy Java code.

## Consequences

T0.6 and later differential tasks run against a stable legacy corpus. Updating
the legacy pin requires a superseding ADR or plan-approved maintenance task, not
an incidental submodule bump.

The GPL-3.0-only posture from ADR-0001 applies to derived outputs.

## Alternatives Considered

Floating `main` was rejected because it would make mining counts, tests, and
differentials change without a corresponding project decision.

