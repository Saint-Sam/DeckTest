# ADR-0014: T4 Neural Policy/Value Path Go/No-Go

Date: 2026-07-14

## Status

Proposed. Owner decision required at CP-NN-GO. The implementation recommendation
is **no-go for T4**; this document does not approve that decision on the
Owner's behalf.

## Context

T4.8 permits an optional learned policy/value network only if it beats the
T4.5 Master search baseline in at least 60% of paired games at equal measured
latency. Adding the path would also require an inference dependency decision,
model/data provenance, deterministic export, device validation, and the human
learning gates. None of those conditions is presently green.

The current priority is to complete and calibrate the typed heuristic/search
baseline. Track B labels, three archetype campaigns, reference-device latency,
CP-AI-BENCH, CP-HUMAN-TRACE, CP-TEACHER-CORPUS-ALPHA, and V00 authorization are
not yet available. There is therefore no valid training corpus or equal-latency
baseline against which a network could satisfy the shipping bar.

## Proposed Decision

Do not add `ort`, `candle`, ONNX artifacts, training code, or an `nn` product
feature during the current T4 checkpoint. Finish the deterministic baseline,
retain versioned training/trace contracts, and reopen the NN option only after
the prerequisite gates and an equal-latency experiment exist.

This is a feature no-go, not removal of the future experiment. It adds no
dependency, changes no license/IP posture, and makes no learned-strength claim.

## Consequences

- T4 remains focused on measurable typed search and calibration.
- Android/WASM size, startup, and latency are not burdened by an unused runtime.
- No human-derived data is trained or relabeled prematurely.
- A later go decision requires a new accepted ADR naming the inference stack,
  model license/provenance, feature boundary, export process, and measured
  60%-at-equal-latency evidence.

## Owner Decision

Pending. CP-NN-GO may accept the proposed no-go, reject it and authorize a
bounded experiment, or request more evidence. Until then, T4.8 remains open and
no learned component ships.
