# T4 Behavioral Guardrails

Status: provisional data path implemented; calibration pending.

Guardrails are soft action-prior penalties loaded from
`assets/ai/action_priors.ron`. They are not legality rules, hard filters, card
scripts, or hidden bonuses. Search may override every penalty when measured
value supports the line.

The strict registry contains tiered penalties for friendly harm, opponent
benefit, unnecessary sacrifice, missed required defense, unfavorable combat
trade, passing with development available, and nonterminal concession. The
parser rejects unknown fields, missing profiles, changed risk ordering, and
positive values that would silently turn a penalty table into a bonus table.

The current production adapter applies only risks proved by typed state:

- passing a main phase while a development action exists;
- an unfavorable blocker trade under visible power, toughness, and deathtouch.

The other registered risks remain dormant until the canonical action surface
can prove them without card-name branches or inferred hidden information. A
registered risk is not evidence that its adapter exists.

Guardrail values remain provisional until paired ablation shows reduced
catastrophic mistakes without a meaningful decline across arena outcomes,
acceptable-action decisions, and blinded human review.
