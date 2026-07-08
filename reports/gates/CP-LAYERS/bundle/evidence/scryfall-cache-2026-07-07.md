# Scryfall Cache Evidence

Date: 2026-07-07

Purpose: local card-data cache for CP-LAYERS Option 1 remediation.

Policy posture: metadata/oracle text only. No card art, set symbols, official
fonts, or image payloads were downloaded or committed.

## Source

- Bulk type: `all_cards`
- Source updated at: `2026-07-07T21:35:30.319+00:00`
- Download URI: `https://data.scryfall.io/all-cards/all-cards-20260707213530.json`
- Local cache: `cards/scryfall/all-cards-20260707213530.json`
- Local size bytes: `2556468737`
- Manifest SHA-256: `163d78522aa8f3232386398a700aa2d4d815b0b35e7eb66e456e41076f3cd9c4`
- All-cards SHA-256: `eeddab7b413016530fa8996b7898ab8f2a1f33b47e65f7924147382d49cc3d04`

## Counts

- Total card records: `531732`
- English card records: `113234`
- Unique English names: `37790`
- Unique English oracle IDs: `38225`

## Top Languages

| Language | Records |
| --- | ---: |
| en | 113234 |
| ja | 60207 |
| fr | 56901 |
| de | 56567 |
| es | 52146 |
| it | 51520 |
| zhs | 40823 |
| pt | 39546 |
| zht | 23582 |
| ru | 21760 |
| ko | 15382 |
| ph | 49 |

## Top Layouts

| Layout | Records |
| --- | ---: |
| normal | 511570 |
| transform | 4235 |
| token | 3287 |
| art_series | 2650 |
| saga | 1630 |
| adventure | 1578 |
| split | 1495 |
| planar | 1187 |
| modal_dfc | 1148 |
| mutate | 570 |
| prepare | 344 |
| leveler | 311 |

## Gate Note

This cache helps the all-card representation effort and later importer work.
It does not by itself satisfy the CP-LAYERS legacy engine differential; that
still requires a runnable legacy Forge harness plus Forge 2.0 card-script
translation for the selected 100-card layered subset.
