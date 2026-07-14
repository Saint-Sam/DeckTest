# T4 Arena Calibration

Status: local diagnostics infrastructure implemented; no tier is promoted.

## Purpose

`forge-arena` calibrates policy tiers with real four-player games while keeping
controller attribution, seeds, seats, decks, determinizations, and worker
configuration inspectable. It never treats a successful process exit as a
strength pass.

`--ladder` measures each configured tier against the tier below. Each pair uses
one physical game seed twice. Candidate and baseline controllers occupy
opposite seat parities, then swap controller ownership while deck and physical
seat assignments remain fixed. The report records every winner, final state
hash, action/search counters, and Wilson interval.

`--calibrate` is the provisional competence-band tool. It searches measured
budgets for the smallest candidate reaching 65-75% against the prior tier. It
does not rewrite `assets/ai/ai_tiers.ron` because the current one-pod campaign
lacks the required archetype, sealed-label, and platform evidence.

`--search-knee` is the separate diminishing-return experiment. It compares
every adjacent exact B/2B pair and performs fixed/adaptive ablations on the same
paired protocol. Missing Track B, CPU, cost, or acceptable-action evidence is
serialized as unavailable and prevents a knee from being selected. Details and
provisional thresholds live in `T4_SEARCH_KNEE.md`.

## Resource Bound

Arena pair workers and per-game search workers share a hard 24-worker ceiling.
The immutable deck manifest is compiled once and shared. Per-budget latency
samples remain separate so p50/p95/p99 values are not computed from a pooled
mixture of policies.

## Promotion Boundary

A promotable calibration requires the development, validation, and sealed
splits; three materially different archetype tracks; acceptable-action and
catastrophic-blunder labels; hidden-information canaries; CPU and memory
measurement; reference desktop, Android, and WASM latency; and Owner
CP-AI-LADDER review. Current results are diagnostics only.
