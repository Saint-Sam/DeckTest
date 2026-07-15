# ADR 0037: Normalized Benchmark Identity

## Status

Accepted for T4 diagnostics and benchmark split enforcement. Exact replay
identity remains unchanged.

## Context

`DecisionContextId`, `DecisionStateKey`, and `CanonicalActionId` intentionally
contain game-local object, ability, trigger, and stack-entry handles. That is
correct for exact replay, but equivalent runtime states assembled in a
different allocation or registration order can receive different exact IDs.
Using only those IDs for benchmark deduplication or leakage checks would miss
cross-split duplicates.

## Decision

`DecisionContext` additively carries a separate `NormalizedBenchmarkKey`, a
normalized actor-visible view hash, and a sorted normalized legal-action
multiset. Actor-visible objects are classified by complete visible
characteristics, strategically ordered zone position where applicable, and
normalized attachment or copy relationships. Unordered visible zones are
canonicalized independently of allocation order.

Production adapters bind game-local activated abilities and triggers to an
immutable program identity plus their normalized source. Stack entries bind to
their visible stack position and complete normalized stack semantics, including
source, targets, payment, copy provenance, and announced choices. Hierarchical
path discriminators remain in the normalized key, so distinct legal choice
paths do not collapse.

Normalization fails closed. A missing runtime binding or unknown object
reference marks the record incomplete and retains exact identity material in
the fallback key. Incomplete records cannot pass benchmark evidence gates.

## Evidence

The deterministic runtime fixture constructs equivalent states with different
object allocation, mana-source creation, ability registration, and runtime
handle order. It also constructs distinct hierarchical paths and unequal
ability semantics. The fixture must show:

- exact replay IDs are unchanged by semantic bindings;
- intended equivalent states share one normalized key;
- exact runtime identities remain distinct;
- hierarchical paths and unequal semantics remain distinct; and
- every fixture binding is complete.

Exact AI replay records persist both identity families. Decision-state audits
use normalized keys for benchmark deduplication while continuing to verify the
recorded exact key/signature contract.

## Promotion Boundary

Normalized keys are evidence and dataset-split identities only. They do not
replace exact replay IDs, kernel object handles, search transposition keys, or
action-selection membership checks. Passing the runtime fixture does not prove
sealed benchmark separation; campaign manifests, hidden canaries, and Owner
promotion remain separate gates.
