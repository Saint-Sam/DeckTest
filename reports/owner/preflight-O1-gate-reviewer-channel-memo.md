# Pre-flight O1 Memo: Gate Reviewer and Owner Channel

Date: 2026-07-05
For: Owner
Decision needed: approve or assign the Gate Reviewer for ADR-0007, and choose the Owner contact channel for ADR-0008.

This is a pre-flight blocker. The project should not start tier work until the Gate Reviewer is designated and reachable, and until the Owner channel is recorded.

## Gate Reviewer Candidates

### Option A: Human Gate Reviewer

A trusted human reviewer outside the build team signs gates and checkpoints.

Tradeoffs:
- Best for accountability, product judgment, and final confidence.
- Strongest fit for decisions that may blend technical risk with project risk.
- Slower if the reviewer is busy; gates may wait on availability.
- Needs enough technical context to challenge tests, specs, and evidence bundles.

Use this if you have a reliable technical reviewer who can spend focused review time at each gate.

### Option B: Designated Strong Reasoning Model

A separate strong reasoning model is assigned only to gate review. It must not be the Orchestrator and must not be any agent that implemented or reviewed the tier.

Tradeoffs:
- Fastest to schedule and easiest to invoke repeatedly.
- Good fit for adversarial probing, checklist discipline, and consistency.
- Weaker than a human on final ownership judgment and external accountability.
- Still needs clear invocation instructions and preserved review records.

Use this if you want the project to move quickly while keeping gate review separate from implementation.

### Option C: Hybrid Reviewer

A strong reasoning model performs the first adversarial review, then a human spot-checks the sign-off at major checkpoints.

Tradeoffs:
- Best balance of speed and human oversight.
- Keeps routine gates moving while reserving human attention for high-risk moments.
- Requires clear rules for when human review is mandatory.
- Slightly more coordination overhead than a single reviewer.

Use this if you want the strong model to carry normal gate workload, with human escalation for CP-LAYERS, plan changes, de-scope decisions, release, licensing, IP posture, credits, and network egress.

Recommended default: Option C if a human reviewer is available for major checkpoints; otherwise Option B. Option A is strongest when you have a reviewer who can respond promptly.

## Owner Contact Channel Options

### Option 1: Dedicated Codex Thread

Owner-facing briefs, trouble bulletins, and O1/O2 asks are sent in one named Codex thread.

Tradeoffs:
- Keeps context next to the work.
- Easy for agents to reference and update.
- Best if the Owner is already using Codex daily.
- Less useful if the Owner expects notifications elsewhere.

### Option 2: Email

Owner-facing artifacts are sent to one chosen email address.

Tradeoffs:
- Durable, searchable, and easy to forward.
- Good for formal decisions and audit trail.
- Slower for urgent questions unless the Owner monitors it closely.
- Agents need the exact address and expected response-time norm.

### Option 3: Chat Channel

Owner-facing artifacts are sent to a dedicated Slack, Discord, Teams, or similar channel.

Tradeoffs:
- Fastest for short acknowledgments and urgent blockers.
- Good for "please reply proceed" moments.
- Easier for important decisions to get buried unless artifacts are also linked clearly.
- Needs a rule that decisions are explicit, for example: "approved", "acknowledged", or "proceed".

Recommended default: use the channel the Owner already checks most reliably. If there is no clear preference, use a dedicated Codex thread for context plus email for formal decision records.

## What The Gate Reviewer Must Be Able To Do

The Gate Reviewer must be reachable, independent from tier implementation, and willing to look for ways the tier could be wrong rather than simply confirming green test output. The role has authority to write gate sign-offs, answer Gate-Reviewer Question Queue items, approve ADRs assigned to the Gate-Reviewer column, and recommend plan amendments. The Orchestrator cannot hold this role or self-certify a gate.

## WHAT YOU SHOULD EXPECT NEXT

After you choose the reviewer and channel, the Orchestrator records them in ADR-0007 and ADR-0008, then proceeds with pre-flight setup. The first Owner artifact after this should be the T0 tier-start Owner Brief before code work begins.

## WHAT WE NEED FROM YOU

Reply with:

1. Gate Reviewer choice: Option A, B, or C, plus the reviewer name or invocation/contact instructions.
2. Owner channel choice: Codex thread, email, chat channel, or another channel you prefer.
3. Any response-time expectation, for example "urgent O1/P0 asks within 24 hours; routine weekly heartbeat by Friday."
