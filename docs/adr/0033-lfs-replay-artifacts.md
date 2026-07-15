# ADR 0033: Store Exact Replay Artifacts in Git LFS

- Status: accepted
- Date: 2026-07-15
- Scope: generated `.frsreplay` gate evidence

## Context

Exact T4 replay artifacts grew beyond GitHub's ordinary-blob size limit. The
57 commits not yet present on `origin/main` contained 95 replay revisions and
approximately 4.4 GB of ordinary Git blobs. Uploading every superseded replay
revision through LFS would preserve redundant generated snapshots and consume
substantially more remote storage than the current gate evidence.

## Decision

1. Every `.frsreplay` path is tracked by Git LFS through `.gitattributes`.
2. Only the unpushed commit range was rewritten. The existing remote history
   and all source, specification, test, metric, and report changes were kept.
3. Superseded replay revisions were removed from that unpushed range. The
   current T1, T1.R10, T2, T3, T3.9, and T4.3 replay artifacts were restored in
   one LFS checkpoint.
4. The original local chain remains recoverable from
   `backup/pre-lfs-20260715`; that recovery branch is not intended for push.
5. T4 product and evidence commit identifiers must be rebound after the
   rewrite. No pre-rewrite hash may be presented as current evidence.

## Consequences

- Fresh Git LFS-aware clones receive the exact current replay bodies while the
  ordinary Git pack stores only small pointer records.
- The public branch avoids roughly 4.1 GB of redundant historical replay
  uploads while retaining the latest exact evidence for every completed tier.
- Historical source commits remain reviewable, but intermediate unpushed
  replay snapshots are available only on the local recovery branch and can be
  regenerated from their corresponding product code when needed.
- Gate scripts continue reading normal working-tree replay files; no runtime or
  verification semantics change.

## Verification

- Inspect every staged `.frsreplay` entry as an LFS pointer with its SHA-256 and
  byte size.
- Scan `origin/main..main` for ordinary Git blobs above GitHub's limit.
- Re-run exact T4 replay, decision-key, latency, and preflight evidence against
  the rewritten product commit before promotion review.
