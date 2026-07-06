# ADR-0003: Comprehensive Rules Vendor Source

Date: 2026-07-06

## Status

Accepted for T0.5.

## Context

The master plan requires the current Magic: The Gathering Comprehensive Rules
text to be vendored under `docs/vendor/` and used as the first authority for
rules questions.

## Decision

Vendor the official Wizards TXT rules document at:

```text
https://media.wizards.com/2026/downloads/MagicCompRules%2020260619.txt
```

The file is stored at:

```text
docs/vendor/comprehensive-rules.txt
```

The document header says the rules are effective as of June 19, 2026. The URL
was discovered from the official Wizards rules page:

```text
https://magic.wizards.com/en/rules
```

## Consequences

Rules-oracle and implementation tasks cite the vendored text first. If Wizards
publishes a newer rules file, updating this file requires an explicit vendor
update task and ADR addendum or superseding ADR.

## Alternatives Considered

Using agent memory or third-party summaries was rejected. The plan requires the
official rules text, then oracle scenarios, then legacy differential behavior.

