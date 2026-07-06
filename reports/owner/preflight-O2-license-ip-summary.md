# O2 License and IP Summary

Date: 2026-07-05

## Decision Requested

Please confirm that Forge 2.0 will be released as **GPL-3.0-only** and will follow the Magic: The Gathering IP rules in the rebuild plan.

## What GPL-3.0-Only Means Here

The old Forge project and its card scripts are GPL-3.0. Because Forge 2.0 is allowed to mine, translate, or mechanically convert parts of that legacy material, the plan treats the whole Forge 2.0 project as GPL-3.0-only.

In plain terms: Forge 2.0 is not being built as proprietary software. Anyone who receives a distributed copy must receive the GPL-3.0 rights that come with it, including the right to inspect, modify, and redistribute the code under the same license.

## What We May Use From Legacy Forge

The team may use legacy Forge as a source of data and behavior:

- card scripts and tests as source material for the new card pipeline;
- old AI profiles as inspiration for new AI behavior;
- rules-engine behavior as a reference when practical behavior is unclear.

The team should not copy old code into the new architecture casually or pretend translated legacy files are newly independent. If a file is mechanically translated from legacy card data, AI data, resources, or tests, it stays under the GPL-3.0 posture.

## Magic IP Rules

Magic: The Gathering belongs to Wizards of the Coast. Forge 2.0 must not ship official Wizards-owned visual assets.

That means the project must not include:

- official card art;
- official set symbols;
- official mana-symbol fonts or copied official SVG symbols.

The project may use card names, rules text, and oracle text as game data under the same fan-project posture used by legacy Forge. The app and README must include the standard Fan Content / unaffiliated notice so users understand this is not made by or endorsed by Wizards of the Coast.

## Card Images and Symbols

Forge 2.0 may fetch card images from Scryfall only when the user requests that behavior. The app must identify itself with a proper User-Agent, respect a client cap of no more than 10 requests per second, prefer Scryfall bulk data for metadata, and cache downloaded images locally.

Before any image download, the first-run experience must plainly explain what will be downloaded and from where. The game must also remain fully playable with no downloads at all.

Mana, tap, and set symbols must be original in-house vector art with a distinct style, not copied from official fonts or images.

## Network and Offline Expectations

The app must have an offline mode that disables all network access across the app. The web demo build must default to offline mode.

For release, the owner should expect a final human audit of credits, licenses, first-run consent, store copy, and network behavior before approval.

## WHAT YOU SHOULD EXPECT NEXT

After you confirm O2, agents can proceed with the GPL-3.0-only assumption and the IP guardrails above as binding project rules. Later owner briefs and release checks should show these rules in action: GPL license notices, Fan Content / unaffiliated notices, no bundled official art or symbols, no network access before consent, and a working offline mode.

## WHAT WE NEED FROM YOU

Reply with approval if this matches your intent. Suggested wording: **"I confirm O2: Forge 2.0 is GPL-3.0-only and must follow the §1.4 and §10.8 IP rules."**
