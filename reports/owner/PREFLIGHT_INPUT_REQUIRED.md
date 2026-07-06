# PREFLIGHT INPUT REQUIRED

Date: 2026-07-05

The Forge rebuild plan stops here before Tier 0 execution.

Plan Appendix A.0 requires a designated Gate Reviewer in
`docs/adr/0007-gate-reviewer.md` before the Orchestrator may initialize the
project state, emit T0 tickets, register recurring jobs, or start code work.
Plan Section 17.3 also requires Owner input O1 and O2 at pre-flight.

## Files prepared for your review

- `reports/owner/preflight-O1-gate-reviewer-channel-memo.md`
- `reports/owner/preflight-O2-license-ip-summary.md`

## What we need from you

1. O1 Gate Reviewer: choose Option A, B, or C from the O1 memo, and provide the
   reviewer name or invocation/contact instructions.
2. O1 Owner channel: choose where Owner briefs, weekly heartbeats, and trouble
   bulletins should be sent.
3. O2 License/IP: confirm whether Forge 2.0 is GPL-3.0-only and must follow the
   plan's Magic IP rules.

Suggested reply:

```text
O1: Gate Reviewer Option C. Use a strong reasoning model for normal gates, with
me as the human reviewer for CP-LAYERS, plan changes, de-scope decisions,
release, licensing, IP posture, credits, and network egress.

Owner channel: this Codex thread for now.

Response time: urgent O1/P0 asks within 24 hours; routine weekly heartbeat by
Friday.

O2: I confirm Forge 2.0 is GPL-3.0-only and must follow the Section 1.4 and
Section 10.8 IP rules.
```

## What happens after your reply

The Orchestrator will record the decisions in ADR-0007 and ADR-0008, initialize
`PLAN_STATE.json`, emit T0.1-T0.6 tickets, write the T0 tier-start Owner Brief,
and only then begin Tier 0 implementation.
