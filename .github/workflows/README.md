# GitHub Actions intentionally disabled

Forge 2.0 verification is local-only by Owner decision. Workflow definitions
are archived under `.github/workflows-disabled/` so pushes, pull requests, and
scheduled events cannot spend GitHub Actions minutes.

Do not move those files back into this directory without an Owner-approved plan
change. Use `scripts/local_verify.sh` and the tier gate scripts instead.
