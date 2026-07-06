# OWNER BRIEF - T0 Foundations Start

Date: 2026-07-06

## 1. WHAT THIS TIER WILL BUILD

T0 builds the empty but enforceable foundation for Forge 2.0: repository
layout, Rust toolchain checks, CI, gate scripts, metrics plumbing, and the first
legacy inventory report. The outcome is not a playable game yet; it is the
workbench that keeps every later rules and AI task honest.

Expected duration: foundation work starts now and continues until the T0 gate
script, CI shape, and legacy inventory are green.

## 2. WHAT YOU SHOULD SEE - TRY IT YOURSELF

- DO: `scripts/vl.sh`
- EXPECT: once the Rust toolchain is installed, every line should pass and the
  script should end with `ALL CHECKS PASSED`.
- RED FLAG: `command not found` for Rust tools, any red check, or a missing
  script. Reply with the output; an agent investigates within 24 hours.

- DO: open `docs/adr/0007-gate-reviewer.md` and
  `docs/adr/0008-owner-channel.md`
- EXPECT: your O1 decisions are recorded plainly.
- RED FLAG: reviewer/channel text does not match your intent.

## 3. NUMBERS THAT MATTER

- 0 rules scenarios exist yet; T0 creates the machinery, not the rules corpus.
- 15 workspace crates are being bootstrapped to match the plan's architecture.
- 6 platform build targets are listed in the toolchain config.
- 33,290 legacy card scripts were found in the vendored Forge corpus.
- 43,649 legacy ability lines were counted; the most common API is
  `T: ChangesZone`.

## 4. KNOWN ROUGH EDGES

The local machine does not currently expose `rustup`, `rustc`, or `cargo` on
PATH, so the first verification loop cannot pass until T0.1 installs or exposes
the Rust toolchain. GitHub remote mirroring is also pending until the repository
remote is configured.

## 5. WHAT YOU SHOULD EXPECT NEXT

You should see Rust installation happen next. Then the Orchestrator will rerun
the T0 verification loop and gate script.

## 6. WHAT WE NEED FROM YOU

Please complete the Rust install request in `reports/t0/T0.1-install-request.md`.
