# CP-LAYERS Owner Decisions

Date: 2026-07-07

Source: Codex thread owner response.

## Decisions

1. Novel scenarios: Codex will interview the owner and turn the owner's answers
   into reviewer scenarios for owner approval. The owner later requested a
   stricter 100-scenario synthetic rules stress packet rather than the original
   15-scenario minimum. The owner approved the 100-scenario packet in the
   Codex thread on 2026-07-07 with `approve 100 scenarios`.
2. Legacy differential: use local-only search for a legacy Forge/layered subset
   first. Ask the owner before any network access or download.
3. Fuzz: run a longer sanitizer fuzz if it is already installed. If it is not
   installed, ask the owner before installing anything.
4. Signoff: CP-LAYERS is not approved yet. Bring the owner the results, then
   the owner will decide proceed or fail.

## Gate Consequence

T2.5+ remains blocked until the approved reviewer scenario packet has executable
pass evidence, the owner accepts or rejects the local legacy differential
evidence, reviews fuzz results, and gives the explicit CP-LAYERS signoff
decision.
