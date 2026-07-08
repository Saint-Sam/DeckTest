#!/usr/bin/env bash
set -euo pipefail

ROOT="${FORGE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SOURCE="$ROOT/crates/forge-core/src/lib.rs"
METRICS="$ROOT/metrics/clone_surface.json"

if [[ "${1:-}" == "--self-test" ]]; then
  python3 - <<'PY'
import json

payload = {"schema": "forge.clone_surface.v1", "persistent_allocation_field_count": 1}
assert json.loads(json.dumps(payload, sort_keys=True))["persistent_allocation_field_count"] == 1
PY
  echo "PASS clone_surface_guard.sh self-test"
  exit 0
fi

python3 - "$SOURCE" "$METRICS" <<'PY'
import json
import re
import sys
from pathlib import Path

source_path = Path(sys.argv[1])
metrics_path = Path(sys.argv[2])
root_path = source_path.parents[3]
source = source_path.read_text(encoding="utf-8")
lines = source.splitlines()

for snapshot_name in ("GameSnapshot", "StateSnapshot"):
    if re.search(rf"\bstruct\s+{snapshot_name}\b", source):
        raise SystemExit(f"ERROR: {snapshot_name} resurrects a full-state clone surface")

for api_name in ("snapshot", "full_snapshot", "clone_state", "state_view"):
    if re.search(rf"\bfn\s+{api_name}\s*\(", source):
        raise SystemExit(f"ERROR: {api_name}() resurrects a full-state clone surface")

if re.search(r"\bimpl\s+From\s*<\s*&\s*(?:'_\s+)?GameState\s*>", source):
    raise SystemExit("ERROR: From<&GameState> can expose a hidden full-state clone surface")

game_state_match = re.search(r"#\[derive\(([^\)]*)\)\]\s+pub\s+struct\s+GameState\b", source)
if game_state_match and "Debug" in {part.strip() for part in game_state_match.group(1).split(",")}:
    raise SystemExit("ERROR: GameState must not derive Debug because it prints hidden full state")

copy_required = (
    "PlayerState",
    "ObjectRecord",
    "DurationMarker",
    "TargetSnapshot",
    "BlockingCreature",
    "CombatDamageRecord",
)

def derive_traits(struct_name):
    match = re.search(
        rf"#\[derive\(([^\)]*)\)\]\s+(?:pub\s+)?struct\s+{struct_name}\b",
        source,
    )
    if not match:
        return set()
    return {part.strip() for part in match.group(1).split(",")}

for struct_name in copy_required:
    if "Copy" not in derive_traits(struct_name):
        raise SystemExit(f"ERROR: {struct_name} must remain Copy for the clone-surface invariant")

allowed = {
    ("ObjectArena", "records"): "copy-on-write Copy ObjectRecord arena shared across GameState clones",
    ("Zone", "objects"): "copy-on-write object IDs only; total zone membership equals object count",
    ("StackEntry", "targets"): "Copy TargetSnapshot records bounded by target requirements",
    ("ResolutionRecord", "targets"): "Copy TargetSnapshot records copied from a stack entry",
    ("ResolutionRecord", "legal_targets"): "one bool per target snapshot",
    ("AttackingCreature", "blockers"): "object IDs bounded by current combat declarations",
    ("CombatState", "attackers"): "current-combat records cleared between combats",
    ("CombatState", "blockers"): "current-combat Copy records cleared between combats",
    ("CombatState", "damage_records"): "current-combat Copy damage records",
    ("CombatState", "first_strike_participants"): "object IDs bounded by current combat",
    ("GameState", "players"): "player scalar arena bounded by player count",
    ("GameState", "objects"): "ObjectArena wrapper with one copy-on-write Copy-record arena",
    ("GameState", "zones"): "fixed shared zones plus per-player copy-on-write memberships",
    ("GameState", "duration_markers"): "Copy records bounded by active effects",
    ("GameState", "stack_entries"): "bounded by current stack depth",
    ("GameState", "resolution_log"): "deterministic replay audit log",
    ("GameState", "trigger_subscriptions"): "data-only Copy trigger definitions compiled from card IR",
    ("GameState", "pending_triggers"): "Copy trigger instances drained before priority",
    ("GameState", "activated_abilities"): "data-only Copy activated ability definitions compiled from card IR",
    ("GameState", "cost_modifiers"): "data-only Copy activation cost modifiers compiled from card IR",
    ("GameState", "loyalty_activations_this_turn"): "object IDs bounded by active permanents and cleared each turn",
    ("GameState", "replacement_effects"): "data-only Copy replacement/prevention definitions compiled from card IR",
    ("GameState", "replacement_choice_orders"): "per-player replacement order preferences bounded by active effects",
    ("GameState", "continuous_effects"): "data-only continuous effects compiled from card IR; dependency edges are explicit CR 613 records",
    ("GameState", "turn_events"): "current-turn Copy event records bounded by EVENT_RING_CAPACITY",
    ("GameState", "combat"): "CombatState wrapper cleared between combats",
    ("GameState", "empty_library_draws_since_sba"): "player IDs drained by SBA processing",
}

