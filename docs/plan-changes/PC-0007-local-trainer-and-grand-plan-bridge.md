# PC-0007: Local Trainer and Grand Plan v2.0 bridge

Date: 2026-07-10

Status: Accepted by the Owner on 2026-07-10 after Gate Reviewer recommendation.

Source record: `docs/transition/GRAND_PLAN_V2_INTAKE.json`.

Conflict review: `docs/transition/GRAND_PLAN_V2_CONFLICT_MATRIX.md`.

## Motivation

The Owner has supplied Grand Plan v2.0 and clarified that the project has two
related outcomes: a real local human-playable GPL Forge successor and a
separate report-only PodBench service. The standalone application is also the
research environment for opt-in Owner demonstrations, preferences, and AI
evaluation. The transition must expand the roadmap without replacing the
active v1.7 plan, weakening signed gates, mislabeling structural coverage as
semantic readiness, or authorizing commercial/data uses prematurely.

## Exact Plan Change

If ratified, amend the master plan to v1.8 with these changes:

1. Add a dual-track product charter. Forge Standalone/Local Trainer owns human
   play, local deck loading, AI, replay, trainer UI, and trace export. PodBench
   remains a separate report-only service consuming a pinned headless build.
2. Preserve GPL-3.0-only, no GitHub Actions, local verification, no unapproved
   installs/egress, fail-closed translation, and every signed gate through
   CP-DSL. The bundled v1.3 Forge snapshot has no authority over v1.7 history.
3. Split card reporting into scope classification (`in_v1_scope`, `out_of_v1`,
   `catalog_only`) and implementation maturity (`absent`, `parsed`,
   `mapped_partial`, `structurally_translated`, `compiler_valid`,
   `runtime_smoke_passed`, `semantic_verified`, `pod_integration_verified`,
   `ai_supported`, `product_eligible`). Each transition requires generated
   evidence; units must not be mixed in one funnel.
4. Rename breadth proxies in generated/public status surfaces. Use
   `structurally_tested_uses`, `compiler_valid_translated_definitions`,
   `distinct_scenario_commands`, `observed_semantic_atom_combinations`, and
   `cross_compile_artifacts_passed`. Preserve serialized compatibility where a
   direct enum rename would invalidate existing artifacts; mark old labels as
   deprecated until a versioned migration lands.
5. Finish the current bounded PC-0006 mapper batch. Then run T3.3, T3.5, T3.6,
   and integration work concurrently at a planning allocation of 40% mapper
   breadth, 30% generated runtime smoke, 20% card-specific semantic evidence,
   and 10% complete deck/pod integration and telemetry.
6. Extend the blocker planner with versioned completion units after the first
   card/deck selections exist: complete reference/training decks, four-deck
   pods, semantic strata, trainer scenarios, archetype gain, priority gain,
   global gain, and measured engineering minutes. Cards-per-hour may not be
   optimized independently from semantic defects.
7. Add T3.5 capability-specific runtime-smoke synthesis. An unsupported setup
   is reason-coded and does not pass. Add T3.6's first 100-card Commander
   semantic gold set and CP-CARD-SEMANTICS-100.
8. Add four compiled integration decks and CP-FOUR-PLAYER-POD: four seats at 40
   life, commanders/command zone/tax, normal legal actions, production
   casting/priority/triggers/combat/elimination, deterministic replay,
   per-seat hidden-information canaries, and at least 1,000 invariant-clean
   seeded games with runtime/resource metrics.
9. Add T1.R10 and CP-HUMAN-PLAY-CLI. A human must complete a real local game
   through legal-action prompts against a random-legal or heuristic bot; no
   scripted winner or post-start direct mutation is accepted. Replay must
   reproduce every human and AI action.
10. Bring forward a focused T5 desktop Trainer slice after the CLI checkpoint:
    board/hand/zones, stack/priority, targets/modes/payments, combat, card text,
    deck loading/setup, replay timeline, post-game decision review, and capture
    controls. Mobile, quest, draft, collection, and public WASM remain on the
    later standalone roadmap and are not PodBench prerequisites.
