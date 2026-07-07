# CP-LAYERS Owner Decisions

Date: 2026-07-07

Source: Codex thread owner response.

## Decisions

1. Novel scenarios: Codex will interview the owner and turn the owner's answers
   into 15 reviewer scenarios for owner approval.
2. Legacy differential: use local-only search for a legacy Forge/layered subset
   first. Ask the owner before any network access or download.
3. Fuzz: run a longer sanitizer fuzz if it is already installed. If it is not
   installed, ask the owner before installing anything.
4. Signoff: CP-LAYERS is not approved yet. Bring the owner the results, then
   the owner will decide proceed or fail.

## Gate Consequence

T2.5+ remains blocked until the owner approves the 15 scenarios, accepts or
rejects the local legacy differential evidence, reviews fuzz results, and gives
the explicit CP-LAYERS signoff decision.
