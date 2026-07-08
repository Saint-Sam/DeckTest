# Scryfall Local Cache

This directory is for local Scryfall bulk-data cache files used during Forge 2.0
development and gate evidence runs.

Policy:

- Cache card metadata and oracle text only.
- Do not download or commit card art, set symbols, official fonts, or image
  payloads.
- Do not commit bulk JSON files from this directory; they are intentionally
  ignored by `.gitignore`.
- Record reproducible source URLs, timestamps, sizes, and hashes in reports.

Current CP-LAYERS remediation cache:

- Manifest: `bulk-data.json`
- All-cards cache: `all-cards-20260707213530.json`
- Source type: Scryfall `all_cards`
- Updated at source: `2026-07-07T21:35:30.319+00:00`