11. Add a human-learning bridge with versioned `GameManifest`,
    `DecisionRecord`, and `PostGameReview` contracts; D0-D6 dataset lifecycle;
    replay-family split isolation; exact `PlayerView` reconstruction; opt-in
    capture; hidden-information canaries; immutable dataset manifests; and
    CP-HUMAN-TRACE / CP-TEACHER-CORPUS-ALPHA.
12. Human moves are demonstrations/preferences, not automatic optimal labels.
    Human-derived training cannot begin until the applicable V00 dataset row
    is approved, CP-AI-BENCH, CP-HUMAN-TRACE, and
    CP-TEACHER-CORPUS-ALPHA pass, and replay-family-isolated splits are fixed.
    Promotion is a separate decision requiring ablation evidence plus V02
    Track A outcome, Track B decision, and Track C blinded-human evidence to
    remain independently green under CP-HUMAN-LEARNING-PROMOTION.
13. Make requested inactive AI capabilities fail nonzero or with a distinct
    `NOT_IMPLEMENTED` result. Only explicit non-gate `--allow-skip` may convert
    that state to success. Metrics distinguish `not_run`, `not_implemented`,
    `run_failed`, and `passed`.
14. Adopt a two-commit T3+ evidence model: product commit first, clean detached
    verification of that exact commit/tree, then evidence-only commit. Evidence
    records product/tree/generator/evidence commits, source-scope and lockfile
    hashes, commands, logs, and generation time; stale target evidence fails.
15. Add changed-line and subsystem coverage targets without replacing the 80%
    workspace floor: changed lines >=90%, forge-porttools >=85%, planner branch
    coverage >=90%, registered mapper families with tests 100%, and curated
    mapper/planner mutation score >=90%. Introduce gates only after the local
    tool reports are reproducible.
16. Schedule behavior-preserving core/mapper module extraction after the
    current T3 batch and before T4/trainer coupling. Schedule display/runtime/
    format artifact splitting only after size, decode, RSS, startup, clone, and
    legal-action baselines plus an ADR.
17. Add `STATUS.md` and generated `metrics/card_maturity.json` as the truthful
    owner-facing technical scoreboard. No single percentage is the project
    north star.
18. Correct active repository metadata to `Saint-Sam/DeckTest`. Historical
    evidence retains the repository name that was true when recorded.
19. Add an immutable T1.10 gate addendum recording the Owner-approved broader
    local legacy test source, exact mapped/covered/blocked/not-meaningful rows,
    and the fact that no ranked top-100 source existed. Do not rewrite the
    historical gate as though a literal upstream top-100 was ported.
20. Prepare GitHub-visible review metadata through local task branches, exact
    sanitized evidence attachments, and independent review records for
    `Saint-Sam/DeckTest` only. The Owner performs or explicitly approves every
    push/PR egress action. Private datasets, model artifacts, Grand Plan
    business files, and unrelated repository data are excluded. PC-0001 still
    prohibits GitHub Actions; no hosted check is required or triggered.
21. Complete a dependency/ADR audit before the next T3 integration. Record the
    existing Rayon use as porttools-only production orchestration outside the
    kernel, pinned by `Cargo.lock`, with a machine-enforced 24-worker ceiling,
    deterministic translator and planner replay across worker counts, and a
    manifest/import boundary guard. Criterion's already-approved benchmark
    dev graph may contain transitive Rayon; `forge-core` has no direct,
    non-dev, imported, or production/runtime Rayon dependency. Any other
    unrecorded direct dependency receives an ADR or is removed. ADR-0012 needs
    a separate Gate Reviewer disposition and passing offline `cargo deny`
    evidence before acceptance.
22. Preserve the historical T1.11 kernel signoff while correcting its product
    claim: the scripted terminal demonstration is demo-only evidence, not a
    human-finishable client. Record a signed-tier addendum marking human play
    pending T1.R10 and CP-HUMAN-PLAY-CLI; do not treat T1.11 as satisfying the
    interactive expectation ledger until that checkpoint passes.
