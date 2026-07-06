# ADR-0008: Owner Channel

Date: 2026-07-06

## Status

Accepted by Owner pre-flight O1.

## Context

The master plan requires Owner-facing briefs, weekly heartbeats, trouble
bulletins, and input requests to be delivered on the Owner's chosen channel and
recorded in the repository.

## Decision

Use this Codex thread as the primary Owner channel.

Also mirror durable Owner-facing artifacts through the GitHub repository
attached to this Codex project once the repository remote is configured.

Urgent O1/P0 asks have a 24-hour response expectation. Routine weekly
heartbeats are due by Friday.

## Consequences

Every Owner-facing artifact must live under `reports/owner/` or
`reports/status/` and must be summarized in this Codex thread. Once the GitHub
remote is attached, those artifacts should also be visible through normal repo
history and PR/review flow.

Until the GitHub remote is known, this ADR records the GitHub channel as pending
remote configuration rather than inventing a repository URL.

## Alternatives Considered

Email and external chat were considered but not chosen for the primary channel.
They can be added later by a superseding ADR if needed.