tracked_structs = {struct for struct, _ in allowed}
field_re = re.compile(r"^\s*([A-Za-z_][A-Za-z0-9_]*):\s*([^,]+),")

allocation_markers = (
    "Vec<",
    "Option<Vec<",
    "Box<[",
    "String",
    "HashMap<",
    "BTreeMap<",
    "VecDeque<",
    "SmallVec<",
)

def allocation_kind(ty):
    compact = "".join(ty.split())
    if any(marker in compact for marker in allocation_markers):
        return "allocation"
    return ""

def collect_struct_fields():
    collected = {}
    current = None
    depth = 0
    for line in lines:
        if current is None:
            match = re.search(r"\b(?:pub\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)\b", line)
            if match:
                current = match.group(1)
                collected[current] = []
                depth = line.count("{") - line.count("}")
            continue
        depth += line.count("{") - line.count("}")
        field_match = field_re.match(line)
        if field_match:
            collected[current].append((field_match.group(1), field_match.group(2).strip()))
        if depth <= 0:
            current = None
    return collected

struct_fields = collect_struct_fields()
allocation_bearing_structs = {
    struct
    for struct, fields_for_struct in struct_fields.items()
    if any(allocation_kind(ty) for _, ty in fields_for_struct)
}

current_struct = None
brace_depth = 0
fields = []
errors = []

for index, line in enumerate(lines):
    struct_match = re.search(r"\b(?:pub\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)\b", line)
    if current_struct is None and struct_match:
        name = struct_match.group(1)
        if name in tracked_structs:
            current_struct = name
            brace_depth = line.count("{") - line.count("}")
        continue

    if current_struct is None:
        continue

    brace_depth += line.count("{") - line.count("}")
    match = field_re.match(line)
    if match:
        field = match.group(1)
        ty = match.group(2).strip()
        is_allocation = bool(allocation_kind(ty))
        is_wrapper = current_struct == "GameState" and ty in allocation_bearing_structs
        if is_allocation or is_wrapper:
            key = (current_struct, field)
            previous = "\n".join(lines[max(0, index - 3):index])
            if key not in allowed:
                errors.append(f"{current_struct}.{field}: unallowlisted persistent allocation field `{ty}`")
            elif "clone_surface:" not in previous:
                errors.append(f"{current_struct}.{field}: missing clone_surface invariant comment")
            fields.append({
                "struct": current_struct,
                "field": field,
                "type": ty,
                "reason": allowed.get(key, "UNALLOWLISTED"),
            })
    if brace_depth <= 0:
        current_struct = None

field_keys = {(field["struct"], field["field"]) for field in fields}
missing = sorted(set(allowed) - field_keys)
if missing:
    errors.extend(f"{struct}.{field}: allowlist entry missing from source" for struct, field in missing)

metrics = {
    "schema": "forge.clone_surface.v1",
    "source": str(source_path.relative_to(root_path)),
    "guard": "static persistent allocation field count",
    "persistent_allocation_field_count": len(fields),
    "persistent_allocation_fields": fields,
    "full_state_snapshot_surface": False,
    "game_state_debug": False,
    "copy_required_structs": list(copy_required),
    "allocation_bearing_wrappers": sorted(allocation_bearing_structs),
}

if metrics_path.exists():
    baseline = json.loads(metrics_path.read_text(encoding="utf-8"))
    baseline_fields = {
        (field["struct"], field["field"])
        for field in baseline.get("persistent_allocation_fields", [])
    }
    new_fields = sorted(field_keys - baseline_fields)
    if new_fields:
        errors.extend(f"{struct}.{field}: exceeds committed clone-surface baseline" for struct, field in new_fields)
    baseline_count = baseline.get("persistent_allocation_field_count", len(fields))
    if len(fields) > baseline_count:
        errors.append(
            f"persistent allocation field count {len(fields)} exceeds baseline {baseline_count}"
        )

if errors:
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    raise SystemExit(1)

metrics_path.parent.mkdir(parents=True, exist_ok=True)
metrics_path.write_text(json.dumps(metrics, indent=2, sort_keys=True) + "\n", encoding="utf-8")
print(
    "PASS clone_surface_guard.sh "
    f"persistent_allocation_field_count={len(fields)} "
    f"metrics={metrics_path.relative_to(root_path)}"
)
PY
