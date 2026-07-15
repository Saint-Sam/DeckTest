#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

# The evidence commit is intentionally older than the evidence-only commits
# that may be present in the worktree.  Never infer this binding from HEAD.
PRODUCT_COMMIT="19ef3302c40db3e916d2a60925546d4ebc28608d"
PRODUCT_TREE="e79efa91e0146f23f7219367e117db34ce13867a"

python3 tools/verify_t4_regression_pod.py \
  --fixture assets/ai/pods/regression-v1.json \
  --manifest reports/gates/T4-CARDS/regression-v1-manifest.json \
  --product-commit "$PRODUCT_COMMIT" \
  --product-tree "$PRODUCT_TREE"

python3 tools/build_t4_card_admission.py \
  --check \
  --product-commit "$PRODUCT_COMMIT" \
  --product-tree "$PRODUCT_TREE"

# This is an engineering diagnostic only.  Keep the promotion boundary
# explicit even if all local integrity and admission checks are healthy.
jq -e '
  .status == "blocked" and
  .promotion_eligible == false and
  .gate.status == "blocked" and
  .gate.cp_ai_realistic_pod_passed == false
' reports/gates/T4-CARDS/ADMISSION.json >/dev/null

echo "BLOCKED gate_T4_cards.sh (diagnostic only; CP-AI-REALISTIC-POD remains unpassed)"
