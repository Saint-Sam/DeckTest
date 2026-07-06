# ADR-0001: GPL-3.0-Only License And Magic IP Posture

Date: 2026-07-06

## Status

Accepted by Owner pre-flight O2.

## Context

The rebuild may mine, translate, or mechanically convert legacy Forge card
scripts, AI data, resources, and test behavior. Legacy Forge is GPL-3.0, and
the master plan binds the rebuild to the same license posture for derived
distribution.

Magic: The Gathering is Wizards of the Coast intellectual property. The rebuild
must remain a fan project with no shipped official visual assets.

## Decision

Forge 2.0 is GPL-3.0-only.

Forge 2.0 must not ship official card art, official set symbols, official mana
symbol fonts, or copied official visual assets. User-requested card image
fetching may use Scryfall only with consent, clear identification, rate-limit
compliance, and local caching. The app must stay playable with no downloads and
must include an unaffiliated Fan Content notice.

## Consequences

All source and distributed artifacts must preserve GPL-3.0-only terms. Release
work must include a license and IP audit before public artifacts are produced.

Network behavior related to card images is consent-gated and optional. Offline
mode is a product requirement, not a polish task.

## Alternatives Considered

Permissive or proprietary licensing was rejected because the project expects to
derive substantial value from GPL legacy Forge data and behavior.

Bundling official card art or symbols was rejected because it violates the IP
guardrails in the master plan.

