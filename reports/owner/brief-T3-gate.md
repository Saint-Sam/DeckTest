# OWNER BRIEF - T3 Card Factory Gate

Date: 2026-07-14

## 1. What Just Happened

Tier 3 reached its approved card-factory exit point and the four-player
checkpoint passed locally. The factory parses every pinned legacy script,
emits 20,082 compiler-valid definitions, keeps the rest visibly quarantined,
and separately proves 100 cards semantically. Four complete Commander decks
then completed 1,000 deterministic four-player games with exact replays and no
invariant or hidden-information failures. Nothing was pushed and no GitHub
Actions, network access, or install was used.

## 2. What You Should See - Try It Yourself

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && python3 tools/write_card_maturity.py --check`
- EXPECT: `PASS card maturity artifacts are current` and `STATUS.md` keeps compiler, runtime, semantic, pod, AI, and product stages separate.
- RED FLAG: Any stale-artifact error or a structural/compiler count described as playable. Reply with the output; it is a P1 evidence defect.

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && scripts/gates/gate_T3.sh`
- EXPECT: The last line says `PASS gate_T3.sh: structural=60.3244% semantic=100/100 pod=1000/1000 coverage=80.3761%`.
- RED FLAG: Any `ERROR`, a semantic count below 100, pod count below 1,000, or coverage below 80%. Reply with the output; work reopens immediately.

- DO: `cd "/Users/juanlopez2016/Desktop/Forge 2.0" && jq '.results | {games_completed, direct_typed_action_replays_matched, invariant_violations, hidden_information_canary_violations}' reports/gates/T3.9/cp-four-player-pod-2026-07-13.json`
- EXPECT: 1,000 games, 1,000 replay matches, and both violation counts at zero.
- RED FLAG: A replay mismatch or nonzero violation count. Do not approve the gate.

## 3. Numbers That Matter

- 33,290/33,290 legacy scripts parse successfully.
- 20,082/33,290 scripts compile, or 60.3244%; 13,208 remain fail-closed and visible.
- 100/100 frozen Commander cards pass card-specific semantic replay.
- 1,000/1,000 four-player games replay exactly with 3,000 eliminations and zero invariant or hidden-information-canary violations.
- Workspace line coverage is 51,845/64,503, or 80.3761%.
- Eight local AddressSanitizer workers completed 3,608 verified worker-seconds and 43,243,299 executions with no final crash artifact.
- The scored Tier 3 mutation gate killed 5/5 declared mutants with zero survivors.

## 4. Known Rough Edges

The four integration decks deliberately reuse a small proven vocabulary: 396
mainboard slots cover 21 unique semantic identities. The four commanders are
compiled and exercised but are not falsely promoted into the frozen semantic
100. The broad card corpus is not yet playable: 13,208 scripts remain
quarantined, and compiler-valid does not mean semantically verified. Human
prompted play, AI quality, UI, packaging, and release remain later gates.
The integration controller is not a balanced AI: the fixed seat/deck win split
was `721/22/257/0`. That does not invalidate execution/replay coverage, but it is
not evidence of AI quality or deck balance; T4 must establish those separately.

The pre-final sanitizer campaign found an adversarial RON nesting stack overflow.
Tier 3 did not waive it: the parser now fails closed beyond depth 128, the exact
crashing input replays safely, a focused regression and mutation protect the
boundary, and the complete sanitizer campaign then passed.

## What You Should Expect Next

T1.R10 is now unblocked: the next visible milestone is a genuinely prompted
local game whose human and bot choices replay exactly. After that, the initial
T4 AI baseline can build on the card-driven four-player path proven here.

## What We Need From You

Read this brief and reply `acknowledged` when convenient. This is the normal O6
tier-gate acknowledgment, not technical approval and not authorization to push
or release.