23. Extend the Owner input map with O12 at CP-HUMAN-PLAY-CLI (complete one real
    local game and replay, estimated 45-120 minutes), O13 at
    CP-TEACHER-CORPUS-ALPHA (20 games, 500 validated choices, and 100 reviewed
    states, estimated 20-40 hours spread across sessions), and O14 at
    CP-TRAINER-UI (complete one prepared desktop Trainer game and post-game
    review script, estimated 60-120 minutes). Agents provide copy-paste launch
    material and the review checklist first. No corpus quota is silently
    treated as automatic approval or an optimal label.

## New Tasks And Checkpoints

| ID | Purpose | Blocks |
| --- | --- | --- |
| CP-STATUS-TRUTH | Generated two-axis maturity and truthful terminology | Broad readiness/coverage claims |
| T3.5 | Capability-specific generated runtime smoke | Runtime-clean card claims |
| T3.6 | First 100-card semantic Commander gold set | Semantic/reference claims |
| T3.9 | Four complete compiled integration decks | CP-FOUR-PLAYER-POD |
| CP-CARD-SEMANTICS-100 | Gold-set semantic evidence | Reference deck/pod claims |
| CP-FOUR-PLAYER-POD | Real deterministic four-seat card-driven path | Serious AI arena/PodBench worker claims |
| T1.R10 | Interactive local CLI completion | Human play checkpoint |
| CP-HUMAN-PLAY-CLI | Owner completes a real CLI game | Meaningful trace collection |
| T4.H1 | Trace schemas, validators, and dataset manifests | CP-HUMAN-TRACE |
| CP-HUMAN-TRACE | Replay/view/legal-action integrity | Teacher corpus collection |
| CP-TEACHER-CORPUS-ALPHA | Diverse validated Owner corpus | Human-derived training |
| CP-HUMAN-LEARNING-PROMOTION | Track A/B/C plus ablation | Promoted learned component |
| CP-TRAINER-UI | Owner game and post-game review in desktop Trainer | Trainer usable claim |
| CP-PODBENCH-WORKER | Pinned promoted build emits V04 artifact | Real S1 reports |

## Affected Existing Tasks

T1.10, T1.11, T3.3-T3.9, T4.1-T4.8, selected T5 tasks, T7 packaging,
CP-PORT-20, project metrics, evidence packets, Owner briefs, and release
acceptance.

## Explicit Non-Authorization

This PC does not authorize public signup, ads, payments, public report shares,
production external card/deck sources, customer/user training data, distribution
of trainer/WASM builds, or any claim that an API boundary resolves GPL. Passing
CP-PODBENCH-WORKER only proves the worker contract and does not authorize S1
exposure or launch. Those remain V00/professional and Owner decisions.

## Risks And Mitigations

- Scope expansion can starve the card factory. Keep the current batch, use the
  40/30/20/10 allocation, and cap work in progress.
- Structural progress may still be overstated. Use the generated maturity
  funnel and literal checkpoint names.
- One Owner corpus can encode personal bias. Preserve confidence, acceptable
  alternatives, sealed evaluation, and three independent V02 tracks.
- The local trainer and report service may blur together. Keep separate repos,
  customer capabilities, deployment artifacts, and Owner gates.
- Evidence commits can drift from source. Bind every T3+ packet to the exact
  product commit/tree and reject stale targets.

## Ratification

Gate Reviewer recommendation: approve the exact revision recorded in
`docs/transition/GRAND_PLAN_V2_INTAKE.json` (2026-07-10).

Owner approval: accepted substantive SHA-256
`8a63ec707562fd50353b028c27152dd215186b014c35af919e7c870f2a02aed6`
on 2026-07-10. The Owner also selected the private/public repository split and
retained personal control of GitHub push/PR egress. See
`reports/gates/PC-0007/OWNER_APPROVAL.md`.

ADR-0012 disposition: separate Gate Reviewer decision required; PC approval
does not substitute for dependency approval.

Approved sentence:

`I approve PC-0007 as drafted: preserve standalone Forge, add the Local Trainer,
PodBench bridge, truthful maturity model, semantic/pod/human-play checkpoints,
and governed human-teacher program, while retaining no GitHub Actions and all
existing GPL/IP/egress gates. The approval applies only to the exact PC-0007
SHA-256 stated in this thread; I will perform or explicitly approve all GitHub
push/PR actions.`
