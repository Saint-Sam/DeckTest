# Grand Plan v2.0 Transition Conflict Matrix

Status: ratified through Owner-approved PC-0007 and GP-PC-0001 on 2026-07-10.

Pinned source: `docs/transition/GRAND_PLAN_V2_INTAKE.json`.

| Domain | Active Forge v1.7 | Grand Plan v2.0 | Relationship | Proposed disposition |
| --- | --- | --- | --- | --- |
| Plan version | Current signed history through PC-0006 | Bundled Forge snapshot is v1.3 | Conflict | Never overwrite active plan/state; bundled snapshot is historical only |
| Product outcome | Human-playable GPL Forge successor across local clients | Customer-facing product is report-only | Coexists | Preserve standalone Forge; PodBench remains a separate report-only product |
| Repository truth | Actual public remote is `Saint-Sam/DeckTest` | V01 says private while memo says public | Conflict | Correct public repository metadata; production service and research stores may remain private |
| License | Forge code/content is GPL-3.0-only | Private service may consume a pinned worker | Interface plus legal gate | Keep public GPL engine; V00/counsel determines worker/service/conveyance facts; no legal-safe-harbor claim |
| GitHub verification | PC-0001 prohibits GitHub Actions; heavy verification is local | Suggests cheap hosted checks if budget allows | Conflict | No Actions; prepare only sanitized `Saint-Sam/DeckTest` review material locally and let the Owner perform/approve egress |
| Network and installs | Ask Owner before egress/install; routine T3 is offline | Production service eventually needs approved providers | Stage conflict | Forge remains local/offline now; service integrations stay blocked behind V00/V06/V07/V14 |
| T3 priority | Structural mapper breadth and PC-0006 throughput | Start runtime smoke, semantic evidence, and pod integration now | Compatible reprioritization | Finish current bounded batch, then allocate 40/30/20/10 across mapper/smoke/semantic/pod work |
| Card status | Catalog scope/classification and compiler translation are separate but labels remain overloaded | Requires scope and implementation-maturity axes | Compatible correction | Add generated maturity funnel; deprecate semantic use of `playable` and `verified` breadth proxies |
| Mapper planner | Global/priority/fan-out/effort weighted batches | Add complete deck/pod/semantic/trainer completion units and measured effort | Extension | Add versioned completion-unit input after four integration decks and semantic set are selected |
| Semantic evidence | T3.6 exists after factory stages | Begin first 100-card Commander gold set immediately | Compatible acceleration | Add CP-CARD-SEMANTICS-100 before reference/pod claims |
| Runtime smoke | T3.5 is planned | Generate capability-specific states for every compiler-valid definition | Compatible acceleration | Unsupported setup synthesis becomes a reason-coded result, never an implicit pass |
| Commander integration | T2 hooks and small scenarios exist | Requires real card-driven four-seat pod | Evidence gap | Add four integration decks and CP-FOUR-PLAYER-POD with 1,000 deterministic seeded games |
| Human CLI | T1.11 scripted demo was accepted at gate | Owner requires a real interactive complete game | Evidence gap | Preserve the signed kernel gate, record T1.11 as demo-only in a signed-tier addendum, and add T1.R10/CP-HUMAN-PLAY-CLI |
| Local UI | Full T5 UI comes later | Focused desktop Trainer is needed earlier | Priority interface | Bring forward board/prompts/replay/review slices, then require Owner CP-TRAINER-UI play/review; defer unrelated mobile/content surfaces |
| AI readiness | `forge-ai` is currently a boundary/bootstrap | V02 requires Track A/B/C, PilotIntent, threat model, and promotion gates | Compatible future scope | Inactive requested capabilities fail closed; build reproducible heuristic/search baseline before learning |
| Human traces | Not in current plan | Owner demonstrations/preferences become governed research data | New interface | Add versioned GameManifest/DecisionRecord/PostGameReview and CP-HUMAN-TRACE; no training authorization yet |
| Hidden information | `PlayerView` boundary is mandatory | Training inputs must reconstruct exact actor view | Coexists | Full-state replay may aid debugging but cannot enter model input or labels |
| Training | T4 optional learned path | Human-informed learning sequence L0-L5 | Gated extension | No human-derived training before V00 dataset status, CP-AI-BENCH, CP-HUMAN-TRACE, CP-TEACHER-CORPUS-ALPHA, and fixed splits; promotion is separately gated |
| Service boundary | T8 networking deferred; standalone may eventually ship clients | PodBench worker is private and customer receives reports only | Distinct product interface | Add a narrow worker contract behind applicable Forge, V00-V06, boundary, SBOM/provenance, and conveyance evidence; worker pass does not authorize S1 exposure |
| Local parallel dependency | PC-0006 evidence uses Rayon-backed porttools sweeps without a prior dependency ADR | V01 requires reviewed dependency/provenance boundaries | Governance defect | Enforce 1-24 workers, compare translator/planner output at 1 vs N, guard imports/manifests, pass offline deny, and obtain separate ADR-0012 Gate Reviewer disposition before next T3 integration |
| Evidence binding | Current metrics may be generated before final source commit | Requires exact product-commit evidence | Process correction | Adopt product commit then clean detached evidence commit model for T3+ gates |
| Coverage | Workspace floor is 80% | Adds changed-line, per-crate, planner branch, mapper-family, and mutation floors | Compatible strengthening | Introduce incrementally without weakening current gate; tool availability remains local-only |
| Source structure | Core and mapper files are large | Mechanical extraction before T4/trainer expansion | Compatible maintenance | Schedule behavior-preserving split after current T3 batch and before AI/trainer coupling grows |
| Card database | Catalog and mechanics currently share artifacts | Split display, runtime mechanics, and format indexes | Compatible future optimization | Measure size/decode/RSS first; preserve current format until an ADR and migration test exist |
| Commercialization | No release/payment decision is authorized | V00-V14 define staged S0-S5 business work | Owner/legal gate | S0 private feasibility only; public signup, ads, payments, public shares, production sources, and distribution remain blocked |
| Brand/product name | Forge 2.0 is the standalone project; remote is DeckTest; PodBench is proposed service name | PodBench branding requires clearance | Interface plus Owner gate | Correct repository URL now; defer public product naming/branding to V00/V05/Owner |

## No-Reset Rule

Signed Forge gates, PC-0001 through PC-0006, current T3.3 work, generated
metrics, and local-only controls remain in force. Grand Plan integration adds
interfaces and evidence; it does not reopen rules architecture without a
specific regression or gate failure.
