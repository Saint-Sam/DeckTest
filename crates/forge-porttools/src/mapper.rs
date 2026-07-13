//! Fail-closed legacy ability API mapping into typed Forge operations.

use crate::legacy::{
    collect_scripts, git_revision, parse_legacy_script, LegacyAbilityPrefix, LegacyExpression,
    LegacyLineKind,
};
use forge_carddef::{Expression, Operation};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

/// One legacy ability lowered into typed Forge expressions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MappedLegacyAbility {
    /// Legacy line family.
    pub prefix: LegacyAbilityPrefix,
    /// Legacy API name.
    pub api: String,
    /// Typed payment costs in source order.
    pub costs: Vec<Expression>,
    /// Typed trigger/replacement event when this is not a direct effect.
    pub event: Option<Expression>,
    /// Typed activation timing restriction when present.
    pub timing: Option<Expression>,
    /// Typed Forge effect or event expression.
    pub expression: Expression,
}

/// Stable fail-closed mapping diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MappingDiagnostic {
    /// Machine-readable quarantine reason.
    pub code: String,
    /// Human-readable mapping explanation.
    pub message: String,
}

pub(crate) struct MappedScriptAbility {
    pub line: usize,
    pub selector: String,
    pub ability: MappedLegacyAbility,
}

pub(crate) struct ScriptMappingFailure {
    pub line: usize,
    pub diagnostic: MappingDiagnostic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScriptBlockerObservation {
    pub line: usize,
    pub source: String,
    pub code: String,
    pub message: String,
    pub linked_root_fanout: usize,
}

/// Per-API mapping coverage row.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ApiCoverageRow {
    /// Legacy line prefix.
    pub prefix: String,
    /// Legacy API name.
    pub api: String,
    /// Uses observed in the pinned corpus.
    pub legacy_uses: usize,
    /// Uses fully lowered by the current mapper.
    pub mapped: usize,
    /// Mapped uses covered by the API's structural test pack.
    pub verified: usize,
    /// Uses sent to reason-coded quarantine.
    pub quarantined: usize,
    /// Quarantined uses grouped by stable reason code.
    pub quarantine_reasons: BTreeMap<String, usize>,
}

/// Full-corpus API mapping metrics.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ApiCoverageReport {
    /// Metrics schema version.
    pub schema_version: u32,
    /// Audited cards root.
    pub source_root: String,
    /// Exact vendored legacy revision.
    pub source_revision: String,
    /// Total top-level legacy ability uses.
    pub legacy_uses: usize,
    /// Fully lowered ability uses.
    pub mapped_uses: usize,
    /// Structurally verified mapped uses.
    pub verified_uses: usize,
    /// Reason-coded quarantined uses.
    pub quarantined_uses: usize,
    /// Percentage of uses fully lowered.
    pub mapped_percent: f64,
    /// Coverage rows in descending legacy-frequency order.
    pub apis: Vec<ApiCoverageRow>,
    /// Quarantine counts by stable reason code.
    pub quarantine_reason_counts: BTreeMap<String, usize>,
}

/// One sampled quarantined mapping occurrence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QuarantineSample {
    /// Source path relative to the cards root.
    pub path: String,
    /// One-based source line.
    pub line: usize,
    /// Legacy line prefix.
    pub prefix: String,
    /// Legacy API, when recoverable.
    pub api: String,
    /// Stable quarantine reason.
    pub diagnostic: MappingDiagnostic,
}

#[derive(Serialize)]
struct QuarantineReport<'a> {
    schema_version: u32,
    source_revision: &'a str,
    total_quarantined: usize,
    reason_counts: &'a BTreeMap<String, usize>,
    samples: &'a [QuarantineSample],
}

#[derive(Default)]
struct MutableCoverage {
    legacy_uses: usize,
    mapped: usize,
    verified: usize,
    quarantine_reasons: BTreeMap<String, usize>,
}

type MapperFn = fn(
    LegacyAbilityPrefix,
    &str,
    &str,
    &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic>;

struct MapperSpec {
    prefix: LegacyAbilityPrefix,
    api: &'static str,
    mapper: MapperFn,
}

const MAPPERS: &[MapperSpec] = &[
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Mana",
        mapper: map_mana,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "ManaReflected",
        mapper: map_mana_reflected,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "RearrangeTopOfLibrary",
        mapper: map_rearrange_top_of_library,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "MakeCard",
        mapper: map_make_card,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "AlterAttribute",
        mapper: map_alter_attribute,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Amass",
        mapper: map_amass,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Draw",
        mapper: map_draw,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Dig",
        mapper: map_dig,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "DigUntil",
        mapper: map_dig_until,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Seek",
        mapper: map_seek,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "DealDamage",
        mapper: map_damage,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Pump",
        mapper: map_pump,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "PumpAll",
        mapper: map_pump_all,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Debuff",
        mapper: map_debuff,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "GainLife",
        mapper: map_gain_life,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "LoseLife",
        mapper: map_lose_life,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Mill",
        mapper: map_mill,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Tap",
        mapper: map_tap,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Untap",
        mapper: map_untap,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Regenerate",
        mapper: map_regenerate,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Destroy",
        mapper: map_destroy,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "PutCounter",
        mapper: map_put_counter,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "NameCard",
        mapper: map_name_card,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "Continuous",
        mapper: map_continuous,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "ChangeZone",
        mapper: map_change_zone,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Token",
        mapper: map_token,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "DestroyAll",
        mapper: map_destroy_all,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "DamageAll",
        mapper: map_damage_all,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Discard",
        mapper: map_discard,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Counter",
        mapper: map_counter_spell,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Scry",
        mapper: map_scry,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Surveil",
        mapper: map_surveil,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "ReduceCost",
        mapper: map_reduce_cost,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "RaiseCost",
        mapper: map_raise_cost,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "CantBlockBy",
        mapper: map_cant_block_by,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "ChangeZoneAll",
        mapper: map_change_zone_all,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Effect",
        mapper: map_effect,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Animate",
        mapper: map_animate,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "RollDice",
        mapper: map_roll_dice,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "PeekAndReveal",
        mapper: map_peek_and_reveal,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "SetState",
        mapper: map_set_state,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "AlternativeCost",
        mapper: map_alternative_cost,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Sacrifice",
        mapper: map_sacrifice_effect,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "SacrificeAll",
        mapper: map_sacrifice_all,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "GainControl",
        mapper: map_gain_control,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "PreventDamage",
        mapper: map_prevent_damage,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Protection",
        mapper: map_protection,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "ChooseType",
        mapper: map_choose_type,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "ChooseColor",
        mapper: map_choose_color,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Fog",
        mapper: map_fog,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Fight",
        mapper: map_fight,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Explore",
        mapper: map_explore_or_connive,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Connive",
        mapper: map_explore_or_connive,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "PutCounterAll",
        mapper: map_put_counter_all,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "CopySpellAbility",
        mapper: map_copy_spell_ability,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "CopyPermanent",
        mapper: map_copy_permanent,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Clone",
        mapper: map_clone,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "ChooseCard",
        mapper: map_choose_card,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "ChoosePlayer",
        mapper: map_choose_player,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Play",
        mapper: map_play,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "AddTurn",
        mapper: map_add_turn,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "UntapAll",
        mapper: map_untap_all,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "TapAll",
        mapper: map_tap_all,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "TapOrUntap",
        mapper: map_tap_or_untap,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "RemoveCounter",
        mapper: map_remove_counter,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Proliferate",
        mapper: map_proliferate,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "CantAttack",
        mapper: map_cant_attack_or_block,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "CantBlock",
        mapper: map_cant_attack_or_block,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "CantAttack,CantBlock",
        mapper: map_cant_attack_or_block,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "MustAttack",
        mapper: map_must_attack,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "MinMaxBlocker",
        mapper: map_min_max_blocker,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "CastWithFlash",
        mapper: map_cast_with_flash,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Static,
        api: "CantBeCast",
        mapper: map_cant_be_cast,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Shuffle",
        mapper: map_shuffle,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "SetLife",
        mapper: map_set_life,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Venture",
        mapper: map_owner_marker_effect,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "BecomeMonarch",
        mapper: map_owner_marker_effect,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "TakeInitiative",
        mapper: map_owner_marker_effect,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Investigate",
        mapper: map_investigate,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Attach",
        mapper: map_attach,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "RevealHand",
        mapper: map_reveal_hand,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "AnimateAll",
        mapper: map_animate_all,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "Cleanup",
        mapper: map_cleanup,
    },
];

#[derive(Clone)]
pub(crate) struct MappingContext<'a> {
    svars: BTreeMap<String, &'a LegacyExpression>,
    duplicate_svars: BTreeSet<String>,
    value_bindings: BTreeMap<String, Expression>,
}

impl<'a> MappingContext<'a> {
    pub(crate) fn from_script(script: &'a crate::legacy::LegacyScript) -> Self {
        let mut svars = BTreeMap::new();
        let mut duplicate_svars = BTreeSet::new();
        for line in &script.lines {
            let LegacyLineKind::SVar { name, expression } = &line.kind else {
                continue;
            };
            if svars.insert(name.clone(), expression).is_some() {
                duplicate_svars.insert(name.clone());
            }
        }
        Self {
            svars,
            duplicate_svars,
            value_bindings: BTreeMap::new(),
        }
    }
}

/// Maps one parsed top-level legacy ability without approximation.
pub fn map_legacy_ability(
    prefix: LegacyAbilityPrefix,
    expression: &LegacyExpression,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    let (unconditioned, presence_condition) = extract_presence_condition(expression)?;
    let empty_context = MappingContext {
        svars: BTreeMap::new(),
        duplicate_svars: BTreeSet::new(),
        value_bindings: BTreeMap::new(),
    };
    let (unconditioned, legacy_condition, check_on_resolution) =
        extract_legacy_conditions(&unconditioned, &empty_context)?;
    let (unconditioned, unless_clause) = extract_unless_clause(&unconditioned)?;
    let condition = combine_conditions(
        [presence_condition, legacy_condition]
            .into_iter()
            .flatten()
            .collect(),
    );
    if !check_on_resolution && condition.is_none() {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "NoResolvingCheck requires a closed trigger condition",
        ));
    }
    let expression = &unconditioned;
    let Some(selector) = expression.fields.first() else {
        return Err(diagnostic("MALFORMED_API", "ability has no API selector"));
    };
    let Some(selector_key) = selector.key.as_deref() else {
        return Err(diagnostic(
            "MALFORMED_API",
            "first ability field has no selector key",
        ));
    };
    if !matches!(selector_key, "AB" | "SP" | "DB" | "Mode" | "Event" | "ST") {
        return Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            &format!("unsupported API selector `{selector_key}`"),
        ));
    }
    let api = selector.value.trim();
    if api.is_empty() {
        return Err(diagnostic("MALFORMED_API", "ability API name is empty"));
    }
    let optional_effect = if prefix == LegacyAbilityPrefix::Activated && api != "Dig" {
        let optional = expression
            .fields
            .iter()
            .find(|field| field.key.as_deref() == Some("Optional"))
            .map(|field| field.value.as_str());
        let decider = expression
            .fields
            .iter()
            .find(|field| field.key.as_deref() == Some("OptionalDecider"))
            .map(|field| field.value.as_str());
        match (optional, decider) {
            (None, None) => false,
            (Some("True" | "You"), None) | (None, Some("You")) => true,
            (Some(value), None) => return Err(unsupported_value("Optional", value)),
            (None, Some(value)) => return Err(unsupported_value("OptionalDecider", value)),
            (Some(_), Some(_)) => {
                return Err(diagnostic(
                    "UNSUPPORTED_PARAMETER",
                    "Optional and OptionalDecider cannot be combined",
                ));
            }
        }
    } else {
        false
    };
    let secondary = expression
        .fields
        .iter()
        .find(|field| field.key.as_deref() == Some("Secondary"))
        .map(|field| field.value.as_str());
    if secondary.is_some_and(|value| value != "True") {
        return Err(unsupported_value(
            "Secondary",
            secondary.unwrap_or_default(),
        ));
    }
    let has_ability_name = expression
        .fields
        .iter()
        .any(|field| field.key.as_deref() == Some("Name"))
        && api != "MakeCard";
    let stripped_expression =
        (optional_effect || secondary.is_some() || has_ability_name).then(|| {
            let mut stripped = expression.clone();
            stripped.fields.retain(|field| {
                !matches!(
                    field.key.as_deref(),
                    Some("Optional" | "OptionalDecider" | "Secondary")
                ) && (!has_ability_name || field.key.as_deref() != Some("Name"))
            });
            stripped
        });
    let expression = stripped_expression.as_ref().unwrap_or(expression);
    let mut parameters = parameters(expression)?;
    let target_range = extract_target_range(&mut parameters)?;
    normalize_legacy_defaults(&mut parameters);
    let timing = extract_legacy_timing(&mut parameters)?;
    let Some(spec) = MAPPERS
        .iter()
        .find(|spec| spec.prefix == prefix && spec.api == api)
    else {
        return Err(diagnostic(
            "UNMAPPED_API",
            &format!("no mapper is registered for {}:{api}", prefix.as_str()),
        ));
    };
    let mut mapped = (spec.mapper)(prefix, api, selector_key, &parameters)?;
    if let Some((minimum, maximum)) = target_range {
        let replaced = apply_target_range(&mut mapped.expression, minimum, maximum)?;
        if replaced == 0 {
            return Err(diagnostic(
                "TARGET_RANGE_MISMATCH",
                "target cardinality did not produce a typed target selector",
            ));
        }
    }
    mapped.timing = timing;
    if optional_effect {
        mapped.expression = call(
            Operation::ChooseUpTo,
            vec![Expression::Integer(1), mapped.expression],
        );
    }
    if let Some(unless_clause) = unless_clause {
        mapped.expression = apply_unless_clause(mapped.expression, unless_clause);
    }
    if let Some(condition) = condition {
        mapped =
            apply_legacy_condition(prefix, selector_key, mapped, condition, check_on_resolution)?;
    }
    Ok(mapped)
}

fn map_legacy_ability_in_context(
    prefix: LegacyAbilityPrefix,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    map_with_context(prefix, expression, context, &mut Vec::new())
}

pub(crate) fn map_named_svar_ability(
    script: &crate::legacy::LegacyScript,
    name: &str,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    let context = MappingContext::from_script(script);
    let expression = context.svars.get(name).copied().ok_or_else(|| {
        diagnostic(
            "MISSING_SVAR",
            &format!("referenced SVar `{name}` is not declared"),
        )
    })?;
    if expression
        .fields
        .first()
        .and_then(|field| field.key.as_deref())
        != Some("DB")
    {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("SVar `{name}` is not a DB effect"),
        ));
    }
    resolve_svar(name, &context, &mut Vec::new())
}

pub(crate) fn map_script_abilities(
    script: &crate::legacy::LegacyScript,
) -> Result<Vec<MappedScriptAbility>, ScriptMappingFailure> {
    let context = MappingContext::from_script(script);
    let mut mapped = Vec::new();
    for line in &script.lines {
        let LegacyLineKind::Ability { prefix, expression } = &line.kind else {
            continue;
        };
        let selector = expression
            .fields
            .first()
            .and_then(|field| field.key.clone())
            .ok_or_else(|| ScriptMappingFailure {
                line: line.line,
                diagnostic: diagnostic("MALFORMED_API", "ability has no typed selector"),
            })?;
        let ability =
            map_legacy_ability_in_context(*prefix, expression, &context).map_err(|diagnostic| {
                ScriptMappingFailure {
                    line: line.line,
                    diagnostic,
                }
            })?;
        mapped.push(MappedScriptAbility {
            line: line.line,
            selector,
            ability,
        });
    }
    Ok(mapped)
}

pub(crate) fn collect_script_mapping_blockers(
    script: &crate::legacy::LegacyScript,
) -> Vec<ScriptBlockerObservation> {
    let context = MappingContext::from_script(script);
    let svar_names = context.svars.keys().cloned().collect::<BTreeSet<_>>();
    let mut svar_lines = BTreeMap::new();
    let mut svar_edges = BTreeMap::new();
    for line in &script.lines {
        let LegacyLineKind::SVar { name, expression } = &line.kind else {
            continue;
        };
        svar_lines.entry(name.clone()).or_insert(line.line);
        svar_edges.insert(
            name.clone(),
            expression_svar_references(expression, &svar_names),
        );
    }

    let mut blockers = Vec::new();
    let mut fanout = BTreeMap::<String, usize>::new();
    for line in &script.lines {
        let LegacyLineKind::Ability { prefix, expression } = &line.kind else {
            continue;
        };
        let roots = expression_svar_references(expression, &svar_names);
        let reachable = reachable_svars(&roots, &svar_edges);
        for name in reachable {
            *fanout.entry(name).or_insert(0) += 1;
        }
        for diagnostic in collect_mapping_diagnostics(*prefix, expression, &context, &[]) {
            blockers.push(ScriptBlockerObservation {
                line: line.line,
                source: format!("root:{}", prefix.as_str()),
                code: diagnostic.code,
                message: diagnostic.message,
                linked_root_fanout: 1,
            });
        }
    }

    for (name, linked_root_fanout) in fanout {
        let line = svar_lines.get(&name).copied().unwrap_or(1);
        if context.duplicate_svars.contains(&name) {
            blockers.push(ScriptBlockerObservation {
                line,
                source: format!("svar:{name}"),
                code: "DUPLICATE_SVAR".to_string(),
                message: format!("SVar `{name}` is declared more than once"),
                linked_root_fanout,
            });
            continue;
        }
        let Some(expression) = context.svars.get(&name).copied() else {
            continue;
        };
        let Some(prefix) = linked_ability_prefix(expression) else {
            continue;
        };
        for diagnostic in
            collect_mapping_diagnostics(prefix, expression, &context, std::slice::from_ref(&name))
        {
            blockers.push(ScriptBlockerObservation {
                line,
                source: format!("svar:{name}"),
                code: diagnostic.code,
                message: diagnostic.message,
                linked_root_fanout,
            });
        }
    }

    blockers.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
    blockers
}

fn collect_mapping_diagnostics(
    prefix: LegacyAbilityPrefix,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    initial_stack: &[String],
) -> Vec<MappingDiagnostic> {
    let mut candidate = expression.clone();
    let mut removed_parameters = BTreeSet::new();
    let mut diagnostics = Vec::new();
    loop {
        let mut stack = initial_stack.to_vec();
        let Err(diagnostic) = map_with_context(prefix, &candidate, context, &mut stack) else {
            break;
        };
        let removable = unsupported_parameter_key(&diagnostic)
            .filter(|key| removed_parameters.insert(key.clone()));
        diagnostics.push(diagnostic);
        let Some(key) = removable else {
            break;
        };
        let before = candidate.fields.len();
        candidate
            .fields
            .retain(|field| field.key.as_deref() != Some(key.as_str()));
        if candidate.fields.len() == before {
            break;
        }
    }
    diagnostics
}

fn unsupported_parameter_key(diagnostic: &MappingDiagnostic) -> Option<String> {
    if diagnostic.code != "UNSUPPORTED_PARAMETER" {
        return None;
    }
    diagnostic.message.split('`').nth(1).map(str::to_string)
}

fn linked_ability_prefix(expression: &LegacyExpression) -> Option<LegacyAbilityPrefix> {
    match expression.fields.first()?.key.as_deref()? {
        "Mode" | "ST" => Some(LegacyAbilityPrefix::Static),
        "Event" => Some(LegacyAbilityPrefix::Replacement),
        "AB" | "SP" | "DB" => Some(LegacyAbilityPrefix::Activated),
        _ => None,
    }
}

fn expression_svar_references(
    expression: &LegacyExpression,
    svar_names: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut references = BTreeSet::new();
    for field in &expression.fields {
        let key = field.key.as_deref().unwrap_or_default();
        if key.contains("Description")
            || key.contains("Prompt")
            || matches!(key, "AILogic" | "AIHint" | "PrecostDesc")
        {
            continue;
        }
        for token in field
            .value
            .split(|character: char| {
                character == ',' || character == '&' || character.is_whitespace()
            })
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            if svar_names.contains(token) {
                references.insert(token.to_string());
            }
        }
    }
    references
}

fn reachable_svars(
    roots: &BTreeSet<String>,
    edges: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let mut reachable = BTreeSet::new();
    let mut pending = roots.iter().cloned().collect::<Vec<_>>();
    while let Some(name) = pending.pop() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        if let Some(children) = edges.get(&name) {
            pending.extend(children.iter().cloned());
        }
    }
    reachable
}

fn map_with_context(
    prefix: LegacyAbilityPrefix,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    let (unconditioned, presence_condition) = extract_presence_condition(expression)?;
    let (unconditioned, legacy_condition, check_on_resolution) =
        extract_legacy_conditions(&unconditioned, context)?;
    let has_switched_unless = unconditioned
        .fields
        .iter()
        .any(|field| field.key.as_deref() == Some("UnlessSwitched"));
    let (unconditioned, unless_clause) = if has_switched_unless {
        extract_unless_clause(&unconditioned)?
    } else {
        (unconditioned, None)
    };
    let condition = combine_conditions(
        [presence_condition, legacy_condition]
            .into_iter()
            .flatten()
            .collect(),
    );
    if !check_on_resolution && condition.is_none() {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "NoResolvingCheck requires a closed trigger condition",
        ));
    }
    let mut mapped = map_with_context_unconditioned(
        prefix,
        &unconditioned,
        context,
        stack,
        condition,
        check_on_resolution,
    )?;
    if let Some(unless_clause) = unless_clause {
        mapped.expression = apply_unless_clause(mapped.expression, unless_clause);
    }
    Ok(mapped)
}

fn map_with_context_unconditioned(
    prefix: LegacyAbilityPrefix,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
    condition: Option<Expression>,
    check_on_resolution: bool,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    let selector = expression
        .fields
        .first()
        .ok_or_else(|| diagnostic("MALFORMED_API", "ability has no API selector"))?;
    let selector_key = selector
        .key
        .as_deref()
        .ok_or_else(|| diagnostic("MALFORMED_API", "first ability field has no selector key"))?;
    let api = selector.value.trim();
    if prefix == LegacyAbilityPrefix::Activated && api == "Charm" {
        let mapped = map_charm_ability(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated && api == "GenericChoice" {
        let mapped = map_generic_choice(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated && api == "ReplaceEffect" {
        let mapped = map_replace_effect(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated && api == "Branch" {
        let mapped = map_branch(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated && api == "FlipCoin" {
        let mapped = map_flip_coin(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated && api == "ImmediateTrigger" {
        let mapped = map_immediate_trigger(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated && api == "DelayedTrigger" {
        let mapped = map_delayed_trigger(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated && api == "RepeatEach" {
        let mapped = map_repeat_each(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated
        && api == "RollDice"
        && parameters(expression)?.contains_key("ResultSubAbilities")
    {
        let mapped = map_roll_dice_table(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated
        && api == "RollDice"
        && parameters(expression)?.contains_key("SubAbility")
        && parameters(expression)?
            .keys()
            .any(|key| matches!(key.as_str(), "ResultSVar" | "ChosenSVar" | "OtherSVar"))
    {
        let mapped = map_roll_dice_with_result(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated
        && api == "Effect"
        && parameters(expression)?
            .keys()
            .any(|key| matches!(key.as_str(), "Triggers" | "ReplacementEffects"))
    {
        let mapped = map_trigger_effect(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated
        && api == "Effect"
        && parameters(expression)?.contains_key("StaticAbilities")
    {
        let static_name = parameters(expression)?
            .get("StaticAbilities")
            .cloned()
            .unwrap_or_default();
        let linked_names = static_name
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        if !matches!(static_name.as_str(), "Unblockable" | "MustAttack")
            && !linked_names.is_empty()
            && linked_names
                .iter()
                .all(|name| context.svars.contains_key(*name))
        {
            let mapped =
                map_linked_static_effect(prefix, selector_key, expression, context, stack)?;
            return apply_optional_legacy_condition(
                prefix,
                selector_key,
                mapped,
                condition,
                check_on_resolution,
            );
        }
    }
    if prefix == LegacyAbilityPrefix::Static
        && api == "Continuous"
        && parameters(expression)?.keys().any(|key| {
            matches!(
                key.as_str(),
                "AddAbility"
                    | "AddTrigger"
                    | "AddStaticAbility"
                    | "AddReplacementEffect"
                    | "AddSVar"
            )
        })
    {
        let mapped =
            map_linked_continuous_traits(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Activated
        && matches!(api, "Animate" | "AnimateAll")
        && parameters(expression)?.keys().any(|key| {
            matches!(
                key.as_str(),
                "Triggers" | "staticAbilities" | "StaticAbilities"
            )
        })
    {
        let mapped =
            map_animated_linked_traits(prefix, api, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Replacement && api == "Moved" {
        let mapped = map_moved_replacement(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Replacement && api == "DamageDone" {
        let mapped = map_damage_replacement(prefix, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Replacement && api == "Untap" {
        let mapped = map_untap_replacement(prefix, selector_key, expression)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Replacement && api == "Counter" {
        let mapped = map_counter_replacement(prefix, selector_key, expression)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Static && selector_key == "Mode" && is_mapped_trigger_api(api)
    {
        let mapped = map_triggered_ability(
            LegacyAbilityPrefix::Triggered,
            api,
            selector_key,
            expression,
            context,
            stack,
        )?;
        return apply_optional_legacy_condition(
            LegacyAbilityPrefix::Triggered,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }
    if prefix == LegacyAbilityPrefix::Triggered {
        let mapped = map_triggered_ability(prefix, api, selector_key, expression, context, stack)?;
        return apply_optional_legacy_condition(
            prefix,
            selector_key,
            mapped,
            condition,
            check_on_resolution,
        );
    }

    let parameter_map = parameters(expression)?;
    let sub_ability = parameter_map.get("SubAbility").cloned();
    let remember_targets = match parameter_map.get("RememberTargets").map(String::as_str) {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("RememberTargets", value)),
    };
    let target_unique = closed_true_flag(&parameter_map, "TargetUnique")?;
    let target_group = match (
        parameter_map.get("TargetsWithSameController"),
        parameter_map.get("TargetsWithDefinedController"),
    ) {
        (None, None) => None,
        (Some(value), None) if value == "True" => Some("same_controller".to_string()),
        (Some(value), None) => return Err(unsupported_value("TargetsWithSameController", value)),
        (None, Some(value))
            if matches!(
                value.as_str(),
                "You" | "Opponent" | "TargetedController" | "Remembered"
            ) =>
        {
            Some(format!("defined_controller:{}", value.to_ascii_lowercase()))
        }
        (None, Some(value)) => {
            return Err(unsupported_value("TargetsWithDefinedController", value));
        }
        (Some(_), Some(_)) => {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "target controller grouping parameters cannot be combined",
            ));
        }
    };
    let reduce_cost = parameter_map
        .get("ReduceCost")
        .map(|value| {
            if prefix != LegacyAbilityPrefix::Activated {
                return Err(unsupported_value("ReduceCost", value));
            }
            value
                .parse::<i64>()
                .ok()
                .filter(|amount| *amount > 0)
                .map(Expression::Integer)
                .map_or_else(|| resolve_value_svar(value, context), Ok)
        })
        .transpose()?;
    let etb_effect = match parameter_map.get("ETB").map(String::as_str) {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("ETB", value)),
    };
    let power_up = if prefix == LegacyAbilityPrefix::Activated {
        closed_true_flag(&parameter_map, "PowerUp")?
    } else {
        false
    };
    let exhaust = if prefix == LegacyAbilityPrefix::Activated {
        closed_true_flag(&parameter_map, "Exhaust")?
    } else {
        false
    };
    let reorder_moved = match parameter_map.get("Reorder").map(String::as_str) {
        None => false,
        Some("True")
            if prefix == LegacyAbilityPrefix::Activated
                && api == "ChangeZone"
                && parameter_map
                    .get("Destination")
                    .is_some_and(|zone| zone == "Library") =>
        {
            true
        }
        Some(value) => return Err(unsupported_value("Reorder", value)),
    };
    let repeat_effect = if prefix == LegacyAbilityPrefix::Activated && api == "CopySpellAbility" {
        parameter_map
            .get("Amount")
            .map(|value| {
                value
                    .parse::<i64>()
                    .ok()
                    .filter(|amount| *amount > 0)
                    .map(Expression::Integer)
                    .map_or_else(|| resolve_value_svar(value, context), Ok)
            })
            .transpose()?
    } else {
        None
    };
    let divided_allocation = parameter_map
        .get("DividedAsYouChoose")
        .map(|allocation| {
            let (amount_key, operation, target_index, amount_index) = match api {
                "DealDamage" => ("NumDmg", Operation::DealDamage, 0, 1),
                "PutCounter" => ("CounterNum", Operation::AddCounter, 0, 2),
                "PreventDamage" => ("Amount", Operation::PreventDamage, 1, 2),
                _ => return Err(unsupported_value("DividedAsYouChoose", allocation)),
            };
            let amount = parameter_map.get(amount_key).ok_or_else(|| {
                diagnostic(
                    "MISSING_PARAMETER",
                    &format!("DividedAsYouChoose requires {amount_key}"),
                )
            })?;
            if allocation != amount {
                return Err(diagnostic(
                    "UNSUPPORTED_VALUE",
                    "DividedAsYouChoose must exactly match the effect amount",
                ));
            }
            Ok((operation, target_index, amount_index))
        })
        .transpose()?;
    let optional_effect = if prefix == LegacyAbilityPrefix::Activated && api != "Dig" {
        match (
            parameter_map.get("Optional").map(String::as_str),
            parameter_map.get("OptionalDecider").map(String::as_str),
        ) {
            (None, None) => false,
            (Some("True" | "You"), None) | (None, Some("You")) => true,
            (Some(value), None) => return Err(unsupported_value("Optional", value)),
            (None, Some(value)) => return Err(unsupported_value("OptionalDecider", value)),
            (Some(_), Some(_)) => {
                return Err(diagnostic(
                    "UNSUPPORTED_PARAMETER",
                    "Optional and OptionalDecider cannot be combined",
                ));
            }
        }
    } else {
        false
    };
    let secondary = parameter_map.get("Secondary").map(String::as_str);
    if secondary.is_some_and(|value| value != "True") {
        return Err(unsupported_value(
            "Secondary",
            secondary.unwrap_or_default(),
        ));
    }
    let has_ability_name = parameter_map.contains_key("Name") && api != "MakeCard";
    let mut base_expression = expression.clone();
    if sub_ability.is_some()
        || remember_targets
        || target_unique
        || target_group.is_some()
        || reduce_cost.is_some()
        || etb_effect
        || power_up
        || exhaust
        || reorder_moved
        || repeat_effect.is_some()
        || optional_effect
        || divided_allocation.is_some()
        || secondary.is_some()
        || has_ability_name
    {
        base_expression.fields.retain(|field| {
            field.key.as_deref() != Some("SubAbility")
                && field.key.as_deref() != Some("RememberTargets")
                && field.key.as_deref() != Some("TargetUnique")
                && field.key.as_deref() != Some("TargetsWithSameController")
                && field.key.as_deref() != Some("TargetsWithDefinedController")
                && field.key.as_deref() != Some("ReduceCost")
                && field.key.as_deref() != Some("ETB")
                && (!power_up || field.key.as_deref() != Some("PowerUp"))
                && (!exhaust || field.key.as_deref() != Some("Exhaust"))
                && (!reorder_moved || field.key.as_deref() != Some("Reorder"))
                && (repeat_effect.is_none() || field.key.as_deref() != Some("Amount"))
                && (!optional_effect || field.key.as_deref() != Some("Optional"))
                && (!optional_effect || field.key.as_deref() != Some("OptionalDecider"))
                && (divided_allocation.is_none()
                    || field.key.as_deref() != Some("DividedAsYouChoose"))
                && field.key.as_deref() != Some("Secondary")
                && (!has_ability_name || field.key.as_deref() != Some("Name"))
        });
    }
    let dynamic_mana_limit = if base_expression
        .fields
        .iter()
        .any(|field| field.value.contains("cmcLEX"))
    {
        let limit = if context.svars.contains_key("X")
            || context.duplicate_svars.contains("X")
            || context.value_bindings.contains_key("X")
        {
            resolve_value_svar("X", context)?
        } else if parameter_map
            .get("Cost")
            .is_some_and(|cost| cost.split_whitespace().any(|symbol| symbol == "X"))
            || parameter_map
                .get("Announce")
                .is_some_and(|value| value == "X")
        {
            call(Operation::PaidX, vec![])
        } else {
            return Err(diagnostic(
                "MISSING_SVAR",
                "cmcLEX requires a typed X binding or an announced/paid X",
            ));
        };
        for field in &mut base_expression.fields {
            field.value = field.value.replace("cmcLEX", "cmcLE0");
        }
        Some(limit)
    } else {
        None
    };
    let mut mapped = match map_dynamic_ability(prefix, &base_expression, context)? {
        Some(mapped) => mapped,
        None => map_legacy_ability(prefix, &base_expression)?,
    };
    if let Some(limit) = dynamic_mana_limit {
        let exclusive = call(Operation::AddValue, vec![limit, Expression::Integer(1)]);
        let replaced = replace_dynamic_mana_limit(&mut mapped.expression, &exclusive);
        if replaced == 0 {
            return Err(diagnostic(
                "DYNAMIC_LOWERING_MISMATCH",
                "cmcLEX did not produce a typed mana-value predicate",
            ));
        }
    }
    if target_unique || target_group.is_some() {
        let replaced = wrap_target_selectors(
            &mut mapped.expression,
            target_unique,
            target_group.as_deref(),
        );
        if replaced == 0 {
            return Err(diagnostic(
                "TARGET_BINDING_MISMATCH",
                "target grouping did not produce a typed target selector",
            ));
        }
    }
    if let Some(reduction) = reduce_cost {
        mapped
            .costs
            .push(call(Operation::ReduceCostBy, vec![reduction]));
    }
    if power_up {
        mapped.costs.push(call(Operation::PowerUpCost, vec![]));
    }
    if exhaust {
        mapped.costs.push(call(Operation::ExhaustCost, vec![]));
    }
    if remember_targets {
        mapped.expression = sequence(
            call(
                Operation::Remember,
                vec![call(Operation::Target, vec![call(Operation::Any, vec![])])],
            ),
            mapped.expression,
        );
    }
    if optional_effect {
        mapped.expression = call(
            Operation::ChooseUpTo,
            vec![Expression::Integer(1), mapped.expression],
        );
    }
    if let Some((operation, target_index, amount_index)) = divided_allocation {
        let applied = apply_target_allocation(
            &mut mapped.expression,
            operation,
            target_index,
            amount_index,
        )?;
        if applied != 1 {
            return Err(diagnostic(
                "TARGET_ALLOCATION_MISMATCH",
                "divided allocation did not produce exactly one typed target declaration",
            ));
        }
    }
    mapped = apply_optional_legacy_condition(
        prefix,
        selector_key,
        mapped,
        condition,
        check_on_resolution,
    )?;
    if let Some(name) = sub_ability {
        let linked = resolve_svar(&name, context, stack)?;
        if linked.event.is_some() || !linked.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{name}` is not a cost-free effect chain"),
            ));
        }
        mapped.expression = sequence(mapped.expression, linked.expression);
    }
    if etb_effect {
        mapped.expression = call(Operation::EtbEffect, vec![mapped.expression]);
    }
    if reorder_moved {
        mapped.expression = call(Operation::ReorderMoved, vec![mapped.expression]);
    }
    if let Some(amount) = repeat_effect {
        mapped.expression = call(Operation::RepeatEffect, vec![amount, mapped.expression]);
    }
    if prefix == LegacyAbilityPrefix::Static
        && parameter_map
            .get("EffectZone")
            .is_some_and(|zone| zone == "Command")
    {
        mapped.expression = call(
            Operation::ActiveInZone,
            vec![mapped.expression, Expression::Text("command".to_string())],
        );
    }
    Ok(mapped)
}

fn apply_optional_legacy_condition(
    prefix: LegacyAbilityPrefix,
    selector_key: &str,
    mapped: MappedLegacyAbility,
    condition: Option<Expression>,
    check_on_resolution: bool,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    match condition {
        Some(condition) => {
            apply_legacy_condition(prefix, selector_key, mapped, condition, check_on_resolution)
        }
        None => Ok(mapped),
    }
}

fn extract_presence_condition(
    expression: &LegacyExpression,
) -> Result<(LegacyExpression, Option<Expression>), MappingDiagnostic> {
    let has_presence = expression
        .fields
        .iter()
        .any(|field| field.key.as_deref() == Some("IsPresent"));
    let has_condition_present = expression
        .fields
        .iter()
        .any(|field| field.key.as_deref() == Some("ConditionPresent"));
    let has_comparison = expression.fields.iter().any(|field| {
        matches!(
            field.key.as_deref(),
            Some("PresentCompare" | "ConditionPresentCompare" | "ConditionCompare")
        )
    });
    if !has_presence && !has_condition_present && !has_comparison {
        return Ok((expression.clone(), None));
    }
    let parameters = parameters(expression)?;
    let present_key = if has_presence {
        "IsPresent"
    } else {
        "ConditionPresent"
    };
    let present = parameters.get(present_key).ok_or_else(|| {
        diagnostic(
            "MISSING_PARAMETER",
            "presence comparison requires a matching presence selector",
        )
    })?;
    let zone_key = if has_presence {
        "PresentZone"
    } else {
        "ConditionZone"
    };
    if has_condition_present && parameters.contains_key("ConditionDefined") {
        if let Some(zone) = parameters.get(zone_key) {
            if zone != "Battlefield" {
                return Err(unsupported_value(zone_key, zone));
            }
        }
    }
    let selector = match parameters.get(zone_key).map(String::as_str) {
        None | Some("Battlefield") if has_condition_present => {
            if let Some(defined) = parameters.get("ConditionDefined") {
                condition_defined_presence_selector(defined, present)?
            } else {
                presence_selector(present)?
            }
        }
        None | Some("Battlefield") => presence_selector(present)?,
        Some(zone @ ("Graveyard" | "Hand" | "Exile" | "Library")) => {
            card_selector_in_zone(present, &zone.to_ascii_lowercase())?
        }
        Some(zone) => return Err(unsupported_value(zone_key, zone)),
    };
    let comparison = parameters
        .get("PresentCompare")
        .or_else(|| parameters.get("ConditionPresentCompare"))
        .or_else(|| parameters.get("ConditionCompare"))
        .map(String::as_str)
        .unwrap_or("GE1");
    let condition = closed_count_comparison(
        call(Operation::Count, vec![selector]),
        comparison,
        if has_presence {
            "PresentCompare"
        } else {
            "ConditionCompare"
        },
    )?;
    let mut unconditioned = expression.clone();
    unconditioned.fields.retain(|field| {
        !matches!(
            field.key.as_deref(),
            Some(
                "IsPresent"
                    | "PresentCompare"
                    | "PresentZone"
                    | "ConditionPresent"
                    | "ConditionDefined"
                    | "ConditionPresentCompare"
                    | "ConditionCompare"
                    | "ConditionZone"
                    | "ConditionDescription"
            )
        )
    });
    Ok((unconditioned, Some(condition)))
}

fn extract_legacy_conditions(
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
) -> Result<(LegacyExpression, Option<Expression>, bool), MappingDiagnostic> {
    let parameters = parameters(expression)?;
    let check_on_resolution = match parameters.get("NoResolvingCheck").map(String::as_str) {
        None => true,
        Some("True") => false,
        Some(value) => return Err(unsupported_value("NoResolvingCheck", value)),
    };
    let mut conditions = Vec::new();
    if let Some(value) = parameters.get("Condition") {
        conditions.push(legacy_named_condition(value)?);
    }
    let check_svar = parameters
        .get("CheckSVar")
        .map(|value| ("CheckSVar", "SVarCompare", value))
        .or_else(|| {
            parameters
                .get("ConditionCheckSVar")
                .map(|value| ("ConditionCheckSVar", "ConditionSVarCompare", value))
        });
    let comparison = parameters
        .get("SVarCompare")
        .or_else(|| parameters.get("ConditionSVarCompare"));
    match (check_svar, comparison) {
        (Some((check_key, compare_key, value)), comparison) => {
            let subject = resolve_comparison_value(value, check_key, context)?;
            conditions.push(closed_value_comparison(
                subject,
                comparison.map(String::as_str).unwrap_or("GE1"),
                compare_key,
                context,
            )?);
        }
        (None, Some(_)) => {
            return Err(diagnostic(
                "MISSING_PARAMETER",
                "SVarCompare or ConditionSVarCompare requires a matching SVar check",
            ));
        }
        (None, None) => {}
    }
    if let Some(value) = parameters.get("ActivatorThisTurnCast") {
        conditions.push(activator_this_turn_cast_condition(expression, value)?);
    }
    if let Some(value) = parameters.get("OpponentTurn") {
        match value.as_str() {
            "True" => conditions.push(legacy_named_condition("NotPlayerTurn")?),
            _ => return Err(unsupported_value("OpponentTurn", value)),
        }
    }
    if let Some(value) = parameters.get("Revolt") {
        match value.as_str() {
            "True" => conditions.push(call(Operation::RevoltOccurred, vec![])),
            _ => return Err(unsupported_value("Revolt", value)),
        }
    }
    if conditions.is_empty() {
        if !check_on_resolution {
            let mut unconditioned = expression.clone();
            unconditioned
                .fields
                .retain(|field| field.key.as_deref() != Some("NoResolvingCheck"));
            return Ok((unconditioned, None, false));
        }
        return Ok((expression.clone(), None, true));
    }
    let mut unconditioned = expression.clone();
    unconditioned.fields.retain(|field| {
        !matches!(
            field.key.as_deref(),
            Some(
                "Condition"
                    | "CheckSVar"
                    | "SVarCompare"
                    | "ConditionCheckSVar"
                    | "ConditionSVarCompare"
                    | "ActivatorThisTurnCast"
                    | "OpponentTurn"
                    | "Revolt"
                    | "NoResolvingCheck"
                    | "ConditionDescription"
            )
        )
    });
    Ok((
        unconditioned,
        combine_conditions(conditions),
        check_on_resolution,
    ))
}

fn combine_conditions(mut conditions: Vec<Expression>) -> Option<Expression> {
    match conditions.len() {
        0 => None,
        1 => conditions.pop(),
        _ => Some(call(Operation::And, conditions)),
    }
}

struct UnlessClause {
    payer: Expression,
    costs: Vec<Expression>,
    switched: bool,
}

fn extract_unless_clause(
    expression: &LegacyExpression,
) -> Result<(LegacyExpression, Option<UnlessClause>), MappingDiagnostic> {
    let parameters = parameters(expression)?;
    let unless_cost = parameters.get("UnlessCost");
    let unless_payer = parameters.get("UnlessPayer");
    let switched = match parameters.get("UnlessSwitched").map(String::as_str) {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("UnlessSwitched", value)),
    };
    if unless_cost.is_none() && unless_payer.is_none() {
        return Ok((expression.clone(), None));
    }
    let cost_text = unless_cost.ok_or_else(|| {
        diagnostic(
            "MISSING_PARAMETER",
            "UnlessPayer requires a matching UnlessCost",
        )
    })?;
    let payer_value = unless_payer
        .map(String::as_str)
        .unwrap_or("TargetedController");
    let payer = defined_player_selector(payer_value).map_err(|mut error| {
        error.message = error.message.replace("`Defined`", "`UnlessPayer`");
        error
    })?;
    let costs = parse_unless_cost(cost_text, payer_value, &payer)?;
    let mut unconditional = expression.clone();
    unconditional.fields.retain(|field| {
        !matches!(
            field.key.as_deref(),
            Some("UnlessCost" | "UnlessPayer" | "UnlessSwitched" | "UnlessResolveSubs")
        )
    });
    if let Some(value) = parameters.get("UnlessResolveSubs") {
        if !switched || value != "WhenPaid" {
            return Err(unsupported_value("UnlessResolveSubs", value));
        }
    }
    Ok((
        unconditional,
        Some(UnlessClause {
            payer,
            costs,
            switched,
        }),
    ))
}

fn parse_unless_cost(
    value: &String,
    payer_value: &str,
    payer: &Expression,
) -> Result<Vec<Expression>, MappingDiagnostic> {
    let tokens = split_cost_tokens(value).map_err(|mut error| {
        error.message = error.message.replace("`Cost`", "`UnlessCost`");
        error
    })?;
    if tokens
        .iter()
        .any(|token| matches!(token.as_str(), "Y" | "Z"))
    {
        return Err(unsupported_value("UnlessCost", value));
    }
    if payer_value != "You"
        && tokens.iter().any(|token| {
            cost_payload(token, "Sac").is_some_and(|payload| {
                payload
                    .split('/')
                    .nth(1)
                    .is_some_and(|validity| validity == "CARDNAME")
            })
        })
    {
        return Err(unsupported_value("UnlessCost", value));
    }
    let costs = parse_cost_with_controller(Some(value), payer.clone()).map_err(|mut error| {
        error.message = error.message.replace("`Cost`", "`UnlessCost`");
        error
    })?;
    if costs.is_empty() {
        return Err(unsupported_value("UnlessCost", value));
    }
    Ok(costs)
}

fn apply_unless_clause(effect: Expression, clause: UnlessClause) -> Expression {
    let mut arguments = if clause.switched {
        vec![clause.payer, effect]
    } else {
        vec![effect, clause.payer]
    };
    arguments.extend(clause.costs);
    call(
        if clause.switched {
            Operation::PayToApply
        } else {
            Operation::UnlessPaid
        },
        arguments,
    )
}

fn legacy_named_condition(value: &str) -> Result<Expression, MappingDiagnostic> {
    match value {
        "PlayerTurn" => Ok(call(
            Operation::During,
            vec![Expression::Text("your_turn".to_string())],
        )),
        "NotPlayerTurn" => Ok(call(
            Operation::Not,
            vec![call(
                Operation::During,
                vec![Expression::Text("your_turn".to_string())],
            )],
        )),
        "ExtraTurn" => Ok(call(
            Operation::During,
            vec![Expression::Text("extra_turn".to_string())],
        )),
        "Threshold" | "Delirium" | "Metalcraft" | "Hellbent" | "Blessing" | "Solved" => {
            closed_activation_condition(value)
        }
        "Kicked" => Ok(call(
            Operation::GreaterThan,
            vec![call(Operation::TimesKicked, vec![]), Expression::Integer(0)],
        )),
        _ => Err(unsupported_value("Condition", value)),
    }
}

fn activator_this_turn_cast_condition(
    expression: &LegacyExpression,
    comparison: &str,
) -> Result<Expression, MappingDiagnostic> {
    let api = expression
        .fields
        .first()
        .map(|field| field.value.trim())
        .unwrap_or_default();
    if !matches!(api, "SpellCast" | "SpellCastOrCopy") {
        return Err(unsupported_value("ActivatorThisTurnCast", comparison));
    }
    let parameters = parameters(expression)?;
    let spells = parameters
        .get("ValidCard")
        .map(|value| spell_selector(value))
        .transpose()?
        .unwrap_or_else(|| call(Operation::Spells, vec![]));
    closed_count_comparison(
        call(
            Operation::HistoryCount,
            vec![spells, Expression::Text("cast_this_turn".to_string())],
        ),
        comparison,
        "ActivatorThisTurnCast",
    )
}

fn resolve_comparison_value(
    value: &str,
    key: &str,
    context: &MappingContext<'_>,
) -> Result<Expression, MappingDiagnostic> {
    if let Ok(value) = value.parse::<i64>() {
        return Ok(Expression::Integer(value));
    }
    if context.svars.contains_key(value)
        || context.duplicate_svars.contains(value)
        || context.value_bindings.contains_key(value)
    {
        return resolve_value_svar(value, context);
    }
    if let Some(value) = value.strip_prefix("Count$") {
        return map_count_value("inline", value);
    }
    Err(unsupported_value(key, value))
}

fn closed_value_comparison(
    subject: Expression,
    comparison: &str,
    key: &str,
    context: &MappingContext<'_>,
) -> Result<Expression, MappingDiagnostic> {
    let (operator, operand) = ["GE", "LE", "EQ", "NE", "GT", "LT"]
        .into_iter()
        .find_map(|operator| {
            comparison
                .strip_prefix(operator)
                .map(|operand| (operator, operand))
        })
        .ok_or_else(|| unsupported_value(key, comparison))?;
    let operand = resolve_comparison_value(operand, key, context)?;
    Ok(match operator {
        "GE" => call(Operation::AtLeast, vec![subject, operand]),
        "LE" => call(
            Operation::Not,
            vec![call(Operation::GreaterThan, vec![subject, operand])],
        ),
        "EQ" => call(Operation::Equals, vec![subject, operand]),
        "NE" => call(
            Operation::Not,
            vec![call(Operation::Equals, vec![subject, operand])],
        ),
        "GT" => call(Operation::GreaterThan, vec![subject, operand]),
        "LT" => call(Operation::LessThan, vec![subject, operand]),
        _ => return Err(unsupported_value(key, comparison)),
    })
}

fn apply_legacy_condition(
    prefix: LegacyAbilityPrefix,
    selector_key: &str,
    mut mapped: MappedLegacyAbility,
    condition: Expression,
    check_on_resolution: bool,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    if !check_on_resolution && prefix != LegacyAbilityPrefix::Triggered {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "NoResolvingCheck is only exact for triggered abilities",
        ));
    }
    match prefix {
        LegacyAbilityPrefix::Triggered => {
            let event = mapped.event.take().ok_or_else(|| {
                diagnostic(
                    "UNSUPPORTED_CONDITION",
                    "trigger presence condition requires a typed event",
                )
            })?;
            mapped.event = Some(call(Operation::EventWhen, vec![event, condition.clone()]));
            if check_on_resolution {
                mapped.expression = call(
                    Operation::WhileCondition,
                    vec![condition, mapped.expression],
                );
            }
        }
        LegacyAbilityPrefix::Replacement => {
            if let Some(event) = mapped.event.take() {
                mapped.event = Some(call(Operation::EventWhen, vec![event, condition.clone()]));
            }
            mapped.expression = call(
                Operation::WhileCondition,
                vec![condition, mapped.expression],
            );
        }
        LegacyAbilityPrefix::Static => {
            mapped.expression = call(
                Operation::WhileCondition,
                vec![condition, mapped.expression],
            );
        }
        LegacyAbilityPrefix::Activated if selector_key == "AB" => {
            let mut timings = mapped.timing.into_iter().collect::<Vec<_>>();
            timings.push(call(Operation::TimingCondition, vec![condition]));
            mapped.timing = combine_timings(timings);
        }
        LegacyAbilityPrefix::Activated => {
            mapped.expression = call(
                Operation::WhileCondition,
                vec![condition, mapped.expression],
            );
        }
    }
    Ok(mapped)
}

fn presence_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    if value == "Card.IsCommander+YouCtrl" {
        return Ok(call(
            Operation::Permanents,
            vec![call(
                Operation::And,
                vec![
                    call(
                        Operation::DesignationIs,
                        vec![Expression::Text("commander".to_string())],
                    ),
                    call(Operation::ControlledBy, vec![call(Operation::You, vec![])]),
                ],
            )],
        ));
    }
    if value == "Equipment.Attached" {
        return Ok(call(
            Operation::Permanents,
            vec![call(
                Operation::And,
                vec![
                    call(
                        Operation::SubtypeIs,
                        vec![Expression::Text("equipment".to_string())],
                    ),
                    call(Operation::AttachedTo, vec![call(Operation::Source, vec![])]),
                ],
            )],
        ));
    }
    if let Some(counter_requirement) = value.strip_prefix("Card.Self+counters_") {
        let (minimum, counter_type) = counter_requirement
            .split_once('_')
            .ok_or_else(|| unsupported_value("IsPresent", value))?;
        let minimum = minimum
            .strip_prefix("GE")
            .ok_or_else(|| unsupported_value("IsPresent", value))?
            .parse::<i64>()
            .map_err(|_| unsupported_value("IsPresent", value))?;
        return Ok(call(
            Operation::Cards,
            vec![call(
                Operation::And,
                vec![
                    call(
                        Operation::Equals,
                        vec![
                            call(Operation::Any, vec![]),
                            call(Operation::Source, vec![]),
                        ],
                    ),
                    call(
                        Operation::WithCounter,
                        vec![
                            Expression::Text(counter_type.to_ascii_lowercase()),
                            Expression::Integer(minimum),
                        ],
                    ),
                ],
            )],
        ));
    }
    affected_selector(value).map_err(|_| unsupported_value("IsPresent", value))
}

fn condition_defined_presence_selector(
    defined: &str,
    present: &str,
) -> Result<Expression, MappingDiagnostic> {
    let presence = presence_selector(present)?;
    match defined {
        "Self" => {
            let Expression::Call {
                operation,
                mut arguments,
            } = presence
            else {
                return Err(unsupported_value("ConditionPresent", present));
            };
            if !matches!(operation, Operation::Cards | Operation::Permanents) || arguments.len() > 1
            {
                return Err(unsupported_value("ConditionPresent", present));
            }
            let source = call(
                Operation::Equals,
                vec![
                    call(Operation::Any, vec![]),
                    call(Operation::Source, vec![]),
                ],
            );
            let predicate = arguments.pop().unwrap_or(Expression::Boolean(true));
            Ok(call(
                operation,
                vec![match predicate {
                    Expression::Boolean(true) => source,
                    predicate => call(Operation::And, vec![source, predicate]),
                }],
            ))
        }
        "Targeted" => Ok(call(
            Operation::Target,
            vec![condition_defined_collection(presence, present)?],
        )),
        "Remembered" => Ok(call(
            Operation::Remembered,
            vec![condition_defined_collection(presence, present)?],
        )),
        "ChosenCard" => Ok(call(
            Operation::Chosen,
            vec![condition_defined_collection(presence, present)?],
        )),
        "TriggeredCard" | "TriggeredCardLKICopy" | "TriggeredNewCardLKICopy" => Ok(call(
            Operation::Triggered,
            vec![condition_defined_collection(presence, present)?],
        )),
        _ => Err(unsupported_value("ConditionDefined", defined)),
    }
}

fn condition_defined_collection(
    presence: Expression,
    present: &str,
) -> Result<Expression, MappingDiagnostic> {
    if matches!(
        presence,
        Expression::Call {
            operation: Operation::Cards | Operation::Permanents,
            ..
        }
    ) {
        Ok(presence)
    } else {
        Err(unsupported_value("ConditionPresent", present))
    }
}

fn closed_count_comparison(
    subject: Expression,
    comparison: &str,
    key: &str,
) -> Result<Expression, MappingDiagnostic> {
    let (operator, amount) = ["GE", "LE", "EQ", "GT", "LT"]
        .into_iter()
        .find_map(|operator| {
            comparison
                .strip_prefix(operator)
                .map(|amount| (operator, amount))
        })
        .ok_or_else(|| unsupported_value(key, comparison))?;
    let amount = amount
        .parse::<i64>()
        .map_err(|_| unsupported_value(key, comparison))?;
    Ok(match operator {
        "GE" => call(
            Operation::AtLeast,
            vec![subject, Expression::Integer(amount)],
        ),
        "LE" => call(
            Operation::LessThan,
            vec![
                subject,
                Expression::Integer(
                    amount
                        .checked_add(1)
                        .ok_or_else(|| unsupported_value(key, comparison))?,
                ),
            ],
        ),
        "EQ" => call(
            Operation::Equals,
            vec![subject, Expression::Integer(amount)],
        ),
        "GT" => call(
            Operation::GreaterThan,
            vec![subject, Expression::Integer(amount)],
        ),
        "LT" => call(
            Operation::LessThan,
            vec![subject, Expression::Integer(amount)],
        ),
        _ => return Err(unsupported_value(key, comparison)),
    })
}

#[derive(Clone, Copy)]
struct DynamicPatchSpec {
    key: &'static str,
    placeholder: &'static str,
    operation: Operation,
    argument: usize,
}

fn map_dynamic_ability(
    prefix: LegacyAbilityPrefix,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
) -> Result<Option<MappedLegacyAbility>, MappingDiagnostic> {
    let selector = expression
        .fields
        .first()
        .ok_or_else(|| diagnostic("MALFORMED_API", "ability has no API selector"))?;
    let api = selector.value.trim();
    let mut specs = match api {
        "Mana" => vec![DynamicPatchSpec {
            key: "Amount",
            placeholder: "1",
            operation: Operation::AddMana,
            argument: 2,
        }],
        "Dig" => vec![
            DynamicPatchSpec {
                key: "DigNum",
                placeholder: "1",
                operation: Operation::LibraryDig,
                argument: 1,
            },
            DynamicPatchSpec {
                key: "ChangeNum",
                placeholder: "1",
                operation: Operation::LibraryDig,
                argument: 2,
            },
        ],
        "Token" => vec![DynamicPatchSpec {
            key: "TokenAmount",
            placeholder: "1",
            operation: Operation::CreateToken,
            argument: 1,
        }],
        "GainLife" => vec![DynamicPatchSpec {
            key: "LifeAmount",
            placeholder: "1",
            operation: Operation::GainLife,
            argument: 0,
        }],
        "LoseLife" => vec![DynamicPatchSpec {
            key: "LifeAmount",
            placeholder: "1",
            operation: Operation::LoseLife,
            argument: 0,
        }],
        "Mill" => vec![DynamicPatchSpec {
            key: "NumCards",
            placeholder: "1",
            operation: Operation::Mill,
            argument: 0,
        }],
        "Draw" => vec![DynamicPatchSpec {
            key: "NumCards",
            placeholder: "1",
            operation: Operation::Draw,
            argument: 0,
        }],
        "DealDamage" | "DamageAll" => vec![DynamicPatchSpec {
            key: "NumDmg",
            placeholder: "1",
            operation: Operation::DealDamage,
            argument: 1,
        }],
        "Pump" | "PumpAll" => vec![
            DynamicPatchSpec {
                key: "NumAtt",
                placeholder: "+1",
                operation: Operation::ModifyPt,
                argument: 1,
            },
            DynamicPatchSpec {
                key: "NumDef",
                placeholder: "+1",
                operation: Operation::ModifyPt,
                argument: 2,
            },
        ],
        "Continuous" => vec![
            DynamicPatchSpec {
                key: "AddPower",
                placeholder: "1",
                operation: Operation::ModifyPt,
                argument: 1,
            },
            DynamicPatchSpec {
                key: "AddToughness",
                placeholder: "1",
                operation: Operation::ModifyPt,
                argument: 2,
            },
            DynamicPatchSpec {
                key: "SetPower",
                placeholder: "1",
                operation: Operation::SetPt,
                argument: 1,
            },
            DynamicPatchSpec {
                key: "SetToughness",
                placeholder: "1",
                operation: Operation::SetPt,
                argument: 2,
            },
        ],
        "Animate" | "AnimateAll" => vec![
            DynamicPatchSpec {
                key: "Power",
                placeholder: "1",
                operation: Operation::SetPt,
                argument: 1,
            },
            DynamicPatchSpec {
                key: "Toughness",
                placeholder: "1",
                operation: Operation::SetPt,
                argument: 2,
            },
        ],
        "ReduceCost" => vec![DynamicPatchSpec {
            key: "Amount",
            placeholder: "1",
            operation: Operation::CostReduction,
            argument: 1,
        }],
        "PutCounter" | "PutCounterAll" => vec![DynamicPatchSpec {
            key: "CounterNum",
            placeholder: "1",
            operation: Operation::AddCounter,
            argument: 2,
        }],
        "RemoveCounter" => vec![DynamicPatchSpec {
            key: "CounterNum",
            placeholder: "1",
            operation: Operation::RemoveCounters,
            argument: 2,
        }],
        "Discard" => vec![DynamicPatchSpec {
            key: "NumCards",
            placeholder: "1",
            operation: Operation::DiscardCards,
            argument: 0,
        }],
        "Scry" => vec![DynamicPatchSpec {
            key: "ScryNum",
            placeholder: "1",
            operation: Operation::Scry,
            argument: 0,
        }],
        "Explore" => vec![DynamicPatchSpec {
            key: "Num",
            placeholder: "1",
            operation: Operation::Explore,
            argument: 1,
        }],
        "Connive" => vec![DynamicPatchSpec {
            key: "ConniveNum",
            placeholder: "1",
            operation: Operation::Connive,
            argument: 1,
        }],
        "Surveil" => vec![DynamicPatchSpec {
            key: "Amount",
            placeholder: "1",
            operation: Operation::Surveil,
            argument: 0,
        }],
        "RearrangeTopOfLibrary" => vec![DynamicPatchSpec {
            key: "NumCards",
            placeholder: "1",
            operation: Operation::ReorderLibraryTop,
            argument: 1,
        }],
        "ChangeZone" => vec![DynamicPatchSpec {
            key: "ChangeNum",
            placeholder: "1",
            operation: Operation::SearchLibrary,
            argument: 2,
        }],
        "Amass" => vec![DynamicPatchSpec {
            key: "Num",
            placeholder: "1",
            operation: Operation::Amass,
            argument: 1,
        }],
        _ => Vec::new(),
    };
    specs.extend([
        DynamicPatchSpec {
            key: "TargetMin",
            placeholder: "0",
            operation: Operation::TargetRange,
            argument: 1,
        },
        DynamicPatchSpec {
            key: "TargetMax",
            placeholder: "2",
            operation: Operation::TargetRange,
            argument: 2,
        },
    ]);
    if specs.is_empty() {
        return Ok(None);
    }

    let parameters = parameters(expression)?;
    let mut replacements = Vec::new();
    let mut placeholder_expression = expression.clone();
    for spec in specs {
        let Some(replacement) = resolve_dynamic_parameter(&parameters, spec.key, context)? else {
            continue;
        };
        for field in &mut placeholder_expression.fields {
            if field.key.as_deref() == Some(spec.key) {
                field.value = spec.placeholder.to_string();
            }
        }
        replacements.push((spec, replacement));
    }
    if replacements.is_empty() {
        return Ok(None);
    }

    let mut mapped = map_legacy_ability(prefix, &placeholder_expression)?;
    for (spec, replacement) in replacements {
        let replaced = replace_operation_argument(
            &mut mapped.expression,
            spec.operation,
            spec.argument,
            &replacement,
        );
        let replaced = if spec.operation == Operation::AddMana && spec.key == "Amount" {
            replaced
                + replace_operation_argument(
                    &mut mapped.expression,
                    Operation::AddRestrictedMana,
                    3,
                    &replacement,
                )
        } else {
            replaced
        };
        if replaced == 0 {
            return Err(diagnostic(
                "DYNAMIC_LOWERING_MISMATCH",
                &format!(
                    "dynamic parameter `{}` did not produce expected `{}` operation",
                    spec.key,
                    spec.operation.as_str()
                ),
            ));
        }
    }
    Ok(Some(mapped))
}

fn resolve_dynamic_parameter(
    parameters: &BTreeMap<String, String>,
    key: &str,
    context: &MappingContext<'_>,
) -> Result<Option<Expression>, MappingDiagnostic> {
    let Some(value) = parameters.get(key) else {
        return Ok(None);
    };
    if value.parse::<i64>().is_ok() {
        return Ok(None);
    }
    let (reference, negative) = match value.strip_prefix('-') {
        Some(reference) => (reference, true),
        None => (value.strip_prefix('+').unwrap_or(value), false),
    };
    if !context.svars.contains_key(reference)
        && !context.duplicate_svars.contains(reference)
        && !context.value_bindings.contains_key(reference)
    {
        return Ok(None);
    }
    let resolved = resolve_value_svar(reference, context)?;
    Ok(Some(if negative {
        call(Operation::Negate, vec![resolved])
    } else {
        resolved
    }))
}

fn apply_target_allocation(
    expression: &mut Expression,
    expected: Operation,
    target_index: usize,
    amount_index: usize,
) -> Result<usize, MappingDiagnostic> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Ok(0);
    };
    if *operation == expected {
        if amount_index >= arguments.len() || target_index >= arguments.len() {
            return Err(diagnostic(
                "TARGET_ALLOCATION_MISMATCH",
                "effect does not expose the expected target and amount arguments",
            ));
        }
        let amount = arguments[amount_index].clone();
        let target = arguments[target_index].clone();
        if !matches!(
            target,
            Expression::Call {
                operation: Operation::Target | Operation::TargetRange,
                ..
            }
        ) {
            return Err(diagnostic(
                "TARGET_ALLOCATION_MISMATCH",
                "divided allocation requires a typed target declaration",
            ));
        }
        arguments[target_index] = call(Operation::TargetAllocation, vec![target, amount]);
        return Ok(1);
    }
    let mut applied = 0;
    for argument in arguments {
        applied += apply_target_allocation(argument, expected, target_index, amount_index)?;
    }
    Ok(applied)
}

pub(crate) fn resolve_value_svar(
    name: &str,
    context: &MappingContext<'_>,
) -> Result<Expression, MappingDiagnostic> {
    resolve_value_svar_inner(name, context, &mut Vec::new())
}

fn resolve_value_svar_inner(
    name: &str,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<Expression, MappingDiagnostic> {
    if let Some(value) = context.value_bindings.get(name) {
        return Ok(value.clone());
    }
    if context.duplicate_svars.contains(name) {
        return Err(diagnostic(
            "DUPLICATE_SVAR",
            &format!("SVar `{name}` is declared more than once"),
        ));
    }
    if stack.iter().any(|active| active == name) {
        return Err(diagnostic(
            "CYCLIC_SVAR",
            &format!("value SVar cycle reaches `{name}`"),
        ));
    }
    let expression = context.svars.get(name).copied().ok_or_else(|| {
        diagnostic(
            "MISSING_SVAR",
            &format!("referenced value SVar `{name}` is not declared"),
        )
    })?;
    if expression.fields.len() != 1 {
        return Err(diagnostic(
            "UNSUPPORTED_VALUE_SVAR",
            &format!("value SVar `{name}` is not a single closed expression"),
        ));
    }
    let field = &expression.fields[0];
    stack.push(name.to_string());
    let result = match field.key.as_deref() {
        Some("Count") => map_count_value(name, &field.value),
        Some("TriggerCount") => map_trigger_count_value(name, &field.value),
        Some("PlayerCountOpponents") => map_opponent_count_value(name, &field.value),
        Some("PlayerCountPlayers") => map_player_count_value(name, &field.value),
        Some("PlayerCountPropertyYou") => map_player_property_value(name, &field.value),
        Some("Targeted") => map_characteristic_value(
            name,
            call(Operation::Target, vec![call(Operation::Any, vec![])]),
            &field.value,
        ),
        Some("ParentTargeted") => {
            map_characteristic_value(name, call(Operation::ParentTarget, vec![]), &field.value)
        }
        Some("Triggered") => {
            map_characteristic_value(name, call(Operation::Triggered, vec![]), &field.value)
        }
        Some("TriggeredCard") => map_triggered_card_value(name, &field.value),
        Some("TargetedPlayer") => map_targeted_player_value(name, &field.value),
        Some("Sacrificed") => map_characteristic_value(
            name,
            call(
                Operation::Remembered,
                vec![Expression::Text("sacrificed".to_string())],
            ),
            &field.value,
        ),
        Some("Remembered") => map_remembered_value(name, &field.value),
        Some("SVar") => map_svar_arithmetic(name, &field.value, context, stack),
        _ => Err(diagnostic(
            "UNSUPPORTED_VALUE_SVAR",
            &format!(
                "value SVar `{name}` expression `{}` has no exact lowering",
                expression.raw
            ),
        )),
    };
    stack.pop();
    result
}

fn map_svar_arithmetic(
    name: &str,
    value: &str,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<Expression, MappingDiagnostic> {
    let (base_name, operation) = value.split_once('/').unwrap_or((value, ""));
    let base = resolve_value_svar_inner(base_name, context, stack)?;
    if operation.is_empty() {
        return Ok(base);
    }
    let operand = |value: &str, stack: &mut Vec<String>| {
        value
            .parse::<i64>()
            .map(Expression::Integer)
            .map_or_else(|_| resolve_value_svar_inner(value, context, stack), Ok)
    };
    if operation == "Twice" {
        return Ok(call(
            Operation::ScaleValue,
            vec![base, Expression::Integer(2)],
        ));
    }
    if operation == "Thrice" {
        return Ok(call(
            Operation::ScaleValue,
            vec![base, Expression::Integer(3)],
        ));
    }
    if operation == "HalfDown" || operation == "HalfUp" {
        return Ok(call(
            Operation::DivideValue,
            vec![
                base,
                Expression::Integer(2),
                Expression::Text(
                    if operation == "HalfDown" {
                        "floor"
                    } else {
                        "ceiling"
                    }
                    .to_string(),
                ),
            ],
        ));
    }
    if operation == "Abs" {
        return Ok(call(Operation::AbsoluteValue, vec![base]));
    }
    for (prefix, operation_kind) in [
        ("Plus.", Operation::AddValue),
        ("Minus.", Operation::AddValue),
        ("Times.", Operation::ScaleValue),
        ("LimitMin.", Operation::MinimumValue),
        ("LimitMax.", Operation::MaximumValue),
    ] {
        if let Some(value) = operation.strip_prefix(prefix) {
            let mut right = operand(value, stack)?;
            if prefix == "Minus." {
                right = call(Operation::Negate, vec![right]);
            }
            return Ok(call(operation_kind, vec![base, right]));
        }
    }
    Err(diagnostic(
        "UNSUPPORTED_VALUE_SVAR",
        &format!("value SVar `{name}` arithmetic `{value}` has no exact lowering"),
    ))
}

fn map_remembered_value(name: &str, value: &str) -> Result<Expression, MappingDiagnostic> {
    let remembered = call(Operation::Remembered, vec![call(Operation::Any, vec![])]);
    match value {
        "Amount" => Ok(call(Operation::Count, vec![remembered])),
        "CardPower" => Ok(call(
            Operation::Aggregate,
            vec![remembered, Expression::Text("sum_power".to_string())],
        )),
        "CardToughness" => Ok(call(
            Operation::Aggregate,
            vec![remembered, Expression::Text("sum_toughness".to_string())],
        )),
        "CardManaCost" => Ok(call(
            Operation::Aggregate,
            vec![remembered, Expression::Text("sum_mana_value".to_string())],
        )),
        "DifferentCardManaCost" => Ok(call(
            Operation::Aggregate,
            vec![
                remembered,
                Expression::Text("distinct_mana_value".to_string()),
            ],
        )),
        _ => Err(diagnostic(
            "UNSUPPORTED_VALUE_SVAR",
            &format!("value SVar `{name}` remembered value `{value}` has no exact lowering"),
        )),
    }
}

fn map_trigger_count_value(name: &str, value: &str) -> Result<Expression, MappingDiagnostic> {
    let canonical = match value {
        "DamageAmount" => "damage",
        "LifeAmount" => "life",
        "Amount" => "amount",
        "Result" => "result",
        "ScryNum" => "scry",
        _ => {
            return Err(diagnostic(
                "UNSUPPORTED_VALUE_SVAR",
                &format!("value SVar `{name}` trigger count `{value}` has no exact lowering"),
            ));
        }
    };
    Ok(call(
        Operation::TriggeredAmount,
        vec![Expression::Text(canonical.to_string())],
    ))
}

fn map_targeted_player_value(name: &str, value: &str) -> Result<Expression, MappingDiagnostic> {
    let target = call(Operation::Target, vec![call(Operation::Any, vec![])]);
    match value {
        "CardsInHand" => Ok(call(
            Operation::Count,
            vec![card_selector_in_zone("Card.TargetedPlayerOwn", "hand")?],
        )),
        "CardsInLibrary" => Ok(call(
            Operation::Count,
            vec![card_selector_in_zone("Card.TargetedPlayerOwn", "library")?],
        )),
        "CardsInGraveyard" => Ok(call(
            Operation::Count,
            vec![card_selector_in_zone(
                "Card.TargetedPlayerOwn",
                "graveyard",
            )?],
        )),
        "DamageThisTurn" => Ok(call(
            Operation::HistoryCount,
            vec![
                target,
                Expression::Text("damage_received_this_turn".to_string()),
            ],
        )),
        value => Err(diagnostic(
            "UNSUPPORTED_VALUE_SVAR",
            &format!("value SVar `{name}` targeted-player value `{value}` has no exact lowering"),
        )),
    }
}

fn map_opponent_count_value(name: &str, value: &str) -> Result<Expression, MappingDiagnostic> {
    if value == "Amount" {
        return Ok(call(Operation::OpponentCount, vec![]));
    }
    Err(diagnostic(
        "UNSUPPORTED_VALUE_SVAR",
        &format!("value SVar `{name}` opponent count `{value}` has no exact lowering"),
    ))
}

fn map_player_count_value(name: &str, value: &str) -> Result<Expression, MappingDiagnostic> {
    if value == "Amount" {
        return Ok(call(Operation::PlayerCount, vec![]));
    }
    let aggregate = match value {
        "LowestLifeTotal" => "min_life_total",
        "HighestLifeTotal" => "max_life_total",
        "AttackersDeclared" => "sum_attackers_declared_this_turn",
        "CardsDiscardedThisTurn" => "sum_cards_discarded_this_turn",
        "HasPropertyisMonarch" => "count_monarch",
        "HasPropertyLostLifeThisTurn" => "count_lost_life_this_turn",
        "HasPropertywasDealtCombatDamageThisTurn" => "count_dealt_combat_damage_this_turn",
        "HasPropertyDefending" => "count_defending_players",
        "Counters.RAD" => "sum_rad_counters",
        "Counters.ALL" => "sum_all_player_counters",
        value => {
            return Err(diagnostic(
                "UNSUPPORTED_VALUE_SVAR",
                &format!("value SVar `{name}` player count `{value}` has no exact lowering"),
            ));
        }
    };
    Ok(call(
        Operation::PlayerAggregate,
        vec![Expression::Text(aggregate.to_string())],
    ))
}

fn map_player_property_value(name: &str, value: &str) -> Result<Expression, MappingDiagnostic> {
    let history = match value {
        "CardsDiscardedThisTurn" => "cards_discarded_this_turn",
        "LifeLostThisTurn" => "life_lost_this_turn",
        "LifeLostLastTurn" => "life_lost_last_turn",
        "DamageThisTurn" => "damage_received_this_turn",
        "DamageToOppsThisTurn" => "damage_to_opponents_this_turn",
        "OpponentsAttackedThisCombat" => "opponents_attacked_this_combat",
        "OpponentsAttackedThisTurn" => "opponents_attacked_this_turn",
        "HasPropertyBeenAttackedThisCombat" => "was_attacked_this_combat",
        "LandsPlayed" => "lands_played_this_turn",
        "RingTemptedYou" => "ring_tempted_you",
        value => {
            return Err(diagnostic(
                "UNSUPPORTED_VALUE_SVAR",
                &format!("value SVar `{name}` player property `{value}` has no exact lowering"),
            ));
        }
    };
    Ok(call(
        Operation::HistoryCount,
        vec![
            call(Operation::You, vec![]),
            Expression::Text(history.to_string()),
        ],
    ))
}

fn map_count_value(name: &str, value: &str) -> Result<Expression, MappingDiagnostic> {
    if value == "xPaid" {
        return Ok(call(Operation::PaidX, vec![]));
    }
    if value == "Kicked.1.0" {
        return Ok(call(Operation::TimesKicked, vec![]));
    }
    if value == "CardPower" {
        return Ok(call(
            Operation::Power,
            vec![call(Operation::Source, vec![])],
        ));
    }
    if value == "LifeYouGainedThisTurn" {
        return Ok(call(
            Operation::HistoryCount,
            vec![
                call(Operation::You, vec![]),
                Expression::Text("life_gained_this_turn".to_string()),
            ],
        ));
    }
    if value == "AttackersDeclared" {
        return Ok(call(
            Operation::HistoryCount,
            vec![
                call(Operation::You, vec![]),
                Expression::Text("attackers_declared_this_turn".to_string()),
            ],
        ));
    }
    if value == "Domain" {
        return Ok(call(
            Operation::DistinctCount,
            vec![
                affected_selector("Land.YouCtrl")?,
                Expression::Text("basic_land_types".to_string()),
            ],
        ));
    }
    if value == "YourLifeTotal" {
        return Ok(call(
            Operation::LifeTotal,
            vec![call(Operation::You, vec![])],
        ));
    }
    if value == "Morbid.1.0" {
        return Ok(call(
            Operation::HistoryCount,
            vec![
                affected_selector("Creature")?,
                Expression::Text("died_this_turn".to_string()),
            ],
        ));
    }
    if let Some(counter_type) = value.strip_prefix("CardCounters.") {
        return Ok(call(
            Operation::CounterCount,
            vec![
                call(Operation::Source, vec![]),
                Expression::Text(counter_type.to_ascii_lowercase()),
            ],
        ));
    }
    if let Some(counter_type) = value.strip_prefix("YourCounters") {
        if counter_type.is_empty() {
            return Err(unsupported_value("SVar", value));
        }
        return Ok(call(
            Operation::CounterCount,
            vec![
                call(Operation::You, vec![]),
                Expression::Text(counter_type.to_ascii_lowercase()),
            ],
        ));
    }
    if let Some(color) = value.strip_prefix("Devotion.") {
        if color.is_empty() {
            return Err(unsupported_value("SVar", value));
        }
        return Ok(call(
            Operation::Devotion,
            vec![
                call(Operation::You, vec![]),
                Expression::Text(color.to_ascii_lowercase()),
            ],
        ));
    }
    if let Some(valid) = value.strip_prefix("Valid ") {
        if let Some(selector) = valid.strip_suffix("$GreatestCardPower") {
            return Ok(call(
                Operation::Aggregate,
                vec![
                    affected_selector(selector)?,
                    Expression::Text("max_power".to_string()),
                ],
            ));
        }
        if let Some(selector) = valid.strip_suffix("$Colors") {
            return Ok(call(
                Operation::DistinctCount,
                vec![
                    affected_selector(selector)?,
                    Expression::Text("colors".to_string()),
                ],
            ));
        }
        return Ok(call(Operation::Count, vec![affected_selector(valid)?]));
    }
    if let Some(valid) = value.strip_prefix("ValidGraveyard ") {
        if let Some(selector) = valid.strip_suffix("$GreatestCardPower") {
            return Ok(call(
                Operation::Aggregate,
                vec![
                    card_selector_in_zone(selector, "graveyard")?,
                    Expression::Text("max_power".to_string()),
                ],
            ));
        }
        return Ok(call(
            Operation::Count,
            vec![card_selector_in_zone(valid, "graveyard")?],
        ));
    }
    if let Some(valid) = value.strip_prefix("ValidHand ") {
        return Ok(call(
            Operation::Count,
            vec![card_selector_in_zone(valid, "hand")?],
        ));
    }
    if let Some(valid) = value.strip_prefix("LastStateBattlefield ") {
        return Ok(call(Operation::Count, vec![affected_selector(valid)?]));
    }
    if let Some(valid) = value.strip_prefix("ThisTurnCast_") {
        return Ok(call(
            Operation::HistoryCount,
            vec![
                affected_selector(valid)?,
                Expression::Text("cast_this_turn".to_string()),
            ],
        ));
    }
    if let Some(modifiers) =
        value.strip_prefix("ThisTurnEntered_Graveyard_from_Battlefield_Creature")
    {
        let valid = if modifiers.is_empty() {
            "Creature".to_string()
        } else if modifiers.starts_with('.') {
            format!("Creature{modifiers}")
        } else {
            return Err(unsupported_value("SVar", value));
        };
        return Ok(call(
            Operation::HistoryCount,
            vec![
                affected_selector(&valid)?,
                Expression::Text("died_this_turn".to_string()),
            ],
        ));
    }
    Err(diagnostic(
        "UNSUPPORTED_VALUE_SVAR",
        &format!("value SVar `{name}` count `{value}` has no exact lowering"),
    ))
}

fn map_characteristic_value(
    name: &str,
    selector: Expression,
    value: &str,
) -> Result<Expression, MappingDiagnostic> {
    let operation = match value {
        "CardPower" => Operation::Power,
        "CardToughness" => Operation::Toughness,
        "CardManaCost" => Operation::ManaValue,
        _ => {
            return Err(diagnostic(
                "UNSUPPORTED_VALUE_SVAR",
                &format!("value SVar `{name}` characteristic `{value}` has no exact lowering"),
            ));
        }
    };
    Ok(call(operation, vec![selector]))
}

fn map_triggered_card_value(name: &str, value: &str) -> Result<Expression, MappingDiagnostic> {
    if let Some(counter_type) = value.strip_prefix("CardCounters.") {
        if counter_type.is_empty() {
            return Err(unsupported_value("SVar", value));
        }
        return Ok(call(
            Operation::CounterCount,
            vec![
                call(Operation::Triggered, vec![]),
                Expression::Text(counter_type.to_ascii_lowercase()),
            ],
        ));
    }
    map_characteristic_value(name, call(Operation::Triggered, vec![]), value)
}

fn replace_operation_argument(
    expression: &mut Expression,
    expected: Operation,
    index: usize,
    replacement: &Expression,
) -> usize {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return 0;
    };
    let mut replaced = 0;
    if *operation == expected {
        if index < arguments.len() {
            arguments[index] = replacement.clone();
            replaced += 1;
        } else if index == arguments.len() {
            arguments.push(replacement.clone());
            replaced += 1;
        }
    }
    for argument in arguments {
        replaced += replace_operation_argument(argument, expected, index, replacement);
    }
    replaced
}

fn replace_dynamic_mana_limit(expression: &mut Expression, replacement: &Expression) -> usize {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return 0;
    };
    let mut replaced = 0;
    if *operation == Operation::LessThan
        && matches!(
            arguments.first(),
            Some(Expression::Call {
                operation: Operation::ManaValue,
                ..
            })
        )
        && arguments.get(1) == Some(&Expression::Integer(1))
    {
        arguments[1] = replacement.clone();
        replaced += 1;
    }
    for argument in arguments {
        replaced += replace_dynamic_mana_limit(argument, replacement);
    }
    replaced
}

fn wrap_target_selectors(expression: &mut Expression, unique: bool, group: Option<&str>) -> usize {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return 0;
    };
    if matches!(*operation, Operation::Target | Operation::TargetRange) {
        let mut wrapped = expression.clone();
        if unique {
            wrapped = call(Operation::UniqueTarget, vec![wrapped]);
        }
        if let Some(group) = group {
            wrapped = call(
                Operation::TargetGroup,
                vec![wrapped, Expression::Text(group.to_string())],
            );
        }
        *expression = wrapped;
        return 1;
    }
    arguments
        .iter_mut()
        .map(|argument| wrap_target_selectors(argument, unique, group))
        .sum()
}

fn resolve_svar(
    name: &str,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    if context.duplicate_svars.contains(name) {
        return Err(diagnostic(
            "DUPLICATE_SVAR",
            &format!("SVar `{name}` is declared more than once"),
        ));
    }
    if stack.iter().any(|active| active == name) {
        return Err(diagnostic(
            "CYCLIC_SVAR",
            &format!("SVar cycle reaches `{name}`"),
        ));
    }
    let expression = context.svars.get(name).copied().ok_or_else(|| {
        diagnostic(
            "MISSING_SVAR",
            &format!("referenced SVar `{name}` is not declared"),
        )
    })?;
    let selector = expression
        .fields
        .first()
        .and_then(|field| field.key.as_deref())
        .ok_or_else(|| diagnostic("MALFORMED_SVAR", "SVar has no typed selector"))?;
    let prefix = match selector {
        "Mode" if is_mapped_trigger_api(expression.fields[0].value.as_str()) => {
            LegacyAbilityPrefix::Triggered
        }
        "Mode" | "ST" => LegacyAbilityPrefix::Static,
        "Event" => LegacyAbilityPrefix::Replacement,
        "AB" | "SP" | "DB" => LegacyAbilityPrefix::Activated,
        _ => {
            return Err(diagnostic(
                "UNSUPPORTED_SVAR",
                &format!("SVar `{name}` selector `{selector}` is not an ability"),
            ));
        }
    };
    stack.push(name.to_string());
    let result = map_with_context(prefix, expression, context, stack);
    stack.pop();
    result
}

fn is_mapped_trigger_api(api: &str) -> bool {
    matches!(
        api,
        "ChangesZone"
            | "Phase"
            | "Attacks"
            | "SpellCast"
            | "SpellCastOrCopy"
            | "DamageDone"
            | "DamageDoneOnce"
            | "DamageDealtOnce"
            | "Drawn"
            | "AttackersDeclared"
            | "Blocks"
            | "AttackerBlocked"
            | "AttackerBlockedByCreature"
            | "AttackerUnblocked"
            | "BecomesTarget"
            | "Discarded"
            | "CounterAddedOnce"
            | "Taps"
            | "LifeGained"
            | "Cycled"
            | "Sacrificed"
            | "ChangesZoneAll"
            | "TurnFaceUp"
            | "ChaosEnsues"
            | "SetInMotion"
    )
}

fn map_charm_ability(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "Cost",
            "Choices",
            "Defined",
            "CharmNum",
            "MinCharmNum",
            "AdditionalDescription",
            "PrecostDesc",
        ],
    )?;
    if parameters
        .get("Defined")
        .is_some_and(|defined| defined != "You")
    {
        return Err(unsupported_value(
            "Defined",
            required(&parameters, "Defined")?,
        ));
    }
    let choice_names = required(&parameters, "Choices")?
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    if choice_names.len() < 2 {
        return Err(unsupported_value(
            "Choices",
            required(&parameters, "Choices")?,
        ));
    }
    let mut effects = Vec::new();
    for name in choice_names {
        let linked = resolve_svar(name, context, stack)?;
        if linked.event.is_some() || !linked.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("Charm choice `{name}` is not a cost-free effect chain"),
            ));
        }
        effects.push(linked.expression);
    }
    let maximum = optional_positive_integer(&parameters, "CharmNum")?.unwrap_or(1);
    let minimum = parameters
        .get("MinCharmNum")
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| unsupported_value("MinCharmNum", value))
                .and_then(|minimum| {
                    if minimum >= 0 {
                        Ok(minimum)
                    } else {
                        Err(unsupported_value("MinCharmNum", value))
                    }
                })
        })
        .transpose()?
        .unwrap_or(maximum);
    if minimum > maximum {
        return Err(unsupported_value(
            "MinCharmNum",
            required(&parameters, "MinCharmNum")?,
        ));
    }
    let expression = if minimum == 1 && maximum == 1 {
        call(Operation::ChooseOne, effects)
    } else if minimum == maximum {
        let mut arguments = vec![Expression::Integer(maximum)];
        arguments.extend(effects);
        call(Operation::ChooseExactly, arguments)
    } else if minimum == 0 {
        let mut arguments = vec![Expression::Integer(maximum)];
        arguments.extend(effects);
        call(Operation::ChooseUpTo, arguments)
    } else {
        let mut arguments = vec![Expression::Integer(minimum), Expression::Integer(maximum)];
        arguments.extend(effects);
        call(Operation::ChooseBetween, arguments)
    };
    mapped_direct(prefix, "Charm", &parameters, expression)
}

fn map_generic_choice(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "Cost",
            "Choices",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "ChoicePrompt",
            "ShowChoice",
            "SetChosenMode",
            "AtRandom",
            "TempRemember",
            "Secretly",
            "SubAbility",
            "SpellDescription",
            "StackDescription",
            "AILogic",
            "Planeswalker",
            "Ultimate",
            "IsCurse",
        ],
    )?;
    let names = required(&parameters, "Choices")?
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    if names.len() < 2 {
        return Err(unsupported_value(
            "Choices",
            required(&parameters, "Choices")?,
        ));
    }
    let mut effects = Vec::new();
    for name in names {
        let linked = resolve_svar(name, context, stack)?;
        if linked.event.is_some() || linked.timing.is_some() || !linked.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("GenericChoice option `{name}` is not a cost-free effect chain"),
            ));
        }
        effects.push(linked.expression);
    }
    for key in [
        "SetChosenMode",
        "AtRandom",
        "Secretly",
        "Planeswalker",
        "Ultimate",
        "IsCurse",
    ] {
        closed_true_flag(&parameters, key)?;
    }
    if parameters
        .get("TempRemember")
        .is_some_and(|value| value != "Chooser")
    {
        return Err(unsupported_value(
            "TempRemember",
            required(&parameters, "TempRemember")?,
        ));
    }
    let show = match parameters.get("ShowChoice").map(String::as_str) {
        None => "default",
        Some("True" | "Description" | "ExceptSelf") => required(&parameters, "ShowChoice")?,
        Some(value) => return Err(unsupported_value("ShowChoice", value)),
    };
    let mut arguments = vec![
        player_selector(&parameters, DefaultSelector::You)?,
        Expression::Text(format!(
            "{}:{}",
            if parameters.contains_key("AtRandom") {
                "random"
            } else {
                "choose"
            },
            show.to_ascii_lowercase()
        )),
        Expression::Boolean(parameters.contains_key("SetChosenMode")),
        Expression::Boolean(parameters.contains_key("TempRemember")),
        Expression::Boolean(parameters.contains_key("Secretly")),
    ];
    arguments.extend(effects);
    let mut mapped = MappedLegacyAbility {
        prefix,
        api: "GenericChoice".to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(Operation::PlayerChooseEffect, arguments),
    };
    if let Some(name) = parameters.get("SubAbility") {
        let tail = resolve_svar(name, context, stack)?;
        if tail.event.is_some() || tail.timing.is_some() || !tail.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{name}` is not a cost-free effect chain"),
            ));
        }
        mapped.expression = sequence(mapped.expression, tail.expression);
    }
    Ok(mapped)
}

fn map_replace_effect(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "Cost",
            "VarName",
            "VarValue",
            "SubAbility",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let name = required(&parameters, "VarName")?;
    if !matches!(
        name,
        "DamageAmount" | "Number" | "LifeGained" | "Amount" | "Num" | "Result" | "Ignore"
    ) {
        return Err(unsupported_value("VarName", name));
    }
    let value = map_replacement_amount(required(&parameters, "VarValue")?, context)?;
    let mut mapped = MappedLegacyAbility {
        prefix,
        api: "ReplaceEffect".to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::UpdateReplacementAmount,
            vec![Expression::Text(name.to_ascii_lowercase()), value],
        ),
    };
    if let Some(tail_name) = parameters.get("SubAbility") {
        let tail = resolve_svar(tail_name, context, stack)?;
        if tail.event.is_some() || tail.timing.is_some() || !tail.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{tail_name}` is not a cost-free effect chain"),
            ));
        }
        mapped.expression = sequence(mapped.expression, tail.expression);
    }
    Ok(mapped)
}

fn map_replacement_amount(
    value: &str,
    context: &MappingContext<'_>,
) -> Result<Expression, MappingDiagnostic> {
    if let Ok(value) = value.parse::<i64>() {
        return Ok(Expression::Integer(value));
    }
    let expression = if let Some(payload) = value.strip_prefix("ReplaceCount$") {
        payload
    } else if let Some(svar) = context.svars.get(value) {
        if svar.fields.len() != 1 || svar.fields[0].key.as_deref() != Some("ReplaceCount") {
            return resolve_value_svar(value, context);
        }
        svar.fields[0].value.as_str()
    } else {
        return resolve_value_svar(value, context);
    };
    let (name, operation) = expression
        .split_once('/')
        .ok_or_else(|| unsupported_value("VarValue", value))?;
    let current = call(
        Operation::ReplacementValue,
        vec![Expression::Text(name.to_ascii_lowercase())],
    );
    if operation == "Twice" {
        return Ok(call(
            Operation::ScaleValue,
            vec![current, Expression::Integer(2)],
        ));
    }
    if let Some(amount) = operation.strip_prefix("Plus.") {
        let amount = amount
            .parse::<i64>()
            .map_err(|_| unsupported_value("VarValue", value))?;
        return Ok(call(
            Operation::AddValue,
            vec![current, Expression::Integer(amount)],
        ));
    }
    Err(unsupported_value("VarValue", value))
}

fn map_branch(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "Cost",
            "BranchConditionSVar",
            "BranchConditionSVarCompare",
            "TrueSubAbility",
            "FalseSubAbility",
            "SubAbility",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let subject_name = required(&parameters, "BranchConditionSVar")?;
    let subject = if let Some(payload) = subject_name.strip_prefix("Count$") {
        map_count_value("BranchConditionSVar", payload)?
    } else {
        resolve_value_svar(subject_name, context)?
    };
    let comparison = parameters
        .get("BranchConditionSVarCompare")
        .map(String::as_str)
        .unwrap_or("GE1");
    let predicate = closed_count_comparison(subject, comparison, "BranchConditionSVarCompare")?;
    let true_name = required(&parameters, "TrueSubAbility")?;
    let true_effect = resolve_svar(true_name, context, stack)?;
    if true_effect.event.is_some() || true_effect.timing.is_some() || !true_effect.costs.is_empty()
    {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("TrueSubAbility `{true_name}` is not a cost-free effect"),
        ));
    }
    let false_effect = if let Some(false_name) = parameters.get("FalseSubAbility") {
        let mapped = resolve_svar(false_name, context, stack)?;
        if mapped.event.is_some() || mapped.timing.is_some() || !mapped.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("FalseSubAbility `{false_name}` is not a cost-free effect"),
            ));
        }
        mapped.expression
    } else {
        call(Operation::Sequence, vec![])
    };
    let mut effect = call(
        Operation::BranchEffect,
        vec![predicate, true_effect.expression, false_effect],
    );
    if let Some(tail_name) = parameters.get("SubAbility") {
        let tail = resolve_svar(tail_name, context, stack)?;
        if tail.event.is_some() || tail.timing.is_some() || !tail.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{tail_name}` is not a cost-free effect chain"),
            ));
        }
        effect = sequence(effect, tail.expression);
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: "Branch".to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: effect,
    })
}

fn map_flip_coin(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "Flipper",
            "Amount",
            "NoCall",
            "WinSubAbility",
            "LoseSubAbility",
            "HeadsSubAbility",
            "TailsSubAbility",
            "SubAbility",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let player = match parameters.get("Flipper") {
        Some(value) => defined_player_selector(value)?,
        None => player_selector(&parameters, DefaultSelector::You)?,
    };
    let amount = optional_positive_integer(&parameters, "Amount")?.unwrap_or(1);
    let no_call = closed_true_flag(&parameters, "NoCall")?;
    let (left, right, mode) = if no_call {
        (
            parameters.get("HeadsSubAbility"),
            parameters.get("TailsSubAbility"),
            "heads_tails",
        )
    } else {
        (
            parameters.get("WinSubAbility"),
            parameters.get("LoseSubAbility"),
            "win_lose",
        )
    };
    let resolve =
        |name: Option<&String>, stack: &mut Vec<String>| -> Result<Expression, MappingDiagnostic> {
            let Some(name) = name else {
                return Ok(call(Operation::Sequence, vec![]));
            };
            let mapped = resolve_svar(name, context, stack)?;
            if mapped.event.is_some() || mapped.timing.is_some() || !mapped.costs.is_empty() {
                return Err(diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("coin branch `{name}` is not a cost-free effect"),
                ));
            }
            Ok(mapped.expression)
        };
    let mut expression = call(
        Operation::FlipCoin,
        vec![
            player,
            Expression::Integer(amount),
            resolve(left, stack)?,
            resolve(right, stack)?,
            Expression::Text(mode.to_string()),
        ],
    );
    if let Some(tail_name) = parameters.get("SubAbility") {
        let tail = resolve_svar(tail_name, context, stack)?;
        if tail.event.is_some() || tail.timing.is_some() || !tail.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{tail_name}` is not a cost-free effect chain"),
            ));
        }
        expression = sequence(expression, tail.expression);
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: "FlipCoin".to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression,
    })
}

fn map_repeat_each(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "Cost",
            "RepeatPlayers",
            "RepeatCards",
            "DefinedCards",
            "Zone",
            "RepeatSubAbility",
            "SubAbility",
            "ChangeZoneTable",
            "DamageMap",
            "LoseLifeMap",
            "UseImprinted",
            "ChooseOrder",
            "ClearRemembered",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let source_count = ["RepeatPlayers", "RepeatCards", "DefinedCards"]
        .into_iter()
        .filter(|key| parameters.contains_key(*key))
        .count();
    if source_count != 1 {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "RepeatEach requires exactly one of RepeatPlayers, RepeatCards, or DefinedCards",
        ));
    }
    let mut repeated_objects = if let Some(value) = parameters.get("RepeatPlayers") {
        match value.as_str() {
            "Player" => call(
                Operation::All,
                vec![
                    call(Operation::You, vec![]),
                    call(Operation::Opponent, vec![]),
                ],
            ),
            "Player.Opponent" | "Opponent" => call(Operation::Opponent, vec![]),
            "Remembered" => call(Operation::Remembered, vec![call(Operation::Any, vec![])]),
            "Targeted" => call(Operation::Target, vec![call(Operation::Any, vec![])]),
            value => return Err(unsupported_value("RepeatPlayers", value)),
        }
    } else if let Some(valid) = parameters.get("RepeatCards") {
        let zones = parameters
            .get("Zone")
            .map(|value| normalize_zone_list("Zone", value))
            .transpose()?
            .unwrap_or_else(|| vec!["battlefield".to_string()]);
        let mut selectors = Vec::new();
        for zone in zones {
            selectors.push(match zone.as_str() {
                "battlefield" => affected_selector(valid)?,
                "stack" => spell_selector(valid)?,
                "graveyard" | "hand" | "exile" | "library" | "command" => {
                    card_selector_in_zone(valid, &zone)?
                }
                _ => return Err(unsupported_value("Zone", &zone)),
            });
        }
        match selectors.len() {
            1 => selectors.remove(0),
            _ => call(Operation::All, selectors),
        }
    } else {
        defined_selector(required(&parameters, "DefinedCards")?)?
    };
    if let Some(value) = parameters.get("ChooseOrder") {
        let chooser = if value == "True" {
            call(Operation::You, vec![])
        } else {
            defined_player_selector(value)?
        };
        repeated_objects = call(Operation::OrderByPlayer, vec![repeated_objects, chooser]);
    }
    let repeated_name = required(&parameters, "RepeatSubAbility")?;
    let repeated = resolve_svar(repeated_name, context, stack)?;
    if repeated.event.is_some() || !repeated.costs.is_empty() {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("RepeatSubAbility `{repeated_name}` is not a cost-free effect chain"),
        ));
    }
    let use_imprinted = closed_true_flag(&parameters, "UseImprinted")?;
    let mut repeated_effect = call(
        if use_imprinted {
            Operation::ForEachImprinted
        } else {
            Operation::ForEach
        },
        vec![repeated_objects, repeated.expression],
    );
    let mut batch_domains = Vec::new();
    for (key, domain) in [
        ("ChangeZoneTable", "zone_changes"),
        ("DamageMap", "damage"),
        ("LoseLifeMap", "life_loss"),
    ] {
        if closed_true_flag(&parameters, key)? {
            batch_domains.push(domain);
        }
    }
    if !batch_domains.is_empty() {
        repeated_effect = call(
            Operation::BatchEvents,
            vec![repeated_effect, Expression::Text(batch_domains.join(","))],
        );
    }
    if closed_true_flag(&parameters, "ClearRemembered")? {
        repeated_effect = sequence(
            call(
                Operation::Forget,
                vec![Expression::Text("remembered".to_string())],
            ),
            repeated_effect,
        );
    }
    let mut effects = vec![repeated_effect];
    if let Some(tail_name) = parameters.get("SubAbility") {
        let tail = resolve_svar(tail_name, context, stack)?;
        if tail.event.is_some() || !tail.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{tail_name}` is not a cost-free effect chain"),
            ));
        }
        effects.push(tail.expression);
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: "RepeatEach".to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: combine_effects(effects, "RepeatEach requires a linked effect")?,
    })
}

fn map_immediate_trigger(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "Cost",
            "Execute",
            "OptionalDecider",
            "RememberObjects",
            "SubAbility",
            "TriggerDescription",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let optional = match parameters.get("OptionalDecider").map(String::as_str) {
        None => false,
        Some("You") => true,
        Some(value) => return Err(unsupported_value("OptionalDecider", value)),
    };
    let execute = required(&parameters, "Execute")?;
    let linked = resolve_svar(execute, context, stack)?;
    if linked.event.is_some() || !linked.costs.is_empty() {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("Execute `{execute}` is not a cost-free immediate effect chain"),
        ));
    }
    let effect = if optional {
        call(
            Operation::ChooseUpTo,
            vec![Expression::Integer(1), linked.expression],
        )
    } else {
        linked.expression
    };
    let event = call(Operation::EventImmediate, vec![]);
    let mut expression = if let Some(value) = parameters.get("RememberObjects") {
        call(
            Operation::RegisterDelayedTriggerRemembering,
            vec![event, effect, remember_objects_selector(value)?],
        )
    } else {
        call(Operation::RegisterDelayedTrigger, vec![event, effect])
    };
    if let Some(tail_name) = parameters.get("SubAbility") {
        let tail = resolve_svar(tail_name, context, stack)?;
        if tail.event.is_some() || tail.timing.is_some() || !tail.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{tail_name}` is not a cost-free effect chain"),
            ));
        }
        expression = sequence(expression, tail.expression);
    }
    mapped_direct(prefix, "ImmediateTrigger", &parameters, expression)
}

fn map_delayed_trigger(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "Cost",
            "Mode",
            "Phase",
            "ValidPlayer",
            "Execute",
            "NextTurn",
            "ThisTurn",
            "OptionalDecider",
            "RememberObjects",
            "SubAbility",
            "TriggerDescription",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    if required(&parameters, "Mode")? != "Phase" {
        return Err(unsupported_value("Mode", required(&parameters, "Mode")?));
    }
    let player = match parameters.get("ValidPlayer").map(String::as_str) {
        None | Some("Any") | Some("Player") => call(Operation::Any, vec![]),
        Some("You") => call(Operation::You, vec![]),
        Some("Opponent" | "Player.Opponent") => call(Operation::Opponent, vec![]),
        Some(value) => return Err(unsupported_value("ValidPlayer", value)),
    };
    let event = match required(&parameters, "Phase")? {
        "Upkeep" => call(Operation::EventUpkeep, vec![player]),
        "End of Turn" | "End Of Turn" => call(
            Operation::EventPhase,
            vec![player, Expression::Text("end_step".to_string())],
        ),
        "Main1" => call(
            Operation::EventPhase,
            vec![player, Expression::Text("precombat_main".to_string())],
        ),
        "Main2" => call(
            Operation::EventPhase,
            vec![player, Expression::Text("postcombat_main".to_string())],
        ),
        "Draw" => call(
            Operation::EventPhase,
            vec![player, Expression::Text("draw_step".to_string())],
        ),
        "Cleanup" => call(
            Operation::EventPhase,
            vec![player, Expression::Text("cleanup_step".to_string())],
        ),
        "EndCombat" => call(
            Operation::EventPhase,
            vec![player, Expression::Text("end_combat".to_string())],
        ),
        value => return Err(unsupported_value("Phase", value)),
    };
    let lifetime = match (
        parameters.get("NextTurn").map(String::as_str),
        parameters.get("ThisTurn").map(String::as_str),
    ) {
        (None, None) => None,
        (Some("True"), None) => Some("next_turn"),
        (None, Some("True")) => Some("this_turn"),
        (Some(value), None) => return Err(unsupported_value("NextTurn", value)),
        (None, Some(value)) => return Err(unsupported_value("ThisTurn", value)),
        (Some(_), Some(_)) => {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "DelayedTrigger cannot combine NextTurn and ThisTurn",
            ));
        }
    };
    let execute = required(&parameters, "Execute")?;
    let linked = resolve_svar(execute, context, stack)?;
    if linked.event.is_some() || !linked.costs.is_empty() {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("Execute `{execute}` is not a cost-free delayed effect chain"),
        ));
    }
    let effect = match parameters.get("OptionalDecider").map(String::as_str) {
        None => linked.expression,
        Some("You") => call(
            Operation::ChooseUpTo,
            vec![Expression::Integer(1), linked.expression],
        ),
        Some(value) => return Err(unsupported_value("OptionalDecider", value)),
    };
    let mut expression = if let Some(value) = parameters.get("RememberObjects") {
        let mut arguments = vec![event, effect, remember_objects_selector(value)?];
        if let Some(lifetime) = lifetime {
            arguments.push(Expression::Text(lifetime.to_string()));
        }
        call(Operation::RegisterDelayedTriggerRemembering, arguments)
    } else {
        let mut arguments = vec![event, effect];
        if let Some(lifetime) = lifetime {
            arguments.push(Expression::Text(lifetime.to_string()));
        }
        call(Operation::RegisterDelayedTrigger, arguments)
    };
    if let Some(tail_name) = parameters.get("SubAbility") {
        let tail = resolve_svar(tail_name, context, stack)?;
        if tail.event.is_some() || tail.timing.is_some() || !tail.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{tail_name}` is not a cost-free effect chain"),
            ));
        }
        expression = sequence(expression, tail.expression);
    }
    mapped_direct(prefix, "DelayedTrigger", &parameters, expression)
}

fn map_damage_replacement(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Event")?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "ValidSource",
            "ValidTarget",
            "Prevent",
            "IsCombat",
            "ActiveZones",
            "ReplaceWith",
            "PreventionEffect",
            "Secondary",
            "AlwaysReplace",
            "ReplacementResult",
            "Optional",
            "OptionalDecider",
            "Description",
        ],
    )?;
    for key in ["PreventionEffect", "Secondary", "AlwaysReplace"] {
        if parameters.get(key).is_some_and(|value| value != "True") {
            return Err(unsupported_value(key, required(&parameters, key)?));
        }
    }
    if parameters
        .get("ReplacementResult")
        .is_some_and(|value| value != "Updated")
    {
        return Err(unsupported_value(
            "ReplacementResult",
            required(&parameters, "ReplacementResult")?,
        ));
    }
    let source = parameters
        .get("ValidSource")
        .map(|value| damage_event_selector(value, "ValidSource"))
        .transpose()?
        .unwrap_or_else(|| call(Operation::Any, vec![]));
    let target = parameters
        .get("ValidTarget")
        .map(|value| damage_event_selector(value, "ValidTarget"))
        .transpose()?
        .unwrap_or_else(|| call(Operation::Any, vec![]));
    let mut event_arguments = vec![source.clone(), target.clone()];
    if let Some(value) = parameters.get("IsCombat") {
        event_arguments.push(Expression::Text(
            match value.as_str() {
                "True" => "combat",
                "False" => "noncombat",
                _ => return Err(unsupported_value("IsCombat", value)),
            }
            .to_string(),
        ));
    }
    let event = call(Operation::EventDamage, event_arguments);
    let event = match parameters.get("ActiveZones").map(String::as_str) {
        None | Some("Battlefield") => event,
        Some("Command" | "Exile" | "Graveyard" | "Hand") => call(
            Operation::EventActiveZone,
            vec![
                event,
                Expression::Text(required(&parameters, "ActiveZones")?.to_ascii_lowercase()),
            ],
        ),
        Some(value) => return Err(unsupported_value("ActiveZones", value)),
    };
    let mut replacement = match (
        parameters.get("Prevent").map(String::as_str),
        parameters.get("ReplaceWith"),
    ) {
        (Some("True"), None) => call(Operation::PreventDamage, vec![source, target]),
        (Some(value), None) => return Err(unsupported_value("Prevent", value)),
        (None, Some(name)) => {
            let linked = resolve_svar(name, context, stack)?;
            if linked.event.is_some() || linked.timing.is_some() || !linked.costs.is_empty() {
                return Err(diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("ReplaceWith `{name}` is not a cost-free effect chain"),
                ));
            }
            linked.expression
        }
        (Some(_), Some(_)) => {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "DamageDone cannot combine Prevent and ReplaceWith",
            ));
        }
        (None, None) => {
            return Err(diagnostic(
                "MISSING_PARAMETER",
                "DamageDone replacement requires Prevent or ReplaceWith",
            ));
        }
    };
    match (
        parameters.get("Optional").map(String::as_str),
        parameters.get("OptionalDecider").map(String::as_str),
    ) {
        (None, None) => {}
        (Some("True"), None) | (None, Some("You")) => {
            replacement = call(
                Operation::ChooseUpTo,
                vec![Expression::Integer(1), replacement],
            );
        }
        (Some(value), None) => return Err(unsupported_value("Optional", value)),
        (None, Some(value)) => return Err(unsupported_value("OptionalDecider", value)),
        (Some(_), Some(_)) => {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "Optional and OptionalDecider cannot be combined",
            ));
        }
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: "DamageDone".to_string(),
        costs: Vec::new(),
        event: Some(event),
        timing: None,
        expression: replacement,
    })
}

fn map_moved_replacement(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Event")?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "ValidCard",
            "Origin",
            "Destination",
            "ReplaceWith",
            "ReplacementResult",
            "ActiveZones",
            "Description",
        ],
    )?;
    require_battlefield_zone(&parameters, "ActiveZones")?;
    if parameters
        .get("Origin")
        .is_some_and(|origin| origin != "Any")
    {
        return Err(unsupported_value(
            "Origin",
            required(&parameters, "Origin")?,
        ));
    }
    if parameters
        .get("ReplacementResult")
        .is_some_and(|result| result != "Updated")
    {
        return Err(unsupported_value(
            "ReplacementResult",
            required(&parameters, "ReplacementResult")?,
        ));
    }
    let destination = required(&parameters, "Destination")?;
    let affected = affected_selector(required(&parameters, "ValidCard")?)?;
    let event = if destination == "Battlefield" {
        call(Operation::EventEnters, vec![affected])
    } else {
        call(
            Operation::EventZoneChange,
            vec![affected, Expression::Text(destination.to_ascii_lowercase())],
        )
    };
    let replace_with = required(&parameters, "ReplaceWith")?;
    let linked = resolve_svar(replace_with, context, stack)?;
    if linked.event.is_some() || !linked.costs.is_empty() {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("ReplaceWith `{replace_with}` is not a cost-free effect chain"),
        ));
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: "Moved".to_string(),
        costs: Vec::new(),
        event: Some(event),
        timing: None,
        expression: linked.expression,
    })
}

fn map_untap_replacement(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Event")?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "ValidCard",
            "ValidStepTurnToController",
            "Layer",
            "ActiveZones",
            "Description",
        ],
    )?;
    require_battlefield_zone(&parameters, "ActiveZones")?;
    if required(&parameters, "Layer")? != "CantHappen" {
        return Err(unsupported_value("Layer", required(&parameters, "Layer")?));
    }
    match parameters
        .get("ValidStepTurnToController")
        .map(String::as_str)
    {
        None | Some("You") => {}
        Some(value) => return Err(unsupported_value("ValidStepTurnToController", value)),
    }
    let affected = affected_selector(required(&parameters, "ValidCard")?)?;
    Ok(MappedLegacyAbility {
        prefix,
        api: "Untap".to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: call(
            Operation::Continuous,
            vec![
                affected,
                call(
                    Operation::CannotUntap,
                    vec![
                        call(Operation::Any, vec![]),
                        Expression::Text("controller_untap_step".to_string()),
                    ],
                ),
            ],
        ),
    })
}

fn map_counter_replacement(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Event")?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "ValidCard",
            "ValidSA",
            "Layer",
            "ActiveZones",
            "Description",
        ],
    )?;
    if required(&parameters, "Layer")? != "CantHappen" {
        return Err(unsupported_value("Layer", required(&parameters, "Layer")?));
    }
    let affected = match parameters.get("ValidCard").map(String::as_str) {
        Some("Card.Self") => {
            if parameters
                .get("ValidSA")
                .is_some_and(|value| value != "Spell")
            {
                return Err(unsupported_value(
                    "ValidSA",
                    required(&parameters, "ValidSA")?,
                ));
            }
            if parameters.contains_key("ActiveZones") {
                return Err(diagnostic(
                    "UNSUPPORTED_PARAMETER",
                    "self spell counter replacement cannot have ActiveZones",
                ));
            }
            call(Operation::Source, vec![])
        }
        Some(value) => return Err(unsupported_value("ValidCard", value)),
        None => {
            require_battlefield_zone(&parameters, "ActiveZones")?;
            spell_selector(required(&parameters, "ValidSA")?)?
        }
    };
    Ok(MappedLegacyAbility {
        prefix,
        api: "Counter".to_string(),
        costs: Vec::new(),
        event: Some(call(Operation::EventCounterAttempt, vec![affected.clone()])),
        timing: None,
        expression: call(Operation::CannotBeCountered, vec![affected]),
    })
}

fn map_triggered_ability(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    let mut parameters = parameters(expression)?;
    let active_zone = parameters
        .remove("TriggerZones")
        .map(|value| match value.as_str() {
            "Battlefield" => Ok(None),
            "Graveyard" | "Exile" | "Hand" | "Command" => Ok(Some(value.to_ascii_lowercase())),
            _ => Err(unsupported_value("TriggerZones", &value)),
        })
        .transpose()?
        .flatten();
    let player_turn = parameters
        .remove("PlayerTurn")
        .map(|value| match value.as_str() {
            "True" | "You" => Ok(true),
            _ => Err(unsupported_value("PlayerTurn", &value)),
        })
        .transpose()?
        .unwrap_or(false);
    let activation_limit = parameters
        .remove("ActivationLimit")
        .map(|value| {
            value
                .parse::<i64>()
                .ok()
                .filter(|limit| (1..=3).contains(limit))
                .ok_or_else(|| unsupported_value("ActivationLimit", &value))
        })
        .transpose()?;
    let optional = parameters.remove("OptionalDecider");
    if optional.as_deref().is_some_and(|decider| decider != "You") {
        return Err(unsupported_value(
            "OptionalDecider",
            optional.as_deref().unwrap_or_default(),
        ));
    }
    if let Some(secondary) = parameters.remove("Secondary") {
        if secondary != "True" {
            return Err(unsupported_value("Secondary", &secondary));
        }
    }
    let static_trigger = match parameters.remove("Static").as_deref() {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("Static", value)),
    };
    let one_off = match parameters.remove("OneOff").as_deref() {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("OneOff", value)),
    };
    let alone = match parameters.remove("Alone").as_deref() {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("Alone", value)),
    };
    let first_time = match parameters.remove("FirstTime").as_deref() {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("FirstTime", value)),
    };
    let valid_source = if api == "BecomesTarget" {
        parameters
            .remove("ValidSource")
            .map(|value| event_source_selector(&value))
            .transpose()?
    } else {
        None
    };
    let trigger_controller = parameters
        .remove("TriggerController")
        .map(|value| match value.as_str() {
            "You" => Ok(call(Operation::You, vec![])),
            "TriggeredCardController" | "TriggeredSourceController" => Ok(call(
                Operation::ControllerOf,
                vec![call(Operation::Triggered, vec![])],
            )),
            _ => Err(unsupported_value("TriggerController", &value)),
        })
        .transpose()?;
    let execute = required(&parameters, "Execute")?;
    let event = match api {
        "ChangesZone" => map_changes_zone_event(&parameters)?,
        "Phase" => map_phase_event(&parameters)?,
        "Attacks" => map_attacks_event(&parameters)?,
        "SpellCast" => map_spell_cast_event(&parameters)?,
        "SpellCastOrCopy" => map_spell_cast_or_copy_event(&parameters)?,
        "DamageDone" => map_damage_done_event(&parameters)?,
        "DamageDoneOnce" | "DamageDealtOnce" => map_damage_done_once_event(&parameters)?,
        "Drawn" => map_drawn_event(&parameters)?,
        "AttackersDeclared" => map_attackers_declared_event(&parameters)?,
        "Blocks" => map_blocks_event(&parameters)?,
        "AttackerBlocked" => map_attacker_blocked_event(&parameters)?,
        "AttackerBlockedByCreature" => map_attacker_blocked_by_creature_event(&parameters)?,
        "AttackerUnblocked" => map_attacker_unblocked_event(&parameters)?,
        "BecomesTarget" => map_becomes_target_event(&parameters)?,
        "Discarded" => map_discarded_event(&parameters)?,
        "CounterAddedOnce" => map_counter_added_event(&parameters)?,
        "Taps" => map_taps_event(&parameters)?,
        "LifeGained" => map_life_gained_event(&parameters)?,
        "Cycled" => map_cycled_event(&parameters)?,
        "Sacrificed" => map_sacrificed_event(&parameters)?,
        "ChangesZoneAll" => map_changes_zone_all_event(&parameters)?,
        "TurnFaceUp" => map_turn_face_up_event(&parameters)?,
        "ChaosEnsues" => map_chaos_ensues_event(&parameters)?,
        "SetInMotion" => map_set_in_motion_event(&parameters)?,
        _ => {
            return Err(diagnostic(
                "UNMAPPED_API",
                &format!("no linked trigger mapper is registered for T:{api}"),
            ));
        }
    };
    let event = if alone {
        call(Operation::EventAlone, vec![event])
    } else {
        event
    };
    let event = if player_turn {
        call(
            Operation::EventWhen,
            vec![event, legacy_named_condition("PlayerTurn")?],
        )
    } else {
        event
    };
    let event = active_zone.map_or(event.clone(), |zone| {
        call(
            Operation::EventActiveZone,
            vec![event, Expression::Text(zone)],
        )
    });
    let event_limit = match (activation_limit, one_off || first_time) {
        (Some(_), true) => {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "OneOff cannot be combined with ActivationLimit",
            ));
        }
        (Some(limit), false) => Some(limit),
        (None, true) => Some(1),
        (None, false) => None,
    };
    let event = event_limit.map_or(event.clone(), |limit| {
        call(
            Operation::EventLimit,
            vec![
                event,
                call(Operation::Source, vec![]),
                Expression::Integer(limit),
            ],
        )
    });
    let event = if static_trigger {
        call(Operation::EventStatic, vec![event])
    } else {
        event
    };
    let event = trigger_controller.map_or(event.clone(), |controller| {
        call(Operation::EventController, vec![event, controller])
    });
    let event = valid_source.map_or(event.clone(), |source| {
        call(Operation::EventSource, vec![event, source])
    });
    let linked = resolve_svar(execute, context, stack)?;
    if linked.event.is_some() {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("Execute `{execute}` contains a nested event"),
        ));
    }
    let has_linked_costs = !linked.costs.is_empty();
    let mut linked_expression = linked.expression;
    if has_linked_costs {
        let mut arguments = vec![call(Operation::You, vec![]), linked_expression];
        arguments.extend(linked.costs);
        linked_expression = call(Operation::PayToApply, arguments);
    }
    let expression = if optional.is_some() && !has_linked_costs {
        call(
            Operation::ChooseUpTo,
            vec![Expression::Integer(1), linked_expression],
        )
    } else {
        linked_expression
    };
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: Some(event),
        timing: None,
        expression,
    })
}

fn map_changes_zone_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "Origin",
            "Destination",
            "ValidCard",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    let origin = parameters
        .get("Origin")
        .map(String::as_str)
        .unwrap_or("Any");
    let destination = required(parameters, "Destination")?;
    if !closed_zone(origin) || !closed_zone(destination) || origin == "Any" && destination == "Any"
    {
        return Err(diagnostic(
            "UNSUPPORTED_EVENT",
            &format!("ChangesZone transition `{origin}` -> `{destination}` is not a closed zone"),
        ));
    }
    let affected = zone_event_selector(required(parameters, "ValidCard")?, origin)?;
    Ok(if origin == "Any" && destination == "Battlefield" {
        call(Operation::EventEnters, vec![affected])
    } else if origin == "Battlefield" && destination == "Any" {
        call(Operation::EventLeaves, vec![affected])
    } else {
        call(
            Operation::EventZoneChange,
            vec![affected, Expression::Text(destination.to_ascii_lowercase())],
        )
    })
}

fn closed_zone(value: &str) -> bool {
    matches!(
        value,
        "Any" | "Battlefield" | "Graveyard" | "Hand" | "Library" | "Exile" | "Stack" | "Command"
    )
}

fn map_chaos_ensues_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &["Execute", "TriggerZones", "TriggerDescription"],
    )?;
    Ok(call(Operation::EventChaosEnsues, vec![]))
}

fn map_set_in_motion_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &["Execute", "ValidCard", "TriggerZones", "TriggerDescription"],
    )?;
    let scheme = parameters
        .get("ValidCard")
        .map(|value| affected_selector(value))
        .transpose()?
        .unwrap_or_else(|| call(Operation::Any, vec![]));
    Ok(call(Operation::EventSetInMotion, vec![scheme]))
}

fn event_source_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    match value {
        "You" => Ok(call(Operation::You, vec![])),
        "Opponent" => Ok(call(Operation::Opponent, vec![])),
        "Spell"
        | "Ability"
        | "SpellAbility.OppCtrl"
        | "SpellAbility.YouCtrl"
        | "Spell.Aura"
        | "Ability.numTargets EQ1" => Ok(call(
            Operation::StackSource,
            vec![Expression::Text(value.to_ascii_lowercase())],
        )),
        _ => Err(unsupported_value("ValidSource", value)),
    }
}

fn map_taps_event(parameters: &BTreeMap<String, String>) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &["Execute", "ValidCard", "TriggerZones", "TriggerDescription"],
    )?;
    Ok(call(
        Operation::EventTapped,
        vec![affected_selector(required(parameters, "ValidCard")?)?],
    ))
}

fn map_turn_face_up_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(parameters, &["Execute", "ValidCard", "TriggerDescription"])?;
    Ok(call(
        Operation::EventTurnedFaceUp,
        vec![affected_selector(required(parameters, "ValidCard")?)?],
    ))
}

fn map_life_gained_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidPlayer",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    Ok(call(
        Operation::EventLifeGained,
        vec![draw_player_selector(
            required(parameters, "ValidPlayer")?,
            "ValidPlayer",
        )?],
    ))
}

fn map_cycled_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidCard",
            "ValidPlayer",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    let mut arguments = vec![affected_selector(required(parameters, "ValidCard")?)?];
    if let Some(player) = parameters.get("ValidPlayer") {
        arguments.push(draw_player_selector(player, "ValidPlayer")?);
    }
    Ok(call(Operation::EventCycled, arguments))
}

fn map_sacrificed_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidCard",
            "ValidPlayer",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    let mut arguments = vec![affected_selector(required(parameters, "ValidCard")?)?];
    if let Some(player) = parameters.get("ValidPlayer") {
        arguments.push(draw_player_selector(player, "ValidPlayer")?);
    }
    Ok(call(Operation::EventSacrificed, arguments))
}

fn map_changes_zone_all_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "Origin",
            "Destination",
            "ValidCards",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    let origin = parameters
        .get("Origin")
        .map(String::as_str)
        .unwrap_or("Any");
    let destination = parameters
        .get("Destination")
        .map(String::as_str)
        .unwrap_or("Any");
    if !closed_zone(origin) || !closed_zone(destination) || origin == "Any" && destination == "Any"
    {
        return Err(diagnostic(
            "UNSUPPORTED_EVENT",
            &format!(
                "ChangesZoneAll transition `{origin}` -> `{destination}` is not a closed zone"
            ),
        ));
    }
    Ok(call(
        Operation::EventZoneChangeAll,
        vec![
            affected_selector(required(parameters, "ValidCards")?)?,
            Expression::Text(origin.to_ascii_lowercase()),
            Expression::Text(destination.to_ascii_lowercase()),
        ],
    ))
}

fn zone_event_selector(value: &str, origin: &str) -> Result<Expression, MappingDiagnostic> {
    let selector = affected_selector(value)?;
    if origin == "Any" {
        return Ok(selector);
    }
    let zone = call(
        Operation::ZoneIs,
        vec![Expression::Text(origin.to_ascii_lowercase())],
    );
    let selector = match selector {
        Expression::Call {
            operation: Operation::Source,
            ..
        } => call(
            Operation::Cards,
            vec![call(
                Operation::Equals,
                vec![
                    call(Operation::Any, vec![]),
                    call(Operation::Source, vec![]),
                ],
            )],
        ),
        Expression::Call {
            operation,
            arguments,
        } if matches!(operation, Operation::Cards | Operation::Permanents) => {
            let collection = if origin == "Battlefield" {
                operation
            } else {
                Operation::Cards
            };
            call(collection, arguments)
        }
        _ => return Err(unsupported_value("ValidCard", value)),
    };
    add_collection_predicate(selector, zone)
}

fn map_phase_event(parameters: &BTreeMap<String, String>) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "Phase",
            "ValidPlayer",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    let phase = required(parameters, "Phase")?;
    let player = match parameters.get("ValidPlayer").map(String::as_str) {
        None | Some("Any") | Some("Player") => call(Operation::Any, vec![]),
        Some("You") => call(Operation::You, vec![]),
        Some("Opponent") | Some("Player.Opponent") => call(Operation::Opponent, vec![]),
        Some(value) => return Err(unsupported_value("ValidPlayer", value)),
    };
    match phase {
        "Upkeep" => Ok(call(Operation::EventUpkeep, vec![player])),
        "End of Turn" => Ok(call(
            Operation::EventPhase,
            vec![player, Expression::Text("end_step".to_string())],
        )),
        "BeginCombat" => Ok(call(
            Operation::EventPhase,
            vec![player, Expression::Text("begin_combat".to_string())],
        )),
        "Main1" => Ok(call(
            Operation::EventPhase,
            vec![player, Expression::Text("precombat_main".to_string())],
        )),
        "Main2" => Ok(call(
            Operation::EventPhase,
            vec![player, Expression::Text("postcombat_main".to_string())],
        )),
        "Main" => Ok(call(
            Operation::EventPhase,
            vec![player, Expression::Text("main_phase".to_string())],
        )),
        "Draw" => Ok(call(
            Operation::EventPhase,
            vec![player, Expression::Text("draw_step".to_string())],
        )),
        "Cleanup" => Ok(call(
            Operation::EventPhase,
            vec![player, Expression::Text("cleanup_step".to_string())],
        )),
        "EndCombat" => Ok(call(
            Operation::EventPhase,
            vec![player, Expression::Text("end_combat".to_string())],
        )),
        "Declare Attackers" => Ok(call(
            Operation::EventPhase,
            vec![player, Expression::Text("declare_attackers".to_string())],
        )),
        "Untap" => Ok(call(
            Operation::EventPhase,
            vec![player, Expression::Text("untap_step".to_string())],
        )),
        "End Of Turn" => Ok(call(
            Operation::EventPhase,
            vec![player, Expression::Text("end_step".to_string())],
        )),
        _ => Err(unsupported_value("Phase", phase)),
    }
}

fn map_attacks_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &["Execute", "ValidCard", "TriggerZones", "TriggerDescription"],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    Ok(call(
        Operation::EventAttacks,
        vec![affected_selector(required(parameters, "ValidCard")?)?],
    ))
}

fn map_spell_cast_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidCard",
            "ValidActivatingPlayer",
            "TargetsValid",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    let spells = parameters
        .get("ValidCard")
        .map(|value| spell_selector(value))
        .transpose()?
        .unwrap_or_else(|| call(Operation::Spells, vec![]));
    let actor = spell_event_actor(parameters)?;
    if let Some(targets) = parameters.get("TargetsValid") {
        return Ok(call(
            Operation::EventCastTargeting,
            vec![
                spells,
                affected_selector(targets)?,
                actor,
                Expression::Text("cast".to_string()),
            ],
        ));
    }
    let mut arguments = vec![spells];
    if parameters.contains_key("ValidActivatingPlayer") {
        arguments.push(actor);
    }
    Ok(call(Operation::EventCast, arguments))
}

fn map_spell_cast_or_copy_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidCard",
            "ValidActivatingPlayer",
            "TargetsValid",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    let spells = parameters
        .get("ValidCard")
        .map(|value| spell_selector(value))
        .transpose()?
        .unwrap_or_else(|| call(Operation::Spells, vec![]));
    let actor = spell_event_actor(parameters)?;
    if let Some(targets) = parameters.get("TargetsValid") {
        return Ok(call(
            Operation::EventCastTargeting,
            vec![
                spells,
                affected_selector(targets)?,
                actor,
                Expression::Text("cast_or_copy".to_string()),
            ],
        ));
    }
    let actor = match parameters.get("ValidActivatingPlayer").map(String::as_str) {
        None | Some("Any") | Some("Player") => "any",
        Some("You") => "you",
        Some("Opponent") | Some("Player.Opponent") => "opponent",
        Some(value) => return Err(unsupported_value("ValidActivatingPlayer", value)),
    };
    Ok(call(
        Operation::EventCast,
        vec![spells, Expression::Text(format!("cast_or_copy:{actor}"))],
    ))
}

fn spell_event_actor(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    match parameters.get("ValidActivatingPlayer").map(String::as_str) {
        None | Some("Any") | Some("Player") => Ok(call(Operation::Any, vec![])),
        Some("You") => Ok(call(Operation::You, vec![])),
        Some("Opponent") | Some("Player.Opponent") => Ok(call(Operation::Opponent, vec![])),
        Some(value) => Err(unsupported_value("ValidActivatingPlayer", value)),
    }
}

fn map_damage_done_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidSource",
            "ValidTarget",
            "CombatDamage",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    let mut arguments = vec![
        damage_event_selector(required(parameters, "ValidSource")?, "ValidSource")?,
        damage_event_selector(required(parameters, "ValidTarget")?, "ValidTarget")?,
    ];
    if let Some(value) = parameters.get("CombatDamage") {
        arguments.push(Expression::Text(
            match value.as_str() {
                "True" => "combat",
                "False" => "noncombat",
                _ => return Err(unsupported_value("CombatDamage", value)),
            }
            .to_string(),
        ));
    }
    Ok(call(Operation::EventDamage, arguments))
}

fn map_damage_done_once_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidSource",
            "ValidTarget",
            "CombatDamage",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    let damage_kind = parameters
        .get("CombatDamage")
        .map(String::as_str)
        .unwrap_or("Any");
    Ok(call(
        Operation::EventDamage,
        vec![
            parameters
                .get("ValidSource")
                .map(|value| damage_event_selector(value, "ValidSource"))
                .transpose()?
                .unwrap_or_else(|| call(Operation::Any, vec![])),
            parameters
                .get("ValidTarget")
                .map(|value| damage_event_selector(value, "ValidTarget"))
                .transpose()?
                .unwrap_or_else(|| call(Operation::Any, vec![])),
            Expression::Text(
                match damage_kind {
                    "Any" => "once",
                    "True" => "combat_once",
                    "False" => "noncombat_once",
                    value => return Err(unsupported_value("CombatDamage", value)),
                }
                .to_string(),
            ),
        ],
    ))
}

fn map_drawn_event(parameters: &BTreeMap<String, String>) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidCard",
            "ValidPlayer",
            "Number",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    if parameters.contains_key("ValidCard") && parameters.contains_key("ValidPlayer") {
        return Err(diagnostic(
            "UNSUPPORTED_EVENT",
            "Drawn trigger has both ValidCard and ValidPlayer filters",
        ));
    }
    let drawer = if let Some(value) = parameters.get("ValidPlayer") {
        draw_player_selector(value, "ValidPlayer")?
    } else {
        draw_card_owner_selector(required(parameters, "ValidCard")?)?
    };
    let mut arguments = vec![drawer];
    if let Some(number) = parameters.get("Number") {
        arguments.push(Expression::Integer(positive_integer(number, "Number")?));
    }
    Ok(call(Operation::EventDraw, arguments))
}

fn map_attackers_declared_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "AttackingPlayer",
            "ValidAttackers",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    let mut attackers = parameters
        .get("ValidAttackers")
        .map(|value| affected_selector(value))
        .transpose()?
        .unwrap_or_else(|| {
            call(
                Operation::Permanents,
                vec![call(
                    Operation::TypeIs,
                    vec![Expression::Text("creature".to_string())],
                )],
            )
        });
    if let Some(value) = parameters.get("AttackingPlayer") {
        attackers = add_collection_predicate(
            attackers,
            call(
                Operation::ControlledBy,
                vec![draw_player_selector(value, "AttackingPlayer")?],
            ),
        )?;
    }
    Ok(call(
        Operation::EventAttacks,
        vec![attackers, Expression::Text("declaration".to_string())],
    ))
}

fn map_blocks_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidCard",
            "Secondary",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    if parameters
        .get("Secondary")
        .is_some_and(|value| value != "True")
    {
        return Err(unsupported_value(
            "Secondary",
            required(parameters, "Secondary")?,
        ));
    }
    Ok(call(
        Operation::EventBlocks,
        vec![affected_selector(required(parameters, "ValidCard")?)?],
    ))
}

fn map_attacker_blocked_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &["Execute", "ValidCard", "TriggerZones", "TriggerDescription"],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    Ok(call(
        Operation::EventBlocks,
        vec![
            affected_selector(required(parameters, "ValidCard")?)?,
            Expression::Text("attacker_blocked_once".to_string()),
        ],
    ))
}

fn map_attacker_blocked_by_creature_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidCard",
            "ValidBlocker",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    Ok(call(
        Operation::EventBlocks,
        vec![
            affected_selector(required(parameters, "ValidCard")?)?,
            affected_selector(required(parameters, "ValidBlocker")?)?,
        ],
    ))
}

fn map_attacker_unblocked_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &["Execute", "ValidCard", "TriggerZones", "TriggerDescription"],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    Ok(call(
        Operation::EventAttacks,
        vec![
            affected_selector(required(parameters, "ValidCard")?)?,
            Expression::Text("unblocked".to_string()),
        ],
    ))
}

fn map_becomes_target_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidTarget",
            "Secondary",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    if parameters
        .get("Secondary")
        .is_some_and(|value| value != "True")
    {
        return Err(unsupported_value(
            "Secondary",
            required(parameters, "Secondary")?,
        ));
    }
    Ok(call(
        Operation::EventTargeted,
        vec![affected_selector(required(parameters, "ValidTarget")?)?],
    ))
}

fn map_discarded_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidCard",
            "ValidPlayer",
            "Secondary",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    if parameters
        .get("Secondary")
        .is_some_and(|value| value != "True")
    {
        return Err(unsupported_value(
            "Secondary",
            required(parameters, "Secondary")?,
        ));
    }
    let mut arguments = vec![affected_selector(required(parameters, "ValidCard")?)?];
    if let Some(value) = parameters.get("ValidPlayer") {
        arguments.push(draw_player_selector(value, "ValidPlayer")?);
    }
    Ok(call(Operation::EventDiscard, arguments))
}

fn map_counter_added_event(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    reject_unknown(
        parameters,
        &[
            "Execute",
            "ValidCard",
            "CounterType",
            "Secondary",
            "TriggerZones",
            "TriggerDescription",
        ],
    )?;
    require_battlefield_zone(parameters, "TriggerZones")?;
    if parameters
        .get("Secondary")
        .is_some_and(|value| value != "True")
    {
        return Err(unsupported_value(
            "Secondary",
            required(parameters, "Secondary")?,
        ));
    }
    Ok(call(
        Operation::EventCounterAdded,
        vec![
            affected_selector(required(parameters, "ValidCard")?)?,
            Expression::Text(required(parameters, "CounterType")?.to_ascii_lowercase()),
        ],
    ))
}

fn draw_player_selector(value: &str, key: &str) -> Result<Expression, MappingDiagnostic> {
    match value {
        "Any" | "Player" => Ok(call(Operation::Any, vec![])),
        "You" | "Player.You" => Ok(call(Operation::You, vec![])),
        "Opponent" | "Player.Opponent" => Ok(call(Operation::Opponent, vec![])),
        _ => Err(unsupported_value(key, value)),
    }
}

fn draw_card_owner_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    match value {
        "Card" => Ok(call(Operation::Any, vec![])),
        "Card.YouCtrl" | "Card.YouOwn" => Ok(call(Operation::You, vec![])),
        "Card.OppCtrl" | "Card.OppOwn" => Ok(call(Operation::Opponent, vec![])),
        _ => Err(unsupported_value("ValidCard", value)),
    }
}

fn damage_event_selector(value: &str, key: &str) -> Result<Expression, MappingDiagnostic> {
    match value {
        "Any" | "Card" => Ok(call(Operation::Any, vec![])),
        "Card.Self" | "Creature.Self" => Ok(call(Operation::Source, vec![])),
        "You" | "Player.You" => Ok(call(Operation::You, vec![])),
        "Opponent" | "Player.Opponent" => Ok(call(Operation::Opponent, vec![])),
        "Player" => Ok(call(Operation::Any, vec![])),
        _ => affected_selector(value).map_err(|_| unsupported_value(key, value)),
    }
}

fn sequence(first: Expression, second: Expression) -> Expression {
    let mut expressions = Vec::new();
    match first {
        Expression::Call {
            operation: Operation::Sequence,
            arguments,
        } => expressions.extend(arguments),
        expression => expressions.push(expression),
    }
    match second {
        Expression::Call {
            operation: Operation::Sequence,
            arguments,
        } => expressions.extend(arguments),
        expression => expressions.push(expression),
    }
    call(Operation::Sequence, expressions)
}

/// Audits current mapping coverage over the pinned local legacy corpus.
pub fn audit_legacy_mappings(
    root: &Path,
    metrics_path: &Path,
    quarantine_path: &Path,
) -> Result<ApiCoverageReport, String> {
    let mut paths = Vec::new();
    collect_scripts(root, &mut paths)?;
    paths.sort();
    if paths.is_empty() {
        return Err(format!(
            "legacy cards root contains no .txt files: {}",
            root.display()
        ));
    }

    let mut coverage: BTreeMap<(String, String), MutableCoverage> = BTreeMap::new();
    let mut reason_counts = BTreeMap::new();
    let mut samples = Vec::new();
    let mut total_quarantined = 0;
    for path in paths {
        audit_file(
            root,
            &path,
            &mut coverage,
            &mut reason_counts,
            &mut samples,
            &mut total_quarantined,
        )?;
    }

    let mut apis = coverage
        .into_iter()
        .map(|((prefix, api), row)| ApiCoverageRow {
            prefix,
            api,
            legacy_uses: row.legacy_uses,
            mapped: row.mapped,
            verified: row.verified,
            quarantined: row.legacy_uses - row.mapped,
            quarantine_reasons: row.quarantine_reasons,
        })
        .collect::<Vec<_>>();
    apis.sort_by(|left, right| {
        right
            .legacy_uses
            .cmp(&left.legacy_uses)
            .then_with(|| left.prefix.cmp(&right.prefix))
            .then_with(|| left.api.cmp(&right.api))
    });
    let legacy_uses = apis.iter().map(|row| row.legacy_uses).sum();
    let mapped_uses = apis.iter().map(|row| row.mapped).sum();
    let verified_uses = apis.iter().map(|row| row.verified).sum();
    let source_revision = git_revision(root)?;
    let report = ApiCoverageReport {
        schema_version: 1,
        source_root: super::repository_relative(root),
        source_revision: source_revision.clone(),
        legacy_uses,
        mapped_uses,
        verified_uses,
        quarantined_uses: legacy_uses - mapped_uses,
        mapped_percent: mapped_uses as f64 * 100.0 / legacy_uses as f64,
        apis,
        quarantine_reason_counts: reason_counts.clone(),
    };
    super::write_json(metrics_path, &report)?;
    super::write_json(
        quarantine_path,
        &QuarantineReport {
            schema_version: 1,
            source_revision: &source_revision,
            total_quarantined,
            reason_counts: &reason_counts,
            samples: &samples,
        },
    )?;
    Ok(report)
}

fn audit_file(
    root: &Path,
    path: &Path,
    coverage: &mut BTreeMap<(String, String), MutableCoverage>,
    reason_counts: &mut BTreeMap<String, usize>,
    samples: &mut Vec<QuarantineSample>,
    total_quarantined: &mut usize,
) -> Result<(), String> {
    let bytes =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let relative = relative_path(root, path);
    let script = parse_legacy_script(relative.clone(), &text)
        .map_err(|error| format!("parser regression in {relative}: {error}"))?;
    let context = MappingContext::from_script(&script);
    for line in &script.lines {
        let LegacyLineKind::Ability { prefix, expression } = &line.kind else {
            continue;
        };
        let api = expression
            .fields
            .first()
            .map(|field| field.value.clone())
            .unwrap_or_else(|| "<missing>".to_string());
        let key = (prefix.as_str().to_string(), api.clone());
        let row = coverage.entry(key).or_default();
        row.legacy_uses += 1;
        match map_legacy_ability_in_context(*prefix, expression, &context) {
            Ok(_) => {
                row.mapped += 1;
                row.verified += 1;
            }
            Err(error) => {
                *total_quarantined += 1;
                *reason_counts.entry(error.code.clone()).or_insert(0) += 1;
                *row.quarantine_reasons
                    .entry(error.code.clone())
                    .or_insert(0) += 1;
                if samples.len() < 250 {
                    samples.push(QuarantineSample {
                        path: relative.clone(),
                        line: line.line,
                        prefix: prefix.as_str().to_string(),
                        api,
                        diagnostic: error,
                    });
                }
            }
        }
    }
    Ok(())
}

fn map_mana(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Produced",
            "Amount",
            "RestrictValid",
            "Defined",
            "ValidTgts",
            "SpellDescription",
        ],
    )?;
    let produced = required(parameters, "Produced")?;
    let amount = optional_positive_integer(parameters, "Amount")?.unwrap_or(1);
    let player = player_selector(parameters, DefaultSelector::You)?;
    let expression = if let Some(restriction) = parameters.get("RestrictValid") {
        call(
            Operation::AddRestrictedMana,
            vec![
                Expression::Text(normalize_mana(produced, 1)?),
                player,
                Expression::Text(normalize_mana_restriction(restriction)?),
                Expression::Integer(amount),
            ],
        )
    } else {
        call(
            Operation::AddMana,
            vec![Expression::Text(normalize_mana(produced, amount)?), player],
        )
    };
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression,
    })
}

fn map_mana_reflected(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "AB")?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "ColorOrType",
            "Valid",
            "ReflectProperty",
            "SpellDescription",
        ],
    )?;
    let mode = match required(parameters, "ColorOrType")? {
        "Color" => "color",
        "Type" => "type",
        value => return Err(unsupported_value("ColorOrType", value)),
    };
    if required(parameters, "ReflectProperty")? != "Produce" {
        return Err(unsupported_value(
            "ReflectProperty",
            required(parameters, "ReflectProperty")?,
        ));
    }
    let sources = affected_selector(required(parameters, "Valid")?)?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::AddReflectedMana,
            vec![
                sources,
                Expression::Text(mode.to_string()),
                Expression::Text("produce".to_string()),
                call(Operation::You, vec![]),
                Expression::Integer(1),
            ],
        ),
    })
}

fn map_rearrange_top_of_library(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "NumCards",
            "MayShuffle",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let amount = positive_integer(required(parameters, "NumCards")?, "NumCards")?;
    let may_shuffle = match parameters.get("MayShuffle").map(String::as_str) {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("MayShuffle", value)),
    };
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::ReorderLibraryTop,
            vec![
                player_selector(parameters, DefaultSelector::You)?,
                Expression::Integer(amount),
                Expression::Boolean(may_shuffle),
            ],
        ),
    })
}

fn map_make_card(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Conjure",
            "TokenCard",
            "Name",
            "Zone",
            "Amount",
            "RememberMade",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    match (
        parameters.get("Conjure").map(String::as_str),
        parameters.get("TokenCard").map(String::as_str),
    ) {
        (Some("True"), None) | (None, Some("True")) => {}
        (Some(_), Some(_)) => {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "MakeCard cannot combine Conjure and TokenCard",
            ));
        }
        (Some(value), None) => return Err(unsupported_value("Conjure", value)),
        (None, Some(value)) => return Err(unsupported_value("TokenCard", value)),
        (None, None) => {
            return Err(diagnostic(
                "MISSING_PARAMETER",
                "MakeCard requires Conjure or TokenCard",
            ));
        }
    }
    let name = required(parameters, "Name")?.trim();
    if name.is_empty()
        || matches!(
            name,
            "ChosenName" | "NamedCard" | "Targeted" | "TriggeredSource" | "ChosenCard"
        )
    {
        return Err(unsupported_value("Name", name));
    }
    let zone = match required(parameters, "Zone")? {
        "Hand" => "hand",
        "Battlefield" => "battlefield",
        "Library" => "library",
        "Exile" => "exile",
        "Graveyard" => "graveyard",
        value => return Err(unsupported_value("Zone", value)),
    };
    let amount = optional_positive_integer(parameters, "Amount")?.unwrap_or(1);
    if amount > 20 {
        return Err(unsupported_value("Amount", &amount.to_string()));
    }
    let conjure = || {
        call(
            Operation::ConjureCard,
            vec![
                Expression::Text(name.to_string()),
                Expression::Text(zone.to_string()),
                call(Operation::You, vec![]),
            ],
        )
    };
    let expression = if amount == 1 {
        conjure()
    } else {
        call(
            Operation::Sequence,
            (0..amount).map(|_| conjure()).collect(),
        )
    };
    let expression = apply_remembered_result(expression, parameters, "RememberMade", "made")?;
    mapped_direct(prefix, api, parameters, expression)
}

fn map_alter_attribute(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "Attributes",
            "Activate",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let activate = match parameters.get("Activate").map(String::as_str) {
        None | Some("True") => true,
        Some("False") => false,
        Some(value) => return Err(unsupported_value("Activate", value)),
    };
    let target = object_selector(parameters, DefaultSelector::Source)?;
    let mut effects = Vec::new();
    for attribute in required(parameters, "Attributes")?
        .split(',')
        .map(str::trim)
    {
        let normalized = match attribute {
            "Harnessed" => "harnessed",
            "Plotted" => "plotted",
            "Prepared" => "prepared",
            "Solve" | "Solved" => "solved",
            "Suspect" | "Suspected" => "suspected",
            "Saddle" | "Saddled" => "saddled",
            value => return Err(unsupported_value("Attributes", value)),
        };
        effects.push(call(
            Operation::AlterAttribute,
            vec![
                target.clone(),
                Expression::Text(normalized.to_string()),
                Expression::Boolean(activate),
            ],
        ));
    }
    let expression = combine_effects(effects, "AlterAttribute requires an attribute")?;
    mapped_direct(prefix, api, parameters, expression)
}

fn map_amass(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Type",
            "Num",
            "RememberAmass",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let army_type = match required(parameters, "Type")? {
        "Zombie" => "zombie",
        "Orc" => "orc",
        value => return Err(unsupported_value("Type", value)),
    };
    let amount = positive_integer(required(parameters, "Num")?, "Num")?;
    let expression = apply_remembered_result(
        call(
            Operation::Amass,
            vec![
                Expression::Text(army_type.to_string()),
                Expression::Integer(amount),
                call(Operation::You, vec![]),
            ],
        ),
        parameters,
        "RememberAmass",
        "amassed",
    )?;
    mapped_direct(prefix, api, parameters, expression)
}

fn map_draw(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "NumCards",
            "Defined",
            "ValidTgts",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let amount = optional_positive_integer(parameters, "NumCards")?.unwrap_or(1);
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::Draw,
            vec![
                Expression::Integer(amount),
                player_selector(parameters, DefaultSelector::You)?,
            ],
        ),
    })
}

fn map_dig(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "DigNum",
            "ChangeNum",
            "ChangeValid",
            "ChangeValidDesc",
            "SourceZone",
            "DestinationZone",
            "DestinationZone2",
            "LibraryPosition",
            "LibraryPosition2",
            "Reveal",
            "NoReveal",
            "Optional",
            "ForceRevealToController",
            "RestRandomOrder",
            "SkipReorder",
            "RememberChanged",
            "Tapped",
            "ExileFaceDown",
            "WithMayLook",
            "RandomChange",
            "NoLooking",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    if parameters
        .get("SourceZone")
        .is_some_and(|zone| zone != "Library")
    {
        return Err(unsupported_value(
            "SourceZone",
            required(parameters, "SourceZone")?,
        ));
    }
    let player = player_selector(parameters, DefaultSelector::You)?;
    let dig_number = positive_integer(required(parameters, "DigNum")?, "DigNum")?;
    let change_number = match parameters.get("ChangeNum").map(String::as_str) {
        None => Expression::Integer(1),
        Some("All") => Expression::Text("all".to_string()),
        Some("Any") => Expression::Text("any".to_string()),
        Some(value) => Expression::Integer(positive_integer(value, "ChangeNum")?),
    };
    let change_selector = parameters
        .get("ChangeValid")
        .map(|value| card_selector_in_zone(value, "library"))
        .transpose()?
        .unwrap_or_else(|| call(Operation::Cards, vec![]));
    let destination = normalize_dig_zone(
        parameters
            .get("DestinationZone")
            .map(String::as_str)
            .unwrap_or("Hand"),
        "DestinationZone",
    )?;
    let rest_destination = normalize_dig_zone(
        parameters
            .get("DestinationZone2")
            .map(String::as_str)
            .unwrap_or("Library"),
        "DestinationZone2",
    )?;
    let position = dig_library_position(parameters, "LibraryPosition")?;
    let rest_position = dig_library_position(parameters, "LibraryPosition2")?;
    if destination != "library" && parameters.contains_key("LibraryPosition") && position != -1 {
        return Err(unsupported_value(
            "LibraryPosition",
            required(parameters, "LibraryPosition")?,
        ));
    }
    if rest_destination != "library"
        && parameters.contains_key("LibraryPosition2")
        && rest_position != -1
    {
        return Err(unsupported_value(
            "LibraryPosition2",
            required(parameters, "LibraryPosition2")?,
        ));
    }
    if parameters.contains_key("Tapped") && destination != "battlefield" {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "Dig Tapped requires DestinationZone$ Battlefield",
        ));
    }
    if (parameters.contains_key("ExileFaceDown") || parameters.contains_key("WithMayLook"))
        && destination != "exile"
    {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "face-down or may-look Dig metadata requires DestinationZone$ Exile",
        ));
    }
    if parameters.contains_key("RestRandomOrder") && rest_destination != "library" {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "RestRandomOrder requires DestinationZone2$ Library",
        ));
    }
    let flags = [
        "Reveal",
        "NoReveal",
        "Optional",
        "ForceRevealToController",
        "RestRandomOrder",
        "SkipReorder",
        "RememberChanged",
        "Tapped",
        "ExileFaceDown",
        "WithMayLook",
        "RandomChange",
        "NoLooking",
    ]
    .into_iter()
    .map(|key| closed_true_flag(parameters, key).map(|enabled| (key, enabled)))
    .collect::<Result<Vec<_>, _>>()?;
    let options = format!(
        "source=library;position={position};rest_position={rest_position};{}",
        flags
            .into_iter()
            .map(|(key, enabled)| format!("{}={enabled}", key.to_ascii_lowercase()))
            .collect::<Vec<_>>()
            .join(";")
    );
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::LibraryDig,
            vec![
                player,
                Expression::Integer(dig_number),
                change_number,
                change_selector,
                Expression::Text(destination),
                Expression::Text(rest_destination),
                Expression::Text(options),
            ],
        ),
    })
}

fn map_dig_until(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "Valid",
            "ValidDescription",
            "Amount",
            "DigZone",
            "FoundDestination",
            "FoundLibraryPosition",
            "RevealedDestination",
            "RevealedLibraryPosition",
            "Tapped",
            "RevealRandomOrder",
            "Shuffle",
            "RememberFound",
            "RememberRevealed",
            "SpellDescription",
            "StackDescription",
            "IsCurse",
            "AILogic",
            "Planeswalker",
        ],
    )?;
    let player = player_selector(parameters, DefaultSelector::You)?;
    let amount = optional_positive_integer(parameters, "Amount")?.unwrap_or(1);
    let dig_zone = normalize_dig_zone(
        parameters
            .get("DigZone")
            .map(String::as_str)
            .unwrap_or("Library"),
        "DigZone",
    )?;
    let valid = card_selector_in_zone(required(parameters, "Valid")?, &dig_zone)?;
    let found_destination = parameters
        .get("FoundDestination")
        .map(|value| normalize_dig_zone(value, "FoundDestination"))
        .transpose()?
        .unwrap_or_else(|| "none".to_string());
    let revealed_destination = normalize_dig_zone(
        required(parameters, "RevealedDestination")?,
        "RevealedDestination",
    )?;
    let found_position = dig_library_position(parameters, "FoundLibraryPosition")?;
    let revealed_position = dig_library_position(parameters, "RevealedLibraryPosition")?;
    for key in [
        "Tapped",
        "RevealRandomOrder",
        "Shuffle",
        "RememberFound",
        "RememberRevealed",
        "IsCurse",
        "Planeswalker",
    ] {
        closed_true_flag(parameters, key)?;
    }
    if parameters.contains_key("Tapped") && found_destination != "battlefield" {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "DigUntil Tapped requires FoundDestination$ Battlefield",
        ));
    }
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::LibraryDigUntil,
            vec![
                player,
                valid,
                Expression::Integer(amount),
                Expression::Text(dig_zone),
                Expression::Text(found_destination),
                Expression::Integer(found_position),
                Expression::Text(revealed_destination),
                Expression::Integer(revealed_position),
                Expression::Boolean(parameters.contains_key("Tapped")),
                Expression::Boolean(parameters.contains_key("RevealRandomOrder")),
                Expression::Boolean(parameters.contains_key("Shuffle")),
                Expression::Boolean(parameters.contains_key("RememberFound")),
                Expression::Boolean(parameters.contains_key("RememberRevealed")),
            ],
        ),
    )
}

fn map_seek(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidPlayer",
            "Type",
            "Num",
            "RememberFound",
            "ImprintFound",
            "SpellDescription",
            "StackDescription",
            "AILogic",
            "Exhaust",
        ],
    )?;
    if parameters
        .get("ValidPlayer")
        .is_some_and(|value| value != "You")
    {
        return Err(unsupported_value(
            "ValidPlayer",
            required(parameters, "ValidPlayer")?,
        ));
    }
    for key in ["RememberFound", "ImprintFound", "Exhaust"] {
        closed_true_flag(parameters, key)?;
    }
    let player = match parameters.get("Defined") {
        Some(value) => defined_player_selector(value)?,
        None => call(Operation::You, vec![]),
    };
    let candidates = card_selector_in_zone(required(parameters, "Type")?, "library")?;
    let amount = optional_positive_integer(parameters, "Num")?.unwrap_or(1);
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::SeekLibrary,
            vec![
                player,
                candidates,
                Expression::Integer(amount),
                Expression::Boolean(parameters.contains_key("RememberFound")),
                Expression::Boolean(parameters.contains_key("ImprintFound")),
            ],
        ),
    )
}

fn map_damage(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "NumDmg",
            "DamageSource",
            "RememberDamaged",
            "ReplaceDyingDefined",
            "SpellDescription",
            "TgtPrompt",
        ],
    )?;
    if !parameters.contains_key("Defined") && !parameters.contains_key("ValidTgts") {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "DealDamage requires Defined or ValidTgts",
        ));
    }
    let target = object_selector(parameters, DefaultSelector::Source)?;
    let amount = positive_integer(required(parameters, "NumDmg")?, "NumDmg")?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: {
            let mut arguments = vec![target, Expression::Integer(amount)];
            if let Some(source) = parameters.get("DamageSource") {
                arguments.push(damage_source_selector(source)?);
            }
            let effect = call(Operation::DealDamage, arguments);
            let effect = apply_replace_dying(effect, parameters)?;
            apply_remembered_result(effect, parameters, "RememberDamaged", "damaged")?
        },
    })
}

fn map_roll_dice(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "Amount",
            "Sides",
            "ResultSVar",
            "ChosenSVar",
            "OtherSVar",
            "ToVisitYourAttractions",
            "SpellDescription",
            "StackDescription",
            "AILogic",
            "PrecostDesc",
        ],
    )?;
    let amount = optional_positive_integer(parameters, "Amount")?.unwrap_or(1);
    let sides = optional_positive_integer(parameters, "Sides")?.unwrap_or(6);
    if sides < 2 {
        return Err(unsupported_value("Sides", required(parameters, "Sides")?));
    }
    let visit = match parameters.get("ToVisitYourAttractions").map(String::as_str) {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("ToVisitYourAttractions", value)),
    };
    let result = parameters.get("ResultSVar");
    let chosen = parameters.get("ChosenSVar");
    let other = parameters.get("OtherSVar");
    if result.is_some() && (chosen.is_some() || other.is_some()) {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "ResultSVar cannot be combined with ChosenSVar or OtherSVar",
        ));
    }
    if chosen.is_some() != other.is_some() || chosen.is_some() && amount != 2 {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "ChosenSVar and OtherSVar require exactly two dice",
        ));
    }
    for (key, value) in [
        ("ResultSVar", result),
        ("ChosenSVar", chosen),
        ("OtherSVar", other),
    ] {
        if let Some(value) = value {
            let mut chars = value.chars();
            if !chars
                .next()
                .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
                || !chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                return Err(unsupported_value(key, value));
            }
        }
    }
    let options = if let Some(result) = result {
        format!("mode=standard;result={result};visit={visit}")
    } else if let (Some(chosen), Some(other)) = (chosen, other) {
        format!("mode=choose_one;chosen={chosen};other={other};visit={visit}")
    } else {
        format!("mode=standard;visit={visit}")
    };
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::RollDice,
            vec![
                player_selector(parameters, DefaultSelector::You)?,
                Expression::Integer(amount),
                Expression::Integer(sides),
                Expression::Text(options),
            ],
        ),
    })
}

fn map_peek_and_reveal(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "PeekAmount",
            "NoPeek",
            "NoReveal",
            "Reveal",
            "RevealOptional",
            "RevealValid",
            "RememberRevealed",
            "RememberPeeked",
            "ImprintRevealed",
            "SourceZone",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let amount = optional_positive_integer(parameters, "PeekAmount")?.unwrap_or(1);
    let no_peek = closed_true_flag(parameters, "NoPeek")?;
    let no_reveal = closed_true_flag(parameters, "NoReveal")?;
    let explicit_reveal = closed_true_flag(parameters, "Reveal")?;
    let optional_reveal = closed_true_flag(parameters, "RevealOptional")?;
    if (explicit_reveal || optional_reveal) && no_reveal {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "NoReveal cannot be combined with Reveal or RevealOptional",
        ));
    }
    let mode = match (no_peek, no_reveal, optional_reveal) {
        (false, false, false) => "peek_then_reveal",
        (true, false, false) => "reveal_only",
        (false, true, false) => "peek_only",
        (true, true, false) => "hidden_partition",
        (false, false, true) => "peek_then_optional_reveal",
        (true, false, true) => "optional_reveal_only",
        (_, true, true) => unreachable!("optional reveal with NoReveal was rejected"),
    };
    let memory = [
        ("RememberRevealed", "remember_revealed"),
        ("RememberPeeked", "remember_peeked"),
        ("ImprintRevealed", "imprint_revealed"),
    ]
    .into_iter()
    .filter_map(|(key, label)| {
        parameters
            .get(key)
            .map(|value| (key, label, value.as_str()))
    })
    .map(|(key, label, value)| {
        if value == "True" {
            Ok(label)
        } else {
            Err(unsupported_value(key, value))
        }
    })
    .collect::<Result<Vec<_>, _>>()?;
    if memory.len() > 1 {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "PeekAndReveal memory modes are mutually exclusive",
        ));
    }
    let source_zone = match parameters.get("SourceZone").map(String::as_str) {
        None | Some("Library") => "library",
        Some("PlanarDeck") => "planar_deck",
        Some(value) => return Err(unsupported_value("SourceZone", value)),
    };
    let library_players = peek_library_players(parameters)?;
    let revealable = affected_selector(
        parameters
            .get("RevealValid")
            .map(String::as_str)
            .unwrap_or("Card"),
    )?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::PeekLibrary,
            vec![
                library_players,
                call(Operation::You, vec![]),
                Expression::Integer(amount),
                Expression::Text(mode.to_string()),
                revealable,
                Expression::Text(memory.first().copied().unwrap_or("none").to_string()),
                Expression::Text(source_zone.to_string()),
            ],
        ),
    })
}

fn peek_library_players(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    if let Some(targets) = parameters.get("ValidTgts") {
        if parameters
            .get("Defined")
            .is_some_and(|defined| !matches!(defined.as_str(), "Targeted" | "TargetedPlayer"))
        {
            return Err(unsupported_value(
                "Defined",
                required(parameters, "Defined")?,
            ));
        }
        return Ok(call(
            Operation::Target,
            vec![draw_player_selector(targets, "ValidTgts")?],
        ));
    }
    match parameters.get("Defined").map(String::as_str) {
        None | Some("You") => Ok(call(Operation::You, vec![])),
        Some("Opponent" | "Player.Opponent") => Ok(call(Operation::Opponent, vec![])),
        Some("Player") => Ok(call(
            Operation::All,
            vec![
                call(Operation::You, vec![]),
                call(Operation::Opponent, vec![]),
            ],
        )),
        Some("TargetedAndYou") => Ok(call(
            Operation::All,
            vec![
                call(Operation::You, vec![]),
                call(Operation::Target, vec![call(Operation::Any, vec![])]),
            ],
        )),
        Some("Remembered") => Ok(call(
            Operation::Remembered,
            vec![call(Operation::Any, vec![])],
        )),
        Some("TriggeredPlayer") => Ok(call(Operation::Triggered, vec![])),
        Some(value) => Err(unsupported_value("Defined", value)),
    }
}

fn map_roll_dice_with_result(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    let parameter_map = parameters(expression)?;
    let tail_name = required(&parameter_map, "SubAbility")?;
    let mut base = expression.clone();
    base.fields
        .retain(|field| field.key.as_deref() != Some("SubAbility"));
    let base_parameters = parameters(&base)?;
    let mut mapped = map_roll_dice(prefix, "RollDice", selector, &base_parameters)?;
    let mut scoped = context.clone();
    for (key, role) in [
        ("ResultSVar", "result"),
        ("ChosenSVar", "chosen"),
        ("OtherSVar", "other"),
    ] {
        if let Some(name) = parameter_map.get(key) {
            scoped.value_bindings.insert(
                name.clone(),
                call(
                    Operation::RollResult,
                    vec![Expression::Text(role.to_string())],
                ),
            );
        }
    }
    let tail = resolve_svar(tail_name, &scoped, stack)?;
    if tail.event.is_some() || tail.timing.is_some() || !tail.costs.is_empty() {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("SubAbility `{tail_name}` is not a cost-free effect chain"),
        ));
    }
    mapped.expression = sequence(mapped.expression, tail.expression);
    Ok(mapped)
}

fn map_roll_dice_table(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "Amount",
            "Sides",
            "ResultSubAbilities",
            "SubAbility",
            "SpellDescription",
            "StackDescription",
            "AILogic",
            "PrecostDesc",
        ],
    )?;
    let amount = optional_positive_integer(&parameters, "Amount")?.unwrap_or(1);
    let sides = optional_positive_integer(&parameters, "Sides")?.unwrap_or(6);
    if sides < 2 {
        return Err(unsupported_value("Sides", required(&parameters, "Sides")?));
    }
    let mut covered = BTreeSet::new();
    let mut has_else = false;
    let mut table = Vec::new();
    let mut effects = Vec::new();
    for branch in required(&parameters, "ResultSubAbilities")?.split(',') {
        let (range, name) = branch
            .trim()
            .split_once(':')
            .ok_or_else(|| unsupported_value("ResultSubAbilities", branch))?;
        let name = name.trim();
        if name.is_empty() {
            return Err(unsupported_value("ResultSubAbilities", branch));
        }
        let canonical_range = if range == "Else" {
            if has_else {
                return Err(unsupported_value("ResultSubAbilities", branch));
            }
            has_else = true;
            "else".to_string()
        } else {
            let (minimum, maximum) = range.split_once('-').unwrap_or((range, range));
            let minimum = minimum
                .parse::<i64>()
                .map_err(|_| unsupported_value("ResultSubAbilities", branch))?;
            let maximum = maximum
                .parse::<i64>()
                .map_err(|_| unsupported_value("ResultSubAbilities", branch))?;
            if minimum < 1 || maximum > sides || minimum > maximum {
                return Err(unsupported_value("ResultSubAbilities", branch));
            }
            for value in minimum..=maximum {
                if !covered.insert(value) {
                    return Err(unsupported_value("ResultSubAbilities", branch));
                }
            }
            if minimum == maximum {
                minimum.to_string()
            } else {
                format!("{minimum}-{maximum}")
            }
        };
        let linked = resolve_svar(name, context, stack)?;
        if linked.event.is_some() || linked.timing.is_some() || !linked.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("ResultSubAbilities branch `{name}` is not a cost-free effect chain"),
            ));
        }
        table.push(canonical_range);
        effects.push(linked.expression);
    }
    if effects.is_empty() {
        return Err(unsupported_value(
            "ResultSubAbilities",
            required(&parameters, "ResultSubAbilities")?,
        ));
    }
    let mut arguments = vec![
        player_selector(&parameters, DefaultSelector::You)?,
        Expression::Integer(amount),
        Expression::Integer(sides),
        Expression::Text(table.join(",")),
    ];
    arguments.extend(effects);
    let mut effect = call(Operation::RollDiceTable, arguments);
    if let Some(tail_name) = parameters.get("SubAbility") {
        let tail = resolve_svar(tail_name, context, stack)?;
        if tail.event.is_some() || tail.timing.is_some() || !tail.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{tail_name}` is not a cost-free effect chain"),
            ));
        }
        effect = sequence(effect, tail.expression);
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: "RollDice".to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: effect,
    })
}

fn map_pump(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtZone",
            "PumpZone",
            "TgtPrompt",
            "NumAtt",
            "NumDef",
            "KW",
            "Duration",
            "AtEOT",
            "RememberObjects",
            "SpellDescription",
            "StackDescription",
            "IsCurse",
            "AILogic",
        ],
    )?;
    let perpetual = parameters.get("Duration").map(String::as_str) == Some("Perpetual");
    let duration = if perpetual {
        None
    } else {
        pump_duration(parameters)?
    };
    if duration.is_none() && parameters.contains_key("AtEOT") {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "permanent Pump cannot also carry AtEOT cleanup",
        ));
    }
    let affected = pump_object_selector(parameters)?;
    let mut modifications = Vec::new();
    if parameters.contains_key("NumAtt") || parameters.contains_key("NumDef") {
        let power = optional_signed_integer(parameters, "NumAtt")?.unwrap_or(0);
        let toughness = optional_signed_integer(parameters, "NumDef")?.unwrap_or(0);
        let mut arguments = vec![
            affected.clone(),
            Expression::Integer(power),
            Expression::Integer(toughness),
        ];
        if let Some(duration) = duration {
            arguments.push(Expression::Text(duration.to_string()));
        }
        modifications.push(call(Operation::ModifyPt, arguments));
    }
    append_keyword_grants(
        &mut modifications,
        &affected,
        parameters.get("KW"),
        duration,
    )?;
    let mut effects = Vec::new();
    if !modifications.is_empty() {
        let mut modification = combine_effects(modifications, "Pump modifications must map")?;
        if let Some(zones) = parameters.get("PumpZone") {
            let mut arguments = vec![affected.clone(), modification];
            arguments.extend(
                normalize_zone_list("PumpZone", zones)?
                    .into_iter()
                    .map(Expression::Text),
            );
            modification = call(Operation::ApplyInZones, arguments);
        }
        if perpetual {
            modification = call(Operation::Perpetual, vec![modification]);
        }
        effects.push(modification);
    }
    if let Some(value) = parameters.get("AtEOT") {
        effects.push(map_at_eot_cleanup(value, &affected)?);
    }
    if let Some(value) = parameters.get("RememberObjects") {
        effects.push(remember_objects_effect(value)?);
    }
    if effects.is_empty()
        && (parameters.contains_key("ValidTgts") || parameters.contains_key("Defined"))
    {
        effects.push(call(Operation::BindTargets, vec![affected]));
    }
    let expression = combine_effects(effects, "Pump requires a typed effect")?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression,
    })
}

fn pump_object_selector(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    let Some(zones) = parameters.get("TgtZone") else {
        return object_selector(parameters, DefaultSelector::Source);
    };
    let valid = required(parameters, "ValidTgts")?;
    if parameters.contains_key("Defined") {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "TgtZone cannot be combined with Defined",
        ));
    }
    let mut candidates = Vec::new();
    for zone in zones.split(',').map(str::trim) {
        candidates.push(match zone {
            "Battlefield" => affected_selector(valid)?,
            "Graveyard" | "Hand" | "Exile" | "Library" => {
                card_selector_in_zone(valid, &zone.to_ascii_lowercase())?
            }
            "Stack" => spell_selector(valid)?,
            value => return Err(unsupported_value("TgtZone", value)),
        });
    }
    let candidates = match candidates.len() {
        0 => return Err(unsupported_value("TgtZone", zones)),
        1 => candidates.remove(0),
        _ => call(Operation::All, candidates),
    };
    Ok(call(Operation::Target, vec![candidates]))
}

fn normalize_zone_list(key: &str, value: &str) -> Result<Vec<String>, MappingDiagnostic> {
    let mut zones = Vec::new();
    for zone in value.split(',').map(str::trim) {
        let normalized = match zone {
            "Battlefield" => "battlefield",
            "Graveyard" => "graveyard",
            "Hand" => "hand",
            "Exile" => "exile",
            "Library" => "library",
            "Stack" => "stack",
            "Command" => "command",
            "All" => "all",
            _ => return Err(unsupported_value(key, value)),
        };
        zones.push(normalized.to_string());
    }
    if zones.is_empty() {
        return Err(unsupported_value(key, value));
    }
    Ok(zones)
}

fn remember_objects_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    let mut objects = Vec::new();
    for binding in value.split(" & ").map(str::trim) {
        objects.push(match binding {
            "Targeted" | "ThisTargetedCard" => {
                call(Operation::Target, vec![call(Operation::Any, vec![])])
            }
            "Self" => call(Operation::Source, vec![]),
            "Remembered" | "RememberedCard" => {
                call(Operation::Remembered, vec![call(Operation::Any, vec![])])
            }
            "RememberedLKI" | "DelayTriggerRememberedLKI" => call(Operation::RememberedLki, vec![]),
            "ParentTarget" => call(Operation::ParentTarget, vec![]),
            "ChosenCard" => call(Operation::Chosen, vec![call(Operation::Any, vec![])]),
            "TriggeredCard" | "TriggeredCardLKICopy" | "TriggeredNewCardLKICopy" => {
                call(Operation::Triggered, vec![])
            }
            "TriggeredAttacker" | "TriggeredAttackerLKICopy" => {
                call(Operation::TriggeredAttacker, vec![])
            }
            "TriggeredBlocker" | "TriggeredBlockerLKICopy" => {
                call(Operation::TriggeredBlocker, vec![])
            }
            "TriggeredTarget" | "TriggeredTargetLKICopy" => {
                call(Operation::TriggeredTarget, vec![])
            }
            "TargetedController" => call(
                Operation::ControllerOf,
                vec![call(Operation::Target, vec![call(Operation::Any, vec![])])],
            ),
            "TriggeredPlayer" => call(Operation::TriggeredPlayer, vec![]),
            "ReplacedCard" => call(Operation::Triggered, vec![]),
            _ => return Err(unsupported_value("RememberObjects", value)),
        });
    }
    let objects = match objects.len() {
        0 => return Err(unsupported_value("RememberObjects", value)),
        1 => objects.remove(0),
        _ => call(Operation::All, objects),
    };
    Ok(objects)
}

fn remember_objects_effect(value: &str) -> Result<Expression, MappingDiagnostic> {
    Ok(call(
        Operation::Remember,
        vec![remember_objects_selector(value)?],
    ))
}

fn map_at_eot_cleanup(value: &str, affected: &Expression) -> Result<Expression, MappingDiagnostic> {
    let cleanup = match value {
        "Sacrifice" => call(Operation::SacrificeEffect, vec![affected.clone()]),
        "Destroy" => call(Operation::Destroy, vec![affected.clone()]),
        "Exile" => call(Operation::Exile, vec![affected.clone()]),
        "Hand" => call(Operation::ReturnToHand, vec![affected.clone()]),
        _ => return Err(unsupported_value("AtEOT", value)),
    };
    Ok(call(
        Operation::RegisterDelayedTrigger,
        vec![
            call(
                Operation::EventPhase,
                vec![
                    call(Operation::Any, vec![]),
                    Expression::Text("end_step".to_string()),
                ],
            ),
            cleanup,
        ],
    ))
}

fn map_pump_all(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "ValidCards",
            "ValidTgts",
            "PumpZone",
            "NumAtt",
            "NumDef",
            "KW",
            "Duration",
            "SpellDescription",
            "StackDescription",
            "IsCurse",
            "AILogic",
        ],
    )?;
    let perpetual = parameters.get("Duration").map(String::as_str) == Some("Perpetual");
    let duration = if perpetual {
        None
    } else {
        pump_duration(parameters)?
    };
    let affected = scope_collection_to_target_player(
        affected_selector(required(parameters, "ValidCards")?)?,
        parameters,
        Operation::ControlledBy,
    )?;
    let mut effects = Vec::new();
    if parameters.contains_key("NumAtt") || parameters.contains_key("NumDef") {
        let power = optional_signed_integer(parameters, "NumAtt")?.unwrap_or(0);
        let toughness = optional_signed_integer(parameters, "NumDef")?.unwrap_or(0);
        let mut arguments = vec![
            affected.clone(),
            Expression::Integer(power),
            Expression::Integer(toughness),
        ];
        if let Some(duration) = duration {
            arguments.push(Expression::Text(duration.to_string()));
        }
        effects.push(call(Operation::ModifyPt, arguments));
    }
    append_keyword_grants(&mut effects, &affected, parameters.get("KW"), duration)?;
    let mut expression =
        combine_effects(effects, "simple PumpAll requires a PT or keyword modifier")?;
    if let Some(zones) = parameters.get("PumpZone") {
        let mut arguments = vec![affected, expression];
        arguments.extend(
            normalize_zone_list("PumpZone", zones)?
                .into_iter()
                .map(Expression::Text),
        );
        expression = call(Operation::ApplyInZones, arguments);
    }
    if perpetual {
        expression = call(Operation::Perpetual, vec![expression]);
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression,
    })
}

fn map_debuff(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "Keywords",
            "Duration",
            "SpellDescription",
            "StackDescription",
            "IsCurse",
            "AILogic",
        ],
    )?;
    let affected = object_selector(parameters, DefaultSelector::Source)?;
    let duration = match parameters.get("Duration").map(String::as_str) {
        None | Some("UntilEndOfTurn") => Some("until_end_of_turn"),
        Some("UntilYourNextTurn") => Some("until_your_next_turn"),
        Some("Permanent") => None,
        Some(value) => return Err(unsupported_value("Duration", value)),
    };
    let mut effects = Vec::new();
    for keyword in required(parameters, "Keywords")?.split(" & ") {
        let mut arguments = vec![
            affected.clone(),
            Expression::Text(normalize_simple_keyword(keyword)?),
        ];
        if let Some(duration) = duration {
            arguments.push(Expression::Text(duration.to_string()));
        }
        effects.push(call(Operation::RemoveKeyword, arguments));
    }
    mapped_direct(
        prefix,
        api,
        parameters,
        combine_effects(effects, "Debuff requires at least one closed keyword")?,
    )
}

fn map_gain_life(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    map_life_change(prefix, api, selector, parameters, Operation::GainLife)
}

fn map_lose_life(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    map_life_change(prefix, api, selector, parameters, Operation::LoseLife)
}

fn map_life_change(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
    operation: Operation,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "LifeAmount",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let amount = positive_integer(required(parameters, "LifeAmount")?, "LifeAmount")?;
    let affected = player_selector(parameters, DefaultSelector::You)?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(operation, vec![Expression::Integer(amount), affected]),
    })
}

fn map_mill(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "NumCards",
            "Destination",
            "RememberMilled",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    if parameters
        .get("Destination")
        .is_some_and(|destination| destination != "Graveyard")
    {
        return Err(unsupported_value(
            "Destination",
            required(parameters, "Destination")?,
        ));
    }
    let amount = optional_positive_integer(parameters, "NumCards")?.unwrap_or(1);
    let affected = player_selector(parameters, DefaultSelector::You)?;
    let expression = apply_remembered_result(
        call(Operation::Mill, vec![Expression::Integer(amount), affected]),
        parameters,
        "RememberMilled",
        "milled",
    )?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression,
    })
}

fn map_tap(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    map_object_effect(prefix, api, selector, parameters, Operation::Tap)
}

fn map_untap(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    if parameters.contains_key("UntapType") || parameters.contains_key("UntapUpTo") {
        require_selector_one_of(selector, &["AB", "SP", "DB"])?;
        reject_unknown(
            parameters,
            &[
                "Cost",
                "UntapType",
                "UntapUpTo",
                "Amount",
                "SpellDescription",
                "StackDescription",
                "AILogic",
            ],
        )?;
        if required(parameters, "UntapUpTo")? != "True" {
            return Err(unsupported_value(
                "UntapUpTo",
                required(parameters, "UntapUpTo")?,
            ));
        }
        let amount = positive_integer(required(parameters, "Amount")?, "Amount")?;
        let choose = call(
            Operation::ChooseObjects,
            vec![
                affected_selector(required(parameters, "UntapType")?)?,
                Expression::Integer(amount),
                call(Operation::You, vec![]),
                Expression::Text("up_to".to_string()),
            ],
        );
        return mapped_direct(
            prefix,
            api,
            parameters,
            sequence(
                choose,
                call(
                    Operation::Untap,
                    vec![call(Operation::EffectResult, vec![])],
                ),
            ),
        );
    }
    map_object_effect(prefix, api, selector, parameters, Operation::Untap)
}

fn map_regenerate(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &["Cost", "Defined", "ValidTgts", "SpellDescription"],
    )?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::RegenerateShield,
            vec![object_selector(parameters, DefaultSelector::Source)?],
        ),
    })
}

fn map_destroy(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    map_object_effect(prefix, api, selector, parameters, Operation::Destroy)
}

fn map_object_effect(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
    operation: Operation,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "SpellDescription",
            "StackDescription",
            "IsCurse",
            "AILogic",
            "ETB",
            "NoRegen",
            "RememberDestroyed",
        ],
    )?;
    if let Some(etb) = parameters.get("ETB") {
        if operation != Operation::Tap || etb != "True" {
            return Err(unsupported_value("ETB", etb));
        }
    }
    let mut arguments = vec![object_selector(parameters, DefaultSelector::Source)?];
    if let Some(value) = parameters.get("NoRegen") {
        if operation != Operation::Destroy || value != "True" {
            return Err(unsupported_value("NoRegen", value));
        }
        arguments.push(Expression::Text("cannot_regenerate".to_string()));
    }
    let expression = call(operation, arguments);
    let expression = if operation == Operation::Destroy {
        apply_remembered_result(expression, parameters, "RememberDestroyed", "destroyed")?
    } else if parameters.contains_key("RememberDestroyed") {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "RememberDestroyed is only valid for Destroy",
        ));
    } else {
        expression
    };
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression,
    })
}

fn map_put_counter(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtZone",
            "TgtPrompt",
            "CounterType",
            "CounterTypes",
            "CounterNum",
            "Bolster",
            "Support",
            "Adapt",
            "Monstrosity",
            "Placer",
            "TriggeredCounterMap",
            "CounterMapValues",
            "TargetMin",
            "TargetMax",
            "SpellDescription",
            "StackDescription",
            "IsCurse",
            "AILogic",
        ],
    )?;
    let mechanics = ["Bolster", "Support", "Adapt", "Monstrosity"]
        .into_iter()
        .filter(|key| parameters.contains_key(*key))
        .collect::<Vec<_>>();
    if parameters.contains_key("TriggeredCounterMap") {
        if !mechanics.is_empty()
            || parameters.contains_key("CounterType")
            || parameters.contains_key("CounterTypes")
        {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "TriggeredCounterMap cannot be combined with a counter type or named mechanic",
            ));
        }
        if required(parameters, "TriggeredCounterMap")? != "True" {
            return Err(unsupported_value(
                "TriggeredCounterMap",
                required(parameters, "TriggeredCounterMap")?,
            ));
        }
        let target = object_selector(parameters, DefaultSelector::Source)?;
        let placer = match parameters.get("Placer").map(String::as_str) {
            None => call(Operation::You, vec![]),
            Some("TriggeredSource") => call(Operation::Triggered, vec![]),
            Some(value) => defined_player_selector(value)?,
        };
        let mut arguments = vec![target, placer];
        if let Some(value) = parameters.get("CounterMapValues") {
            let amount = value
                .parse::<i64>()
                .ok()
                .filter(|amount| *amount > 0)
                .ok_or_else(|| unsupported_value("CounterMapValues", value))?;
            arguments.push(Expression::Integer(amount));
        }
        return Ok(MappedLegacyAbility {
            prefix,
            api: api.to_string(),
            costs: parse_simple_cost(parameters.get("Cost"))?,
            event: None,
            timing: None,
            expression: call(Operation::CopyTriggeredCounters, arguments),
        });
    }
    if mechanics.len() > 1 {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "PutCounter cannot combine named counter mechanics",
        ));
    }
    if let Some(mechanic) = mechanics.first() {
        if parameters.contains_key("CounterType") || parameters.contains_key("CounterTypes") {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "named counter mechanics cannot be combined with CounterType or CounterTypes",
            ));
        }
        let value = required(parameters, mechanic)?;
        let amount = value
            .parse::<i64>()
            .ok()
            .filter(|amount| *amount > 0)
            .ok_or_else(|| unsupported_value(mechanic, value))?;
        let (operation, target) = match *mechanic {
            "Bolster" => (Operation::Bolster, call(Operation::You, vec![])),
            "Support" => (
                Operation::Support,
                if parameters.contains_key("ValidTgts") || parameters.contains_key("Defined") {
                    object_selector(parameters, DefaultSelector::Source)?
                } else {
                    affected_selector("Creature.Other+YouCtrl")?
                },
            ),
            "Adapt" => (
                Operation::Adapt,
                object_selector(parameters, DefaultSelector::Source)?,
            ),
            "Monstrosity" => (
                Operation::Monstrosity,
                object_selector(parameters, DefaultSelector::Source)?,
            ),
            _ => return Err(unsupported_value("PutCounter", mechanic)),
        };
        return Ok(MappedLegacyAbility {
            prefix,
            api: api.to_string(),
            costs: parse_simple_cost(parameters.get("Cost"))?,
            event: None,
            timing: None,
            expression: call(operation, vec![target, Expression::Integer(amount)]),
        });
    }
    if parameters.contains_key("CounterType") && parameters.contains_key("CounterTypes") {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "PutCounter cannot combine CounterType and CounterTypes",
        ));
    }
    let counter_types = parameters
        .get("CounterTypes")
        .or_else(|| parameters.get("CounterType"))
        .ok_or_else(|| {
            diagnostic(
                "MISSING_PARAMETER",
                "PutCounter requires CounterType or CounterTypes",
            )
        })?;
    let amount = optional_positive_integer(parameters, "CounterNum")?.unwrap_or(1);
    let target = object_selector(parameters, DefaultSelector::Source)?;
    let mut effects = Vec::new();
    for counter_type in counter_types.split(',').map(str::trim) {
        if counter_type.is_empty() {
            return Err(unsupported_value("CounterTypes", counter_types));
        }
        effects.push(call(
            Operation::AddCounter,
            vec![
                target.clone(),
                Expression::Text(counter_type.to_ascii_lowercase()),
                Expression::Integer(amount),
            ],
        ));
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: combine_effects(effects, "PutCounter requires at least one counter type")?,
    })
}

fn map_continuous(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    map_continuous_with_effects(prefix, api, selector, parameters, Vec::new())
}

fn map_name_card(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "ValidCards",
            "ValidDescription",
            "SelectPrompt",
            "AtRandom",
            "SpellDescription",
            "StackDescription",
            "AILogic",
            "Planeswalker",
        ],
    )?;
    if parameters
        .get("Planeswalker")
        .is_some_and(|value| value != "True")
    {
        return Err(unsupported_value(
            "Planeswalker",
            required(parameters, "Planeswalker")?,
        ));
    }
    let chooser = player_selector(parameters, DefaultSelector::You)?;
    let choices = parameters
        .get("ValidCards")
        .map(|value| affected_selector(value))
        .transpose()?
        .unwrap_or_else(|| call(Operation::Any, vec![]));
    let mode = if closed_true_flag(parameters, "AtRandom")? {
        "random"
    } else {
        "choice"
    };
    let mut arguments = vec![chooser, choices, Expression::Text(mode.to_string())];
    if let Some(prompt) = parameters
        .get("SelectPrompt")
        .or_else(|| parameters.get("ValidDescription"))
    {
        arguments.push(Expression::Text(prompt.clone()));
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(Operation::ChooseCardName, arguments),
    })
}

fn map_continuous_with_effects(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
    mut effects: Vec<Expression>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    reject_unknown(
        parameters,
        &[
            "Affected",
            "AddPower",
            "AddToughness",
            "AddKeyword",
            "AddHiddenKeyword",
            "RemoveKeyword",
            "CantHaveKeyword",
            "AdjustLandPlays",
            "SetPower",
            "SetToughness",
            "AddType",
            "RemoveType",
            "RemoveCardTypes",
            "RemoveCreatureTypes",
            "RemoveLandTypes",
            "SetColor",
            "RemoveAllAbilities",
            "GainControl",
            "SetMaxHandSize",
            "AffectedZone",
            "EffectZone",
            "MayPlay",
            "MayLookAt",
            "CharacteristicDefining",
            "Description",
        ],
    )?;
    if let Some(may_play) = parameters.get("MayPlay") {
        if may_play != "True" {
            return Err(unsupported_value("MayPlay", may_play));
        }
        require_static_effect_zone(parameters, "EffectZone")?;
        let affected_value = required(parameters, "Affected")?;
        let zone = match required(parameters, "AffectedZone")? {
            "Exile" => "exile",
            "Library" => "library",
            "Graveyard" => "graveyard",
            "Hand" => "hand",
            value => return Err(unsupported_value("AffectedZone", value)),
        };
        let affected = affected_selector(affected_value)?;
        let play_permission = if zone == "exile" {
            call(
                Operation::PlayExiled,
                vec![affected.clone(), call(Operation::You, vec![])],
            )
        } else {
            call(
                Operation::PlayFromZone,
                vec![
                    affected.clone(),
                    call(Operation::You, vec![]),
                    Expression::Text(zone.to_string()),
                ],
            )
        };
        let mut permissions = vec![play_permission];
        if let Some(value) = parameters.get("MayLookAt") {
            let (viewer, visibility) = match value.as_str() {
                "You" => (call(Operation::You, vec![]), "private"),
                "Player" => (call(Operation::Any, vec![]), "public"),
                _ => return Err(unsupported_value("MayLookAt", value)),
            };
            permissions.push(call(
                Operation::LookPermission,
                vec![
                    affected.clone(),
                    viewer,
                    Expression::Text(visibility.to_string()),
                ],
            ));
        }
        return Ok(MappedLegacyAbility {
            prefix,
            api: api.to_string(),
            costs: Vec::new(),
            event: None,
            timing: None,
            expression: call(
                Operation::Continuous,
                vec![
                    affected.clone(),
                    combine_effects(permissions, "MayPlay requires a permission")?,
                    Expression::Text(zone.to_string()),
                ],
            ),
        });
    }
    let characteristic_defining = match parameters.get("CharacteristicDefining").map(String::as_str)
    {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("CharacteristicDefining", value)),
    };
    let affected_value = match parameters.get("Affected") {
        Some(value) if characteristic_defining => {
            return Err(unsupported_value("Affected", value));
        }
        Some(value) => value.as_str(),
        None if characteristic_defining => "Card.Self",
        None => {
            return Err(diagnostic(
                "MISSING_PARAMETER",
                "required parameter `Affected` is absent",
            ));
        }
    };
    let continuous_zone = if characteristic_defining {
        if let Some(zone) = parameters.get("AffectedZone") {
            return Err(unsupported_value("AffectedZone", zone));
        }
        if let Some(zone) = parameters.get("EffectZone") {
            return Err(unsupported_value("EffectZone", zone));
        }
        if !parameters.contains_key("SetPower") && !parameters.contains_key("SetToughness") {
            return Err(diagnostic(
                "MISSING_PARAMETER",
                "CharacteristicDefining requires SetPower or SetToughness",
            ));
        }
        None
    } else {
        let zone = match parameters.get("AffectedZone").map(String::as_str) {
            None | Some("Battlefield") => None,
            Some("Stack") => Some("stack"),
            Some("Library") => Some("library"),
            Some("Exile") => Some("exile"),
            Some("Graveyard") => Some("graveyard"),
            Some("Hand") => Some("hand"),
            Some("Command") => Some("command"),
            Some("All") => Some("all"),
            Some(value) => return Err(unsupported_value("AffectedZone", value)),
        };
        require_static_effect_zone(parameters, "EffectZone")?;
        zone
    };
    let affected = affected_selector(affected_value)?;
    let affected_player = affected_value == "You";
    if let Some(value) = parameters.get("MayLookAt") {
        let (viewer, visibility) = match value.as_str() {
            "You" => (call(Operation::You, vec![]), "private"),
            "Player" => (call(Operation::Any, vec![]), "public"),
            _ => return Err(unsupported_value("MayLookAt", value)),
        };
        effects.push(call(
            Operation::LookPermission,
            vec![
                call(Operation::Any, vec![]),
                viewer,
                Expression::Text(visibility.to_string()),
            ],
        ));
    }
    if let Some(value) = parameters.get("RemoveCardTypes") {
        if value != "True" || affected_player {
            return Err(unsupported_value("RemoveCardTypes", value));
        }
        effects.push(remove_all_card_types(call(Operation::Any, vec![])));
    }
    if let Some(value) = parameters.get("RemoveLandTypes") {
        if value != "True" || affected_player {
            return Err(unsupported_value("RemoveLandTypes", value));
        }
        effects.push(call(
            Operation::RemoveType,
            vec![
                call(Operation::Any, vec![]),
                Expression::Text("land_subtypes".to_string()),
            ],
        ));
    }
    if parameters.contains_key("AddPower") || parameters.contains_key("AddToughness") {
        if affected_player {
            return Err(unsupported_value("Affected", "You"));
        }
        let power = optional_signed_integer(parameters, "AddPower")?.unwrap_or(0);
        let toughness = optional_signed_integer(parameters, "AddToughness")?.unwrap_or(0);
        effects.push(call(
            Operation::ModifyPt,
            vec![
                call(Operation::Any, vec![]),
                Expression::Integer(power),
                Expression::Integer(toughness),
            ],
        ));
    }
    if let Some(keywords) = parameters.get("AddKeyword") {
        for keyword in keywords.split(" & ") {
            if is_next_untap_restriction(keyword) {
                effects.push(call(
                    Operation::CannotUntap,
                    vec![
                        call(Operation::Any, vec![]),
                        Expression::Text("next_untap_step".to_string()),
                    ],
                ));
            } else {
                effects.push(call(
                    Operation::GrantKeyword,
                    vec![
                        call(Operation::Any, vec![]),
                        Expression::Text(normalize_simple_keyword(keyword)?),
                    ],
                ));
            }
        }
    }
    if let Some(keywords) = parameters.get("AddHiddenKeyword") {
        if affected_player {
            return Err(unsupported_value("Affected", "You"));
        }
        for keyword in keywords.split(" & ").map(str::trim) {
            let affected = call(Operation::Any, vec![]);
            match keyword {
                "CARDNAME can't attack." => {
                    effects.push(call(Operation::CannotAttack, vec![affected]));
                }
                "CARDNAME can't block." => {
                    effects.push(call(Operation::CannotBlock, vec![affected]));
                }
                "CARDNAME can't attack or block." => {
                    effects.push(call(Operation::CannotAttack, vec![affected.clone()]));
                    effects.push(call(Operation::CannotBlock, vec![affected]));
                }
                "This card doesn't untap during your next untap step."
                | "CARDNAME doesn't untap during your next untap step." => {
                    effects.push(call(
                        Operation::CannotUntap,
                        vec![affected, Expression::Text("next_untap_step".to_string())],
                    ));
                }
                "CARDNAME must be blocked if able."
                | "All creatures able to block CARDNAME do so." => {
                    effects.push(call(Operation::MustBeBlocked, vec![affected]));
                }
                value => return Err(unsupported_value("AddHiddenKeyword", value)),
            }
        }
    }
    if let Some(keywords) = parameters.get("RemoveKeyword") {
        if affected_player {
            return Err(unsupported_value("Affected", "You"));
        }
        for keyword in keywords.split(" & ").map(str::trim) {
            effects.push(call(
                Operation::RemoveKeyword,
                vec![
                    call(Operation::Any, vec![]),
                    Expression::Text(normalize_simple_keyword(keyword)?),
                ],
            ));
        }
    }
    if let Some(keyword) = parameters.get("CantHaveKeyword") {
        if affected_player {
            return Err(unsupported_value("Affected", "You"));
        }
        effects.push(call(
            Operation::CannotHaveKeyword,
            vec![
                call(Operation::Any, vec![]),
                Expression::Text(normalize_simple_keyword(keyword)?),
            ],
        ));
    }
    if let Some(amount) = parameters.get("AdjustLandPlays") {
        if !affected_player {
            return Err(unsupported_value("Affected", affected_value));
        }
        let amount = positive_integer(amount, "AdjustLandPlays")?;
        effects.push(call(
            Operation::AdditionalLandPlays,
            vec![call(Operation::Any, vec![]), Expression::Integer(amount)],
        ));
    }
    if parameters.contains_key("SetPower") || parameters.contains_key("SetToughness") {
        if affected_player {
            return Err(unsupported_value("Affected", "You"));
        }
        let power = optional_number_or_value(parameters, "SetPower", Operation::Power)?;
        let toughness = optional_number_or_value(parameters, "SetToughness", Operation::Toughness)?;
        effects.push(call(
            Operation::SetPt,
            vec![call(Operation::Any, vec![]), power, toughness],
        ));
    }
    if let Some(value) = parameters.get("RemoveCreatureTypes") {
        if value != "True" || affected_player {
            return Err(unsupported_value("RemoveCreatureTypes", value));
        }
        effects.push(call(
            Operation::RemoveType,
            vec![
                call(Operation::Any, vec![]),
                Expression::Text("creature_subtypes".to_string()),
            ],
        ));
    }
    if let Some(types) = parameters.get("AddType") {
        if affected_player {
            return Err(unsupported_value("Affected", "You"));
        }
        append_text_effects(&mut effects, Operation::AddType, types, "AddType")?;
    }
    if let Some(types) = parameters.get("RemoveType") {
        if affected_player {
            return Err(unsupported_value("Affected", "You"));
        }
        append_text_effects(&mut effects, Operation::RemoveType, types, "RemoveType")?;
    }
    if let Some(colors) = parameters.get("SetColor") {
        if affected_player {
            return Err(unsupported_value("Affected", "You"));
        }
        let colors = parse_closed_colors(colors)?;
        let mut arguments = vec![call(Operation::Any, vec![])];
        arguments.extend(colors.into_iter().map(Expression::Text));
        effects.push(call(Operation::SetColor, arguments));
    }
    if let Some(value) = parameters.get("RemoveAllAbilities") {
        if value != "True" || affected_player {
            return Err(unsupported_value("RemoveAllAbilities", value));
        }
        effects.push(call(
            Operation::RemoveAllAbilities,
            vec![call(Operation::Any, vec![])],
        ));
    }
    if let Some(controller) = parameters.get("GainControl") {
        if affected_player || controller != "You" {
            return Err(unsupported_value("GainControl", controller));
        }
        effects.push(call(
            Operation::ChangeControl,
            vec![call(Operation::Any, vec![]), call(Operation::You, vec![])],
        ));
    }
    if let Some(maximum) = parameters.get("SetMaxHandSize") {
        if !affected_player || maximum != "Unlimited" {
            return Err(unsupported_value("SetMaxHandSize", maximum));
        }
        effects.push(call(
            Operation::NoMaximumHandSize,
            vec![call(Operation::You, vec![])],
        ));
    }
    let effect = match effects.len() {
        0 => {
            return Err(diagnostic(
                "MISSING_PARAMETER",
                "simple Continuous requires AddPower, AddToughness, or AddKeyword",
            ));
        }
        1 => effects.remove(0),
        _ => call(Operation::Sequence, effects),
    };
    let mut arguments = vec![affected, effect];
    if let Some(zone) = continuous_zone {
        arguments.push(Expression::Text(zone.to_string()));
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: call(Operation::Continuous, arguments),
    })
}

fn map_linked_continuous_traits(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    let parameter_map = parameters(expression)?;
    let mut effects = Vec::new();
    if let Some(ability_names) = parameter_map.get("AddAbility") {
        for name in ability_names.split(" & ").map(str::trim) {
            let linked_expression = context.svars.get(name).copied().ok_or_else(|| {
                diagnostic(
                    "MISSING_SVAR",
                    &format!("AddAbility SVar `{name}` is not declared"),
                )
            })?;
            if linked_expression
                .fields
                .first()
                .and_then(|field| field.key.as_deref())
                != Some("AB")
            {
                return Err(diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("AddAbility SVar `{name}` is not an activated ability"),
                ));
            }
            let linked = resolve_svar(name, context, stack)?;
            if linked.event.is_some() {
                return Err(diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("AddAbility SVar `{name}` unexpectedly contains an event"),
                ));
            }
            let mut arguments = vec![
                call(Operation::Any, vec![]),
                linked.expression,
                linked
                    .timing
                    .unwrap_or_else(|| call(Operation::TimingAll, vec![])),
            ];
            arguments.extend(linked.costs);
            effects.push(call(Operation::GrantActivatedAbility, arguments));
        }
    }
    if let Some(trigger_names) = parameter_map.get("AddTrigger") {
        for name in trigger_names.split(" & ").map(str::trim) {
            let linked = resolve_trigger_svar(name, context, stack)?;
            let event = linked.event.ok_or_else(|| {
                diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("AddTrigger SVar `{name}` has no typed event"),
                )
            })?;
            if !linked.costs.is_empty() || linked.timing.is_some() {
                return Err(diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("AddTrigger SVar `{name}` has an invalid cost or timing"),
                ));
            }
            effects.push(call(
                Operation::GrantTriggeredAbility,
                vec![call(Operation::Any, vec![]), event, linked.expression],
            ));
        }
    }
    if let Some(static_names) = parameter_map.get("AddStaticAbility") {
        for name in static_names.split(" & ").map(str::trim) {
            let linked = resolve_svar(name, context, stack)?;
            if linked.event.is_some() || linked.timing.is_some() || !linked.costs.is_empty() {
                return Err(diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("AddStaticAbility SVar `{name}` is not a cost-free static ability"),
                ));
            }
            effects.push(call(
                Operation::GrantStaticAbility,
                vec![call(Operation::Any, vec![]), linked.expression],
            ));
        }
    }
    if let Some(replacement_names) = parameter_map.get("AddReplacementEffect") {
        for name in replacement_names.split(" & ").map(str::trim) {
            let linked = resolve_replacement_svar(name, context, stack)?;
            let event = linked.event.ok_or_else(|| {
                diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("AddReplacementEffect SVar `{name}` has no typed event"),
                )
            })?;
            if linked.timing.is_some() || !linked.costs.is_empty() {
                return Err(diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("AddReplacementEffect SVar `{name}` has an invalid cost or timing"),
                ));
            }
            effects.push(call(
                Operation::GrantReplacementAbility,
                vec![call(Operation::Any, vec![]), event, linked.expression],
            ));
        }
    }
    if let Some(names) = parameter_map.get("AddSVar") {
        for name in names.split(" & ").map(str::trim) {
            let linked = context.svars.get(name).copied().ok_or_else(|| {
                diagnostic(
                    "MISSING_SVAR",
                    &format!("AddSVar dependency `{name}` is not declared"),
                )
            })?;
            if name.is_empty() {
                return Err(diagnostic(
                    "MISSING_SVAR",
                    &format!("AddSVar dependency `{name}` is not declared"),
                ));
            }
            effects.push(call(
                Operation::GrantSVar,
                vec![
                    call(Operation::Any, vec![]),
                    Expression::Text(name.to_string()),
                    Expression::Text(linked.raw.clone()),
                ],
            ));
        }
    }
    if effects.is_empty() {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "linked Continuous requires a granted ability or SVar",
        ));
    }
    let mut base = expression.clone();
    base.fields.retain(|field| {
        !matches!(
            field.key.as_deref(),
            Some(
                "AddAbility"
                    | "AddTrigger"
                    | "AddStaticAbility"
                    | "AddReplacementEffect"
                    | "AddSVar"
            )
        )
    });
    let base_parameters = parameters(&base)?;
    map_continuous_with_effects(prefix, "Continuous", selector, &base_parameters, effects)
}

fn resolve_trigger_svar(
    name: &str,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    if context.duplicate_svars.contains(name) {
        return Err(diagnostic(
            "DUPLICATE_SVAR",
            &format!("SVar `{name}` is declared more than once"),
        ));
    }
    if stack.iter().any(|active| active == name) {
        return Err(diagnostic(
            "CYCLIC_SVAR",
            &format!("SVar cycle reaches `{name}`"),
        ));
    }
    let expression = context.svars.get(name).copied().ok_or_else(|| {
        diagnostic(
            "MISSING_SVAR",
            &format!("AddTrigger SVar `{name}` is not declared"),
        )
    })?;
    if expression
        .fields
        .first()
        .and_then(|field| field.key.as_deref())
        != Some("Mode")
    {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("AddTrigger SVar `{name}` is not a trigger"),
        ));
    }
    stack.push(name.to_string());
    let result = map_with_context(LegacyAbilityPrefix::Triggered, expression, context, stack);
    stack.pop();
    result
}

fn resolve_replacement_svar(
    name: &str,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    if context.duplicate_svars.contains(name) {
        return Err(diagnostic(
            "DUPLICATE_SVAR",
            &format!("SVar `{name}` is declared more than once"),
        ));
    }
    if stack.iter().any(|active| active == name) {
        return Err(diagnostic(
            "CYCLIC_SVAR",
            &format!("SVar cycle reaches `{name}`"),
        ));
    }
    let expression = context.svars.get(name).copied().ok_or_else(|| {
        diagnostic(
            "MISSING_SVAR",
            &format!("ReplacementEffects SVar `{name}` is not declared"),
        )
    })?;
    if expression
        .fields
        .first()
        .and_then(|field| field.key.as_deref())
        != Some("Event")
    {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("ReplacementEffects SVar `{name}` is not a replacement effect"),
        ));
    }
    stack.push(name.to_string());
    let result = map_with_context(LegacyAbilityPrefix::Replacement, expression, context, stack);
    stack.pop();
    result
}

fn map_change_zone(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "DefinedPlayer",
            "ValidTgts",
            "TgtZone",
            "TgtPrompt",
            "Origin",
            "Destination",
            "ChangeType",
            "ChangeTypeDesc",
            "ChangeNum",
            "Tapped",
            "Reveal",
            "Shuffle",
            "ShuffleNonMandatory",
            "LibraryPosition",
            "Mandatory",
            "GainControl",
            "NoLooking",
            "Hidden",
            "SelectPrompt",
            "Chooser",
            "RememberChanged",
            "Duration",
            "AtEOT",
            "WithCountersType",
            "WithCountersAmount",
            "AttachedTo",
            "SpellDescription",
            "StackDescription",
            "IsCurse",
            "AILogic",
        ],
    )?;
    let origin = required(parameters, "Origin")?;
    if let Some(zones) = parameters.get("TgtZone") {
        let normalized = normalize_zone_list("TgtZone", zones)?;
        if normalized.len() != 1 || normalized[0] != origin.to_ascii_lowercase() {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "ChangeZone TgtZone must exactly match Origin",
            ));
        }
    }
    if let Some(chooser) = parameters.get("Chooser") {
        if chooser != "You" {
            return Err(unsupported_value("Chooser", chooser));
        }
    }
    let replacement_object = parameters
        .get("Defined")
        .is_some_and(|value| value == "ReplacedCard");
    let defined_object_bound = parameters.get("Defined").is_some_and(|value| {
        matches!(
            value.as_str(),
            "Self"
                | "EffectSource"
                | "OriginalHost"
                | "TriggeredCard"
                | "TriggeredCardLKICopy"
                | "TriggeredNewCardLKICopy"
                | "ReplacedCard"
                | "Remembered"
                | "RememberedCard"
                | "RememberedLKI"
                | "DelayTriggerRememberedLKI"
                | "ChosenCard"
                | "ParentTarget"
                | "TriggeredTarget"
                | "TriggeredTargetLKICopy"
        )
    }) && !parameters.contains_key("ValidTgts");
    let player_bound = parameters.contains_key("DefinedPlayer");
    if player_bound && parameters.contains_key("Defined") {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "DefinedPlayer cannot be combined with Defined in a closed zone move",
        ));
    }
    if parameters
        .get("DefinedPlayer")
        .is_some_and(|value| matches!(value.as_str(), "Player" | "Opponent" | "Player.Opponent"))
    {
        return Err(diagnostic(
            "UNSUPPORTED_VALUE",
            "aggregate DefinedPlayer requires per-player selection cardinality",
        ));
    }
    let source_bound = !parameters.contains_key("Defined")
        && !parameters.contains_key("ValidTgts")
        && !player_bound;
    if let Some(value) = parameters.get("Hidden") {
        if value != "True" {
            return Err(unsupported_value("Hidden", value));
        }
    }
    if origin == "Library" && !parameters.contains_key("Defined") {
        return map_library_search(prefix, api, parameters);
    }
    let closed_origin = matches!(
        origin,
        "Library" | "Graveyard" | "Hand" | "Exile" | "Stack" | "Command"
    );
    if origin == "Command"
        && parameters
            .get("Defined")
            .is_some_and(|value| value != "Self")
    {
        return Err(unsupported_value("Origin", origin));
    }
    let zone_targeted = closed_origin
        && parameters.contains_key("ValidTgts")
        && !parameters.contains_key("Defined");
    if !(origin == "Battlefield"
        || zone_targeted
        || origin == "All" && replacement_object
        || closed_origin && (defined_object_bound || source_bound || player_bound)
        || origin == "Battlefield" && player_bound)
    {
        return Err(unsupported_value("Origin", origin));
    }
    let affected = if player_bound {
        let cards = card_selector_in_zone(
            required(parameters, "ChangeType")?,
            &origin.to_ascii_lowercase(),
        )?;
        let relation = if origin == "Battlefield" {
            Operation::ControlledBy
        } else {
            Operation::OwnedBy
        };
        add_collection_predicate(
            cards,
            call(relation, vec![zone_owner_selector(parameters)?]),
        )?
    } else if zone_targeted {
        call(
            Operation::Target,
            vec![card_selector_in_zone(
                required(parameters, "ValidTgts")?,
                &origin.to_ascii_lowercase(),
            )?],
        )
    } else {
        object_selector(parameters, DefaultSelector::Source)?
    };
    let destination = match required(parameters, "Destination")? {
        "Graveyard" => "graveyard",
        "Exile" => "exile",
        "Hand" => "hand",
        "Battlefield" => "battlefield",
        "Library" => match parameters
            .get("LibraryPosition")
            .map(String::as_str)
            .unwrap_or("0")
        {
            "0" => "library_top",
            "-1" => "library_bottom",
            value => return Err(unsupported_value("LibraryPosition", value)),
        },
        value => return Err(unsupported_value("Destination", value)),
    };
    let gain_control = match parameters.get("GainControl").map(String::as_str) {
        None => false,
        Some("True") if destination == "battlefield" => true,
        Some("True") => {
            return Err(unsupported_value(
                "GainControl",
                required(parameters, "GainControl")?,
            ))
        }
        Some(value) => return Err(unsupported_value("GainControl", value)),
    };
    let control_target = affected.clone();
    let expression = if player_bound {
        let amount = optional_positive_integer(parameters, "ChangeNum")?.unwrap_or(1);
        call(
            Operation::MoveZone,
            vec![
                affected,
                Expression::Text(destination.to_string()),
                Expression::Integer(amount),
            ],
        )
    } else if closed_origin {
        call(
            Operation::MoveZoneFrom,
            vec![
                affected,
                Expression::Text(origin.to_ascii_lowercase()),
                Expression::Text(destination.to_string()),
            ],
        )
    } else {
        match destination {
            "exile" => call(Operation::Exile, vec![affected]),
            "hand" => call(Operation::ReturnToHand, vec![affected]),
            _ => call(
                Operation::MoveZone,
                vec![affected, Expression::Text(destination.to_string())],
            ),
        }
    };
    let expression = if gain_control {
        call(
            Operation::Sequence,
            vec![
                expression,
                call(
                    Operation::ChangeControl,
                    vec![control_target, call(Operation::You, vec![])],
                ),
            ],
        )
    } else {
        expression
    };
    let expression = apply_created_attachment(expression, parameters)?;
    let expression = apply_entry_counters(expression, parameters)?;
    let expression = apply_changed_object_metadata(expression, parameters)?;
    let expression = apply_zone_move_lifetime(expression, parameters, origin, destination)?;
    mapped_direct(
        prefix,
        api,
        parameters,
        preserve_hidden_information(parameters, expression),
    )
}

fn map_library_search(
    prefix: LegacyAbilityPrefix,
    api: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    if let Some(no_looking) = parameters.get("NoLooking") {
        if no_looking != "True" {
            return Err(unsupported_value("NoLooking", no_looking));
        }
        let change_type = required(parameters, "ChangeType")?;
        if !change_type.contains(".IsRemembered")
            || parameters.contains_key("Defined")
            || parameters.contains_key("DefinedPlayer")
            || parameters.contains_key("ValidTgts")
        {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "NoLooking requires a closed remembered library selector",
            ));
        }
        if parameters.get("Mandatory").map(String::as_str) != Some("True")
            || parameters
                .get("Shuffle")
                .is_some_and(|value| value != "False")
        {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "NoLooking remembered selection requires Mandatory True and Shuffle False",
            ));
        }
        let amount = optional_positive_integer(parameters, "ChangeNum")?.unwrap_or(1);
        let candidates = card_selector_in_zone(change_type, "library")?;
        let chosen = call(Operation::EffectResult, vec![]);
        let destination = match required(parameters, "Destination")? {
            "Battlefield" => "battlefield",
            "Hand" => "hand",
            "Graveyard" => "graveyard",
            "Exile" => "exile",
            value => return Err(unsupported_value("Destination", value)),
        };
        let choose = call(
            Operation::ChooseObjects,
            vec![
                candidates,
                Expression::Integer(amount),
                call(Operation::You, vec![]),
                Expression::Text("exact".to_string()),
            ],
        );
        let mut move_effect = call(
            Operation::MoveZone,
            vec![chosen.clone(), Expression::Text(destination.to_string())],
        );
        if parameters.get("Tapped").map(String::as_str) == Some("True") {
            move_effect = call(
                Operation::Sequence,
                vec![move_effect, call(Operation::Tap, vec![chosen])],
            );
        } else if let Some(value) = parameters.get("Tapped") {
            return Err(unsupported_value("Tapped", value));
        }
        return mapped_direct(
            prefix,
            api,
            parameters,
            call(Operation::Sequence, vec![choose, move_effect]),
        );
    }
    if let Some(value) = parameters.get("Tapped") {
        if value != "True" {
            return Err(unsupported_value("Tapped", value));
        }
    }
    if let Some(value) = parameters.get("Reveal") {
        if value != "True" {
            return Err(unsupported_value("Reveal", value));
        }
    }
    if let Some(value) = parameters.get("Shuffle") {
        if value != "False" && value != "True" {
            return Err(unsupported_value("Shuffle", value));
        }
    }
    if let Some(value) = parameters.get("ShuffleNonMandatory") {
        if value != "True" {
            return Err(unsupported_value("ShuffleNonMandatory", value));
        }
    }
    if let Some(value) = parameters.get("Mandatory") {
        if value != "True" {
            return Err(unsupported_value("Mandatory", value));
        }
    }
    let amount = optional_positive_integer(parameters, "ChangeNum")?.unwrap_or(1);
    let player = zone_owner_selector(parameters)?;
    let cards = card_selector_in_zone(required(parameters, "ChangeType")?, "library")?;
    let chosen = call(Operation::Chosen, vec![cards.clone()]);
    let mut effects = vec![call(
        Operation::SearchLibrary,
        vec![cards, player.clone(), Expression::Integer(amount)],
    )];
    if parameters.contains_key("Reveal") {
        effects.push(call(Operation::Reveal, vec![chosen.clone()]));
    }
    let destination = required(parameters, "Destination")?.to_ascii_lowercase();
    if !matches!(
        destination.as_str(),
        "battlefield" | "graveyard" | "hand" | "exile" | "library"
    ) {
        return Err(unsupported_value(
            "Destination",
            required(parameters, "Destination")?,
        ));
    }
    let should_shuffle = parameters.get("Shuffle").map(String::as_str) != Some("False");
    let destination = if destination == "library" {
        match parameters
            .get("LibraryPosition")
            .map(String::as_str)
            .unwrap_or("0")
        {
            "0" => "library_top".to_string(),
            "-1" => "library_bottom".to_string(),
            value => return Err(unsupported_value("LibraryPosition", value)),
        }
    } else {
        if parameters.contains_key("LibraryPosition") {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "LibraryPosition is only valid for a library destination",
            ));
        }
        destination
    };
    if should_shuffle && destination.starts_with("library_") {
        effects.push(call(Operation::Shuffle, vec![player.clone()]));
    }
    effects.push(call(
        Operation::MoveZone,
        vec![
            chosen.clone(),
            Expression::Text(destination.clone()),
            Expression::Integer(amount),
        ],
    ));
    if let Some(value) = parameters.get("RememberChanged") {
        if value != "True" {
            return Err(unsupported_value("RememberChanged", value));
        }
        effects.push(call(
            Operation::Remember,
            vec![Expression::Text("changed".to_string()), chosen.clone()],
        ));
    }
    if parameters.contains_key("Tapped") {
        effects.push(call(Operation::Tap, vec![chosen]));
    }
    if should_shuffle && !destination.starts_with("library_") {
        effects.push(call(Operation::Shuffle, vec![player]));
    }
    let expression = combine_effects(effects, "library search requires effects")?;
    mapped_direct(
        prefix,
        api,
        parameters,
        preserve_hidden_information(parameters, expression),
    )
}

fn apply_changed_object_metadata(
    move_effect: Expression,
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    let Some(value) = parameters.get("RememberChanged") else {
        return Ok(move_effect);
    };
    if value != "True" {
        return Err(unsupported_value("RememberChanged", value));
    }
    Ok(call(
        Operation::Sequence,
        vec![
            move_effect,
            call(
                Operation::Remember,
                vec![
                    Expression::Text("changed".to_string()),
                    call(Operation::EffectResult, vec![]),
                ],
            ),
        ],
    ))
}

fn preserve_hidden_information(
    parameters: &BTreeMap<String, String>,
    expression: Expression,
) -> Expression {
    if parameters.contains_key("Hidden") {
        call(Operation::HiddenInformation, vec![expression])
    } else {
        expression
    }
}

fn zone_owner_selector(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    match parameters.get("DefinedPlayer").map(String::as_str) {
        Some("Targeted" | "TargetedPlayer") if parameters.contains_key("ValidTgts") => {
            targeted_player_selector(required(parameters, "ValidTgts")?, "ValidTgts")
        }
        Some("Targeted" | "TargetedPlayer") => Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            "targeted DefinedPlayer requires ValidTgts in the same ability",
        )),
        Some(value) => defined_player_selector(value),
        None => Ok(call(Operation::You, vec![])),
    }
}

fn map_token(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "TokenScript",
            "TokenOwner",
            "ValidTgts",
            "TokenAmount",
            "TokenTapped",
            "TokenAttacking",
            "AttachedTo",
            "RememberTokens",
            "AtEOT",
            "WithCountersType",
            "WithCountersAmount",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let token_scripts = required(parameters, "TokenScript")?
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    if token_scripts.is_empty() || token_scripts.iter().any(|token| token.is_empty()) {
        return Err(unsupported_value(
            "TokenScript",
            required(parameters, "TokenScript")?,
        ));
    }
    let amount = optional_positive_integer(parameters, "TokenAmount")?.unwrap_or(1);
    let tapped = match parameters.get("TokenTapped").map(String::as_str) {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("TokenTapped", value)),
    };
    let owner = match (
        parameters.get("TokenOwner").map(String::as_str),
        parameters.get("ValidTgts"),
    ) {
        (Some("You"), _) => call(Operation::You, vec![]),
        (None | Some("Targeted" | "TargetedPlayer"), Some(value)) => {
            targeted_player_selector(value, "ValidTgts")?
        }
        (Some(value), None) => defined_player_selector(value)?,
        (None, None) => call(Operation::You, vec![]),
        (Some(value), Some(_)) => return Err(unsupported_value("TokenOwner", value)),
    };
    let expression = combine_effects(
        token_scripts
            .into_iter()
            .map(|token| {
                let mut arguments = vec![
                    Expression::Text(token.to_string()),
                    Expression::Integer(amount),
                    owner.clone(),
                ];
                if tapped {
                    arguments.push(Expression::Text("tapped".to_string()));
                }
                let created =
                    apply_token_attacking(call(Operation::CreateToken, arguments), parameters)?;
                let created = apply_created_attachment(created, parameters)?;
                let created = apply_entry_counters(created, parameters)?;
                apply_created_object_metadata(created, parameters)
            })
            .collect::<Result<Vec<_>, _>>()?,
        "Token requires at least one TokenScript",
    )?;
    mapped_direct(prefix, api, parameters, expression)
}

fn apply_entry_counters(
    create_or_move: Expression,
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    let Some(counter_types) = parameters.get("WithCountersType") else {
        if parameters.contains_key("WithCountersAmount") {
            return Err(diagnostic(
                "MISSING_PARAMETER",
                "WithCountersAmount requires WithCountersType",
            ));
        }
        return Ok(create_or_move);
    };
    let amount = optional_positive_integer(parameters, "WithCountersAmount")?.unwrap_or(1);
    let mut effects = vec![create_or_move];
    for counter_type in counter_types.split(',').map(str::trim) {
        if counter_type.is_empty() {
            return Err(unsupported_value("WithCountersType", counter_types));
        }
        effects.push(call(
            Operation::AddCounter,
            vec![
                call(Operation::EffectResult, vec![]),
                Expression::Text(counter_type.to_ascii_lowercase()),
                Expression::Integer(amount),
            ],
        ));
    }
    combine_effects(effects, "entry counters require a move or creation")
}

fn apply_created_object_metadata(
    create_effect: Expression,
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    let result = call(Operation::EffectResult, vec![]);
    let mut effects = vec![create_effect];
    if let Some(value) = parameters.get("RememberTokens") {
        if value != "True" {
            return Err(unsupported_value("RememberTokens", value));
        }
        effects.push(call(
            Operation::Remember,
            vec![Expression::Text("tokens".to_string()), result.clone()],
        ));
    }
    if let Some(value) = parameters.get("AtEOT") {
        effects.push(map_at_eot_cleanup(value, &result)?);
    }
    combine_effects(effects, "created-object effect requires creation")
}

fn apply_token_attacking(
    create_effect: Expression,
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    let Some(value) = parameters.get("TokenAttacking") else {
        return Ok(create_effect);
    };
    let defender = match value.as_str() {
        "True" | "TriggeredAttackedTarget" => call(Operation::TriggeredDefendingPlayer, vec![]),
        "Remembered" => call(Operation::Remembered, vec![call(Operation::Any, vec![])]),
        _ => return Err(unsupported_value("TokenAttacking", value)),
    };
    Ok(call(
        Operation::PutCreatedAttacking,
        vec![create_effect, defender],
    ))
}

fn apply_created_attachment(
    create_effect: Expression,
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    let Some(value) = parameters.get("AttachedTo") else {
        return Ok(create_effect);
    };
    let target = defined_selector(value).or_else(|_| affected_selector(value))?;
    Ok(call(Operation::AttachCreated, vec![create_effect, target]))
}

fn remove_all_card_types(affected: Expression) -> Expression {
    call(
        Operation::Sequence,
        [
            "artifact",
            "creature",
            "enchantment",
            "instant",
            "land",
            "planeswalker",
            "sorcery",
        ]
        .into_iter()
        .map(|card_type| {
            call(
                Operation::RemoveType,
                vec![affected.clone(), Expression::Text(card_type.to_string())],
            )
        })
        .collect(),
    )
}

fn map_destroy_all(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "ValidCards",
            "ValidTgts",
            "NoRegen",
            "RememberDestroyed",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let affected = scope_collection_to_target_player(
        valid_cards_selector(required(parameters, "ValidCards")?)?,
        parameters,
        Operation::ControlledBy,
    )?;
    let mut arguments = vec![affected];
    if let Some(value) = parameters.get("NoRegen") {
        if value != "True" {
            return Err(unsupported_value("NoRegen", value));
        }
        arguments.push(Expression::Text("cannot_regenerate".to_string()));
    }
    let expression = apply_remembered_result(
        call(Operation::Destroy, arguments),
        parameters,
        "RememberDestroyed",
        "destroyed",
    )?;
    mapped_direct(prefix, api, parameters, expression)
}

fn map_damage_all(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "ValidCards",
            "ValidPlayers",
            "ValidTgts",
            "ValidDescription",
            "NumDmg",
            "DamageSource",
            "RememberDamaged",
            "ReplaceDyingDefined",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let mut affected = Vec::new();
    if let Some(cards) = parameters.get("ValidCards") {
        affected.push(scope_collection_to_target_player(
            valid_cards_selector(cards)?,
            parameters,
            Operation::ControlledBy,
        )?);
    } else if parameters.contains_key("ValidTgts") {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "target-player DamageAll requires ValidCards",
        ));
    }
    if let Some(players) = parameters.get("ValidPlayers") {
        affected.push(match players.as_str() {
            "You" => call(Operation::You, vec![]),
            "Opponent" | "Player.Opponent" => call(Operation::Opponent, vec![]),
            "Player" | "Any" => call(Operation::Any, vec![]),
            _ => return Err(unsupported_value("ValidPlayers", players)),
        });
    }
    let target = match affected.len() {
        0 => {
            return Err(diagnostic(
                "MISSING_PARAMETER",
                "DamageAll requires ValidCards or ValidPlayers",
            ));
        }
        1 => affected.remove(0),
        _ => call(Operation::All, affected),
    };
    let amount = positive_integer(required(parameters, "NumDmg")?, "NumDmg")?;
    let mut arguments = vec![target, Expression::Integer(amount)];
    if let Some(source) = parameters.get("DamageSource") {
        arguments.push(damage_source_selector(source)?);
    }
    let expression = call(Operation::DealDamage, arguments);
    let expression = apply_replace_dying(expression, parameters)?;
    let expression = apply_remembered_result(expression, parameters, "RememberDamaged", "damaged")?;
    mapped_direct(prefix, api, parameters, expression)
}

fn apply_replace_dying(
    damage: Expression,
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    let Some(value) = parameters.get("ReplaceDyingDefined") else {
        return Ok(damage);
    };
    let binding = value.strip_suffix(".Creature").unwrap_or(value);
    Ok(call(
        Operation::ExileIfDies,
        vec![damage, defined_selector(binding)?],
    ))
}

fn damage_source_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    match value {
        "Targeted" | "ParentTarget" | "ThisTargetedCard" => {
            Ok(call(Operation::Target, vec![call(Operation::Any, vec![])]))
        }
        "Self" | "EffectSource" | "OriginalHost" => Ok(call(Operation::Source, vec![])),
        "TriggeredCard" | "TriggeredCardLKICopy" | "TriggeredSource" => {
            Ok(call(Operation::Triggered, vec![]))
        }
        "TriggeredAttacker" | "TriggeredAttackerLKICopy" => {
            Ok(call(Operation::TriggeredAttacker, vec![]))
        }
        "TriggeredTarget" | "TriggeredTargetLKICopy" => {
            Ok(call(Operation::TriggeredTarget, vec![]))
        }
        "Remembered" => Ok(call(
            Operation::Remembered,
            vec![call(Operation::Any, vec![])],
        )),
        "Enchanted" => Ok(call(
            Operation::EnchantedObject,
            vec![call(Operation::Source, vec![])],
        )),
        "Any" => Ok(call(Operation::Any, vec![])),
        _ => Err(unsupported_value("DamageSource", value)),
    }
}

fn map_discard(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "Mode",
            "NumCards",
            "SpellDescription",
            "StackDescription",
            "AILogic",
            "RememberDiscarded",
        ],
    )?;
    let mode = match required(parameters, "Mode")? {
        "TgtChoose" | "Choose" => "choose",
        "Random" => "random",
        "Hand" => "hand",
        value => return Err(unsupported_value("Mode", value)),
    };
    let amount = optional_positive_integer(parameters, "NumCards")?.unwrap_or(1);
    let player = player_selector(parameters, DefaultSelector::You)?;
    let expression = call(
        Operation::DiscardCards,
        vec![
            Expression::Integer(amount),
            player,
            Expression::Text(mode.to_string()),
        ],
    );
    let expression =
        apply_remembered_result(expression, parameters, "RememberDiscarded", "discarded")?;
    mapped_direct(prefix, api, parameters, expression)
}

fn map_counter_spell(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "TargetType",
            "ValidTgts",
            "TgtPrompt",
            "Destination",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    if required(parameters, "TargetType")? != "Spell" {
        return Err(unsupported_value(
            "TargetType",
            required(parameters, "TargetType")?,
        ));
    }
    if parameters
        .get("Destination")
        .is_some_and(|destination| destination != "Graveyard")
    {
        return Err(unsupported_value(
            "Destination",
            required(parameters, "Destination")?,
        ));
    }
    let spells = spell_selector(required(parameters, "ValidTgts")?)?;
    let expression = call(
        Operation::CounterSpell,
        vec![call(Operation::Target, vec![spells])],
    );
    mapped_direct(prefix, api, parameters, expression)
}

fn map_scry(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    map_self_number_effect(
        prefix,
        api,
        selector,
        parameters,
        "ScryNum",
        Operation::Scry,
    )
}

fn map_surveil(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    map_self_number_effect(
        prefix,
        api,
        selector,
        parameters,
        "Amount",
        Operation::Surveil,
    )
}

fn map_self_number_effect(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
    amount_key: &str,
    operation: Operation,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            amount_key,
            "Defined",
            "ValidTgts",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let amount = positive_integer(required(parameters, amount_key)?, amount_key)?;
    let expression = call(
        operation,
        vec![
            Expression::Integer(amount),
            player_selector(parameters, DefaultSelector::You)?,
        ],
    );
    mapped_direct(prefix, api, parameters, expression)
}

fn map_reduce_cost(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    reject_unknown(
        parameters,
        &[
            "Type",
            "ValidCard",
            "ValidSpell",
            "ValidTarget",
            "Activator",
            "Amount",
            "OnlyFirstSpell",
            "EffectZone",
            "Description",
        ],
    )?;
    if required(parameters, "Type")? != "Spell" {
        return Err(unsupported_value("Type", required(parameters, "Type")?));
    }
    require_static_effect_zone(parameters, "EffectZone")?;
    let amount = positive_integer(required(parameters, "Amount")?, "Amount")?;
    let mut spells = reduce_cost_spell_selector(parameters)?;
    if let Some(target) = parameters.get("ValidTarget") {
        spells = add_collection_predicate(
            spells,
            call(Operation::Targets, vec![affected_selector(target)?]),
        )?;
    }
    if let Some(activator) = parameters.get("Activator") {
        let player = match activator.as_str() {
            "You" => call(Operation::You, vec![]),
            "Opponent" | "Player.Opponent" => call(Operation::Opponent, vec![]),
            "Player" | "Any" => call(Operation::Any, vec![]),
            _ => return Err(unsupported_value("Activator", activator)),
        };
        spells = add_collection_predicate(spells, call(Operation::ControlledBy, vec![player]))?;
    }
    let first_spell_condition = match parameters.get("OnlyFirstSpell").map(String::as_str) {
        None => None,
        Some("True") => Some(call(
            Operation::Equals,
            vec![
                call(
                    Operation::HistoryCount,
                    vec![
                        spells.clone(),
                        Expression::Text("cast_this_turn".to_string()),
                    ],
                ),
                Expression::Integer(0),
            ],
        )),
        Some(value) => return Err(unsupported_value("OnlyFirstSpell", value)),
    };
    let expression = call(
        Operation::Continuous,
        vec![
            spells,
            call(
                Operation::CostReduction,
                vec![call(Operation::Any, vec![]), Expression::Integer(amount)],
            ),
        ],
    );
    let expression = if let Some(condition) = first_spell_condition {
        call(Operation::WhileCondition, vec![condition, expression])
    } else {
        expression
    };
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression,
    })
}

fn map_raise_cost(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    reject_unknown(
        parameters,
        &[
            "Type",
            "ValidCard",
            "ValidSpell",
            "ValidTarget",
            "Activator",
            "Amount",
            "EffectZone",
            "Description",
        ],
    )?;
    if required(parameters, "Type")? != "Spell" {
        return Err(unsupported_value("Type", required(parameters, "Type")?));
    }
    require_static_effect_zone(parameters, "EffectZone")?;
    let amount = positive_integer(required(parameters, "Amount")?, "Amount")?;
    let mut spells = reduce_cost_spell_selector(parameters)?;
    if let Some(target) = parameters.get("ValidTarget") {
        spells = add_collection_predicate(
            spells,
            call(Operation::Targets, vec![affected_selector(target)?]),
        )?;
    }
    if let Some(activator) = parameters.get("Activator") {
        let player = match activator.as_str() {
            "You" => call(Operation::You, vec![]),
            "Opponent" | "Player.Opponent" => call(Operation::Opponent, vec![]),
            "Player" | "Any" => call(Operation::Any, vec![]),
            value => return Err(unsupported_value("Activator", value)),
        };
        spells = add_collection_predicate(spells, call(Operation::ControlledBy, vec![player]))?;
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: call(
            Operation::Continuous,
            vec![
                spells,
                call(
                    Operation::CostIncrease,
                    vec![call(Operation::Any, vec![]), Expression::Integer(amount)],
                ),
            ],
        ),
    })
}

fn reduce_cost_spell_selector(
    parameters: &BTreeMap<String, String>,
) -> Result<Expression, MappingDiagnostic> {
    match (parameters.get("ValidCard"), parameters.get("ValidSpell")) {
        (Some(value), None) => spell_selector(value),
        (None, Some(value)) => closed_valid_spell_selector(value),
        (Some(_), Some(value)) => Err(unsupported_value("ValidSpell", value)),
        (None, None) => Ok(call(Operation::Spells, vec![])),
    }
}

fn closed_valid_spell_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    let mut branches = Vec::new();
    for branch in value.split(',') {
        let branch = branch.trim();
        let Some(kind) = branch.strip_prefix("Spell.") else {
            return Err(unsupported_value("ValidSpell", value));
        };
        match kind {
            "Instant" | "Sorcery" => branches.push(kind),
            _ => return Err(unsupported_value("ValidSpell", value)),
        }
    }
    if branches.is_empty() {
        return Err(unsupported_value("ValidSpell", value));
    }
    spell_selector(&branches.join(",")).map_err(|mut error| {
        error.message = error.message.replace("`ValidCard`", "`ValidSpell`");
        error
    })
}

fn map_cant_block_by(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    reject_unknown(
        parameters,
        &[
            "ValidAttacker",
            "ValidBlocker",
            "Description",
            "Secondary",
            "EffectZone",
        ],
    )?;
    require_static_effect_zone(parameters, "EffectZone")?;
    if parameters
        .get("Secondary")
        .is_some_and(|value| value != "True")
    {
        return Err(unsupported_value(
            "Secondary",
            required(parameters, "Secondary")?,
        ));
    }
    let attacker = affected_selector(required(parameters, "ValidAttacker")?)?;
    let blockers = parameters
        .get("ValidBlocker")
        .map(|value| affected_selector(value))
        .transpose()?
        .unwrap_or_else(|| {
            call(
                Operation::Permanents,
                vec![call(
                    Operation::TypeIs,
                    vec![Expression::Text("creature".to_string())],
                )],
            )
        });
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: call(
            Operation::Continuous,
            vec![
                attacker,
                call(
                    Operation::CannotBeBlockedBy,
                    vec![call(Operation::Any, vec![]), blockers],
                ),
            ],
        ),
    })
}

fn map_change_zone_all(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "ChangeType",
            "Origin",
            "Destination",
            "ValidTgts",
            "GainControl",
            "SpellDescription",
            "StackDescription",
            "ValidDescription",
            "AILogic",
            "IsCurse",
            "Duration",
            "AtEOT",
            "RememberChanged",
        ],
    )?;
    let origin = required(parameters, "Origin")?;
    let affected = match origin {
        "Battlefield" => scope_collection_to_target_player(
            valid_cards_selector(required(parameters, "ChangeType")?)?,
            parameters,
            Operation::ControlledBy,
        )?,
        "Graveyard" | "Hand" | "Exile" => scope_collection_to_target_player(
            card_selector_in_zone(
                required(parameters, "ChangeType")?,
                &origin.to_ascii_lowercase(),
            )?,
            parameters,
            Operation::OwnedBy,
        )?,
        value => return Err(unsupported_value("Origin", value)),
    };
    let destination = required(parameters, "Destination")?;
    let control_target = affected.clone();
    let expression = match destination {
        "Battlefield" => call(
            Operation::MoveZone,
            vec![affected, Expression::Text("battlefield".to_string())],
        ),
        "Graveyard" => call(
            Operation::MoveZone,
            vec![affected, Expression::Text("graveyard".to_string())],
        ),
        "Exile" => call(Operation::Exile, vec![affected]),
        "Hand" => call(Operation::ReturnToHand, vec![affected]),
        value => return Err(unsupported_value("Destination", value)),
    };
    let gain_control = match parameters.get("GainControl").map(String::as_str) {
        None => false,
        Some("True") if destination == "Battlefield" => true,
        Some("True") => {
            return Err(unsupported_value(
                "GainControl",
                required(parameters, "GainControl")?,
            ));
        }
        Some(value) => return Err(unsupported_value("GainControl", value)),
    };
    let expression = if gain_control {
        call(
            Operation::Sequence,
            vec![
                expression,
                call(
                    Operation::ChangeControl,
                    vec![control_target, call(Operation::You, vec![])],
                ),
            ],
        )
    } else {
        expression
    };
    let expression = apply_changed_object_metadata(expression, parameters)?;
    let expression = apply_zone_move_lifetime(expression, parameters, origin, destination)?;
    mapped_direct(prefix, api, parameters, expression)
}

fn apply_zone_move_lifetime(
    move_effect: Expression,
    parameters: &BTreeMap<String, String>,
    origin: &str,
    destination: &str,
) -> Result<Expression, MappingDiagnostic> {
    let mut effects = vec![move_effect];
    match parameters.get("Duration").map(String::as_str) {
        None => {}
        Some("UntilHostLeavesPlay")
            if origin == "Battlefield" && destination.eq_ignore_ascii_case("exile") =>
        {
            effects.push(call(
                Operation::RegisterDelayedTrigger,
                vec![
                    call(
                        Operation::EventLeaves,
                        vec![call(Operation::Source, vec![])],
                    ),
                    call(
                        Operation::MoveZoneFrom,
                        vec![
                            call(Operation::EffectResult, vec![]),
                            Expression::Text("exile".to_string()),
                            Expression::Text("battlefield".to_string()),
                        ],
                    ),
                ],
            ));
        }
        Some(value) => return Err(unsupported_value("Duration", value)),
    }
    if let Some(value) = parameters.get("AtEOT") {
        effects.push(map_at_eot_cleanup(
            value,
            &call(Operation::EffectResult, vec![]),
        )?);
    }
    combine_effects(effects, "zone move requires an effect")
}

fn map_linked_static_effect(
    prefix: LegacyAbilityPrefix,
    api: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    let parameters = parameters(expression)?;
    let names = required(&parameters, "StaticAbilities")?
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    let mut linked = Vec::new();
    for name in &names {
        let mapped = resolve_svar(name, context, stack)?;
        if mapped.event.is_some() || mapped.timing.is_some() || !mapped.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("StaticAbilities `{name}` is not a cost-free static ability"),
            ));
        }
        linked.push(mapped.expression);
    }
    if linked.len() == 1 && expression_has_operation(&linked[0], Operation::PlayExiled) {
        return map_play_permission_effect(prefix, api, expression, context, stack);
    }
    reject_unknown(
        &parameters,
        &[
            "Cost",
            "StaticAbilities",
            "RememberObjects",
            "ForgetOnMoved",
            "ExileOnMoved",
            "Duration",
            "EffectOwner",
            "ValidTgts",
            "Defined",
            "SubAbility",
            "Name",
            "Image",
            "Planeswalker",
            "Ultimate",
            "Stackable",
            "IsCurse",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    for (key, expected) in [
        ("Planeswalker", "True"),
        ("Ultimate", "True"),
        ("Stackable", "False"),
    ] {
        if parameters.get(key).is_some_and(|value| value != expected) {
            return Err(unsupported_value(key, required(&parameters, key)?));
        }
    }
    let owner = match parameters.get("EffectOwner").map(String::as_str) {
        None | Some("You") => call(Operation::You, vec![]),
        Some("Remembered" | "Player.IsRemembered") => {
            call(Operation::Remembered, vec![call(Operation::Any, vec![])])
        }
        Some("Targeted" | "TargetedPlayer") if parameters.contains_key("ValidTgts") => {
            call(Operation::Target, vec![call(Operation::Any, vec![])])
        }
        Some(value) => return Err(unsupported_value("EffectOwner", value)),
    };
    let remembered = effect_remembered_selector(&parameters)?;
    let cleanup = effect_cleanup_policy(&parameters, remembered.is_some())?;
    let duration = match parameters.get("Duration").map(String::as_str) {
        None | Some("EndOfTurn") | Some("UntilEndOfTurn") => "until_end_of_turn",
        Some("Permanent") => "permanent",
        Some("UntilYourNextTurn") => "until_your_next_turn",
        Some("UntilHostLeavesPlay") => "until_host_leaves_play",
        Some("UntilHostLeavesPlayOrEOT") => "until_host_leaves_play_or_eot",
        Some("UntilTheEndOfYourNextTurn") => "until_end_of_your_next_turn",
        Some(value) => return Err(unsupported_value("Duration", value)),
    };
    let mut effects = Vec::new();
    for static_effect in linked {
        let mut arguments = vec![
            owner.clone(),
            static_effect,
            Expression::Text(duration.to_string()),
            Expression::Boolean(remembered.is_some()),
            Expression::Text(cleanup.clone()),
        ];
        if let Some(objects) = remembered.clone() {
            arguments.push(objects);
        }
        effects.push(call(Operation::RegisterEffectStatic, arguments));
    }
    let mut effect = combine_effects(effects, "StaticAbilities must reference a static effect")?;
    if let Some(tail_name) = parameters.get("SubAbility") {
        let tail = resolve_svar(tail_name, context, stack)?;
        if tail.event.is_some() || tail.timing.is_some() || !tail.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{tail_name}` is not a cost-free effect chain"),
            ));
        }
        effect = sequence(effect, tail.expression);
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: effect,
    })
}

fn map_play_permission_effect(
    prefix: LegacyAbilityPrefix,
    api: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "StaticAbilities",
            "RememberObjects",
            "ForgetOnMoved",
            "ExileOnMoved",
            "Duration",
            "SubAbility",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let static_name = required(&parameters, "StaticAbilities")?;
    let linked = resolve_svar(static_name, context, stack)?;
    if linked.event.is_some()
        || linked.timing.is_some()
        || !linked.costs.is_empty()
        || !expression_has_operation(&linked.expression, Operation::PlayExiled)
    {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("StaticAbilities `{static_name}` is not a closed play permission"),
        ));
    }
    let objects = match parameters.get("RememberObjects").map(String::as_str) {
        Some("Remembered" | "RememberedCard") => {
            call(Operation::Remembered, vec![call(Operation::Any, vec![])])
        }
        None => call(Operation::Remembered, vec![call(Operation::Any, vec![])]),
        Some(value) => return Err(unsupported_value("RememberObjects", value)),
    };
    let post_move = match (
        parameters.get("ForgetOnMoved").map(String::as_str),
        parameters.get("ExileOnMoved").map(String::as_str),
    ) {
        (None, None) => "none",
        (Some("Exile"), None) => "forget",
        (None, Some("Exile")) => "exile",
        (Some(_), Some(_)) => {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "ForgetOnMoved and ExileOnMoved cannot be combined",
            ));
        }
        (Some(value), None) => return Err(unsupported_value("ForgetOnMoved", value)),
        (None, Some(value)) => return Err(unsupported_value("ExileOnMoved", value)),
    };
    let duration = match parameters.get("Duration").map(String::as_str) {
        None | Some("EndOfTurn") => "until_end_of_turn",
        Some("Permanent") => "permanent",
        Some("UntilYourNextTurn") => "until_your_next_turn",
        Some("UntilYourNextEndStep") => "until_your_next_end_step",
        Some("UntilTheEndOfYourNextTurn") => "until_end_of_your_next_turn",
        Some(value) => return Err(unsupported_value("Duration", value)),
    };
    let mut effect = call(
        Operation::PlayPermission,
        vec![
            objects,
            Expression::Text("exile".to_string()),
            call(Operation::You, vec![]),
            Expression::Text(duration.to_string()),
            Expression::Text(post_move.to_string()),
        ],
    );
    if let Some(tail_name) = parameters.get("SubAbility") {
        let tail = resolve_svar(tail_name, context, stack)?;
        if tail.event.is_some() || tail.timing.is_some() || !tail.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{tail_name}` is not a cost-free effect chain"),
            ));
        }
        effect = sequence(effect, tail.expression);
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: effect,
    })
}

fn expression_has_operation(expression: &Expression, expected: Operation) -> bool {
    match expression {
        Expression::Call {
            operation,
            arguments,
        } => {
            *operation == expected
                || arguments
                    .iter()
                    .any(|argument| expression_has_operation(argument, expected))
        }
        _ => false,
    }
}

fn map_trigger_effect(
    prefix: LegacyAbilityPrefix,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    let parameters = parameters(expression)?;
    reject_unknown(
        &parameters,
        &[
            "Cost",
            "Triggers",
            "ReplacementEffects",
            "StaticAbilities",
            "RememberObjects",
            "ExileOnMoved",
            "ForgetOnMoved",
            "Duration",
            "EffectOwner",
            "ValidTgts",
            "Defined",
            "SubAbility",
            "OneOff",
            "Name",
            "Image",
            "Planeswalker",
            "Ultimate",
            "Stackable",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    for (key, expected) in [
        ("Planeswalker", "True"),
        ("Ultimate", "True"),
        ("Stackable", "False"),
    ] {
        if parameters.get(key).is_some_and(|value| value != expected) {
            return Err(unsupported_value(key, required(&parameters, key)?));
        }
    }
    let owner = match parameters.get("EffectOwner").map(String::as_str) {
        None | Some("You") => call(Operation::You, vec![]),
        Some("Remembered" | "Player.IsRemembered") => {
            call(Operation::Remembered, vec![call(Operation::Any, vec![])])
        }
        Some("Targeted" | "TargetedPlayer") if parameters.contains_key("ValidTgts") => {
            call(Operation::Target, vec![call(Operation::Any, vec![])])
        }
        Some(value) => return Err(unsupported_value("EffectOwner", value)),
    };
    let remembered = effect_remembered_selector(&parameters)?;
    let cleanup = effect_cleanup_policy(&parameters, remembered.is_some())?;
    let duration = match (
        parameters.get("OneOff").map(String::as_str),
        parameters.get("Duration").map(String::as_str),
    ) {
        (Some("True"), None) => "one_shot",
        (Some(value), None) => return Err(unsupported_value("OneOff", value)),
        (Some(_), Some(_)) => {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "OneOff cannot be combined with Duration",
            ));
        }
        (None, None | Some("EndOfTurn") | Some("UntilEndOfTurn")) => "until_end_of_turn",
        (None, Some("Permanent")) => "permanent",
        (None, Some("UntilYourNextTurn")) => "until_your_next_turn",
        (None, Some("UntilTheEndOfYourNextTurn")) => "until_end_of_your_next_turn",
        (None, Some(value)) => return Err(unsupported_value("Duration", value)),
    };
    let mut effects = Vec::new();
    if let Some(names) = parameters.get("Triggers") {
        for name in names.split(',').map(str::trim) {
            let linked = resolve_trigger_svar(name, context, stack)?;
            let event = linked.event.ok_or_else(|| {
                diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("Triggers SVar `{name}` has no typed event"),
                )
            })?;
            if !linked.costs.is_empty() || linked.timing.is_some() {
                return Err(diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("Triggers SVar `{name}` has an invalid cost or timing"),
                ));
            }
            let mut arguments = vec![
                owner.clone(),
                event,
                linked.expression,
                Expression::Text(duration.to_string()),
                Expression::Boolean(remembered.is_some()),
                Expression::Text(cleanup.clone()),
            ];
            if let Some(objects) = remembered.clone() {
                arguments.push(objects);
            }
            effects.push(call(Operation::RegisterEffectTrigger, arguments));
        }
    }
    if let Some(names) = parameters.get("ReplacementEffects") {
        for name in names.split(',').map(str::trim) {
            let linked = resolve_replacement_svar(name, context, stack)?;
            let event = linked.event.ok_or_else(|| {
                diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("ReplacementEffects SVar `{name}` has no typed event"),
                )
            })?;
            if !linked.costs.is_empty() || linked.timing.is_some() {
                return Err(diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("ReplacementEffects SVar `{name}` has an invalid cost or timing"),
                ));
            }
            let mut arguments = vec![
                owner.clone(),
                event,
                linked.expression,
                Expression::Text(duration.to_string()),
                Expression::Boolean(remembered.is_some()),
                Expression::Text(cleanup.clone()),
            ];
            if let Some(objects) = remembered.clone() {
                arguments.push(objects);
            }
            effects.push(call(Operation::RegisterEffectReplacement, arguments));
        }
    }
    if let Some(names) = parameters.get("StaticAbilities") {
        for name in names.split(',').map(str::trim) {
            let linked = resolve_svar(name, context, stack)?;
            if linked.event.is_some() || linked.timing.is_some() || !linked.costs.is_empty() {
                return Err(diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("StaticAbilities SVar `{name}` is not a cost-free static ability"),
                ));
            }
            let mut arguments = vec![
                owner.clone(),
                linked.expression,
                Expression::Text(duration.to_string()),
                Expression::Boolean(remembered.is_some()),
                Expression::Text(cleanup.clone()),
            ];
            if let Some(objects) = remembered.clone() {
                arguments.push(objects);
            }
            effects.push(call(Operation::RegisterEffectStatic, arguments));
        }
    }
    let mut effect = combine_effects(
        effects,
        "Effect must reference at least one triggered, replacement, or static effect",
    )?;
    if let Some(tail_name) = parameters.get("SubAbility") {
        let tail = resolve_svar(tail_name, context, stack)?;
        if tail.event.is_some() || tail.timing.is_some() || !tail.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{tail_name}` is not a cost-free effect chain"),
            ));
        }
        effect = sequence(effect, tail.expression);
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: "Effect".to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: effect,
    })
}

fn effect_remembered_selector(
    parameters: &BTreeMap<String, String>,
) -> Result<Option<Expression>, MappingDiagnostic> {
    let Some(value) = parameters.get("RememberObjects") else {
        return Ok(None);
    };
    let mut objects = Vec::new();
    for binding in value.split(" & ").map(str::trim) {
        objects.push(match binding {
            "Targeted" | "ThisTargetedCard" => {
                call(Operation::Target, vec![call(Operation::Any, vec![])])
            }
            "Self" => call(Operation::Source, vec![]),
            "ParentTarget" => call(Operation::ParentTarget, vec![]),
            "ChosenCard" => call(Operation::Chosen, vec![call(Operation::Any, vec![])]),
            "Remembered" | "RememberedCard" => {
                call(Operation::Remembered, vec![call(Operation::Any, vec![])])
            }
            "TriggeredCard" | "TriggeredCardLKICopy" => call(Operation::Triggered, vec![]),
            "Equipped" => call(
                Operation::EquippedObject,
                vec![call(Operation::Source, vec![])],
            ),
            "Enchanted" => call(
                Operation::EnchantedObject,
                vec![call(Operation::Source, vec![])],
            ),
            _ => return Err(unsupported_value("RememberObjects", value)),
        });
    }
    Ok(match objects.len() {
        0 => return Err(unsupported_value("RememberObjects", value)),
        1 => objects.pop(),
        _ => Some(call(Operation::All, objects)),
    })
}

fn effect_cleanup_policy(
    parameters: &BTreeMap<String, String>,
    has_remembered: bool,
) -> Result<String, MappingDiagnostic> {
    let policy = match (
        parameters.get("ForgetOnMoved").map(String::as_str),
        parameters.get("ExileOnMoved").map(String::as_str),
    ) {
        (None, None) => return Ok("none".to_string()),
        (Some(_), Some(_)) => {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "ForgetOnMoved and ExileOnMoved cannot be combined",
            ));
        }
        (Some(value), None) => ("forget", value),
        (None, Some(value)) => ("exile", value),
    };
    if !has_remembered {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "effect cleanup requires RememberObjects",
        ));
    }
    let zones = if policy.1 == "True" {
        "any".to_string()
    } else {
        let mut normalized = Vec::new();
        for zone in policy.1.split(',').map(str::trim) {
            if !matches!(
                zone,
                "Battlefield" | "Graveyard" | "Exile" | "Hand" | "Stack"
            ) {
                return Err(unsupported_value(
                    if policy.0 == "forget" {
                        "ForgetOnMoved"
                    } else {
                        "ExileOnMoved"
                    },
                    policy.1,
                ));
            }
            normalized.push(zone.to_ascii_lowercase());
        }
        normalized.join(",")
    };
    Ok(format!("{}_on_moved:{zones}", policy.0))
}

fn map_effect(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "StaticAbilities",
            "RememberObjects",
            "ExileOnMoved",
            "ForgetOnMoved",
            "Duration",
        ],
    )?;
    let static_ability = required(parameters, "StaticAbilities")?;
    if !matches!(static_ability, "Unblockable" | "MustAttack") {
        return Err(unsupported_value("StaticAbilities", static_ability));
    }
    if let Some(value) = parameters.get("Defined") {
        if value != "Self" {
            return Err(unsupported_value("Defined", value));
        }
    }
    if let Some(value) = parameters.get("RememberObjects") {
        if !matches!(
            value.as_str(),
            "Targeted" | "Self" | "Equipped" | "Enchanted" | "Remembered"
        ) {
            return Err(unsupported_value("RememberObjects", value));
        }
        if value == "Targeted" && !parameters.contains_key("ValidTgts") {
            return Err(diagnostic(
                "MISSING_PARAMETER",
                "RememberObjects Targeted requires ValidTgts",
            ));
        }
    }
    for key in ["ExileOnMoved", "ForgetOnMoved"] {
        if let Some(value) = parameters.get(key) {
            if value != "Battlefield" {
                return Err(unsupported_value(key, value));
            }
        }
    }
    let affected = if parameters.contains_key("ValidTgts") || parameters.contains_key("Defined") {
        object_selector(parameters, DefaultSelector::Source)?
    } else {
        match parameters.get("RememberObjects").map(String::as_str) {
            Some("Self") => call(Operation::Source, vec![]),
            Some("Equipped") => call(
                Operation::EquippedObject,
                vec![call(Operation::Source, vec![])],
            ),
            Some("Enchanted") => call(
                Operation::EnchantedObject,
                vec![call(Operation::Source, vec![])],
            ),
            Some("Remembered") => call(Operation::Remembered, vec![call(Operation::Any, vec![])]),
            _ => {
                return Err(diagnostic(
                    "MISSING_PARAMETER",
                    "Effect requires an explicit target, source, attached object, or remembered binding",
                ));
            }
        }
    };
    if static_ability == "MustAttack" {
        let duration = match parameters.get("Duration").map(String::as_str) {
            None | Some("EndOfTurn") | Some("UntilEndOfTurn") => "until_end_of_turn",
            Some("UntilEndOfCombat") => "until_end_of_combat",
            Some("UntilYourNextTurn") => "until_your_next_turn",
            Some(value) => return Err(unsupported_value("Duration", value)),
        };
        return mapped_direct(
            prefix,
            api,
            parameters,
            call(
                Operation::MustAttack,
                vec![affected, Expression::Text(duration.to_string())],
            ),
        );
    }
    match parameters.get("Duration").map(String::as_str) {
        None | Some("EndOfTurn") | Some("UntilEndOfTurn") => {}
        Some(value) => return Err(unsupported_value("Duration", value)),
    }
    let blockers = call(
        Operation::Permanents,
        vec![call(
            Operation::TypeIs,
            vec![Expression::Text("creature".to_string())],
        )],
    );
    let expression = call(
        Operation::UntilEndOfTurn,
        vec![call(
            Operation::Continuous,
            vec![
                affected,
                call(
                    Operation::CannotBeBlockedBy,
                    vec![call(Operation::Any, vec![]), blockers],
                ),
            ],
        )],
    );
    mapped_direct(prefix, api, parameters, expression)
}

fn map_animate(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    map_animate_with_effects(prefix, api, selector, parameters, Vec::new())
}

fn map_animate_with_effects(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
    mut effects: Vec<Expression>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtZone",
            "Power",
            "Toughness",
            "Types",
            "RemoveCardTypes",
            "RemoveCreatureTypes",
            "RemoveLandTypes",
            "Colors",
            "OverwriteColors",
            "Keywords",
            "RemoveAllAbilities",
            "Duration",
            "AtEOT",
            "RememberObjects",
        ],
    )?;
    let affected = object_selector(parameters, DefaultSelector::Source)?;
    if let Some(value) = parameters.get("RemoveCardTypes") {
        if value != "True" {
            return Err(unsupported_value("RemoveCardTypes", value));
        }
        effects.push(remove_all_card_types(affected.clone()));
    }
    if let Some(value) = parameters.get("RemoveLandTypes") {
        if value != "True" {
            return Err(unsupported_value("RemoveLandTypes", value));
        }
        effects.push(call(
            Operation::RemoveType,
            vec![
                affected.clone(),
                Expression::Text("land_subtypes".to_string()),
            ],
        ));
    }
    if parameters.contains_key("Power") || parameters.contains_key("Toughness") {
        let power = optional_number_or_value(parameters, "Power", Operation::Power)?;
        let toughness = optional_number_or_value(parameters, "Toughness", Operation::Toughness)?;
        effects.push(call(
            Operation::SetPt,
            vec![affected.clone(), power, toughness],
        ));
    }
    if let Some(value) = parameters.get("RemoveCreatureTypes") {
        if value != "True" {
            return Err(unsupported_value("RemoveCreatureTypes", value));
        }
        effects.push(call(
            Operation::RemoveType,
            vec![
                affected.clone(),
                Expression::Text("creature_subtypes".to_string()),
            ],
        ));
    }
    if let Some(types) = parameters.get("Types") {
        for card_type in types.split(',').map(str::trim) {
            if card_type.is_empty() {
                return Err(unsupported_value("Types", types));
            }
            effects.push(call(
                Operation::AddType,
                vec![affected.clone(), Expression::Text(card_type.to_string())],
            ));
        }
    }
    if let Some(colors) = parameters.get("Colors") {
        if parameters.get("OverwriteColors").map(String::as_str) != Some("True") {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "Animate Colors requires OverwriteColors$ True",
            ));
        }
        let colors = parse_animate_colors(colors)?;
        let mut arguments = vec![affected.clone()];
        arguments.extend(colors.into_iter().map(Expression::Text));
        effects.push(call(Operation::SetColor, arguments));
    } else if parameters.contains_key("OverwriteColors") {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "OverwriteColors requires Colors",
        ));
    }
    if let Some(keywords) = parameters.get("Keywords") {
        for keyword in keywords.split(" & ") {
            effects.push(call(
                Operation::GrantKeyword,
                vec![
                    affected.clone(),
                    Expression::Text(normalize_simple_keyword(keyword)?),
                ],
            ));
        }
    }
    if let Some(value) = parameters.get("RemoveAllAbilities") {
        if value != "True" {
            return Err(unsupported_value("RemoveAllAbilities", value));
        }
        effects.push(call(Operation::RemoveAllAbilities, vec![affected.clone()]));
    }
    if let Some(value) = parameters.get("RememberObjects") {
        effects.push(call(
            Operation::RememberOn,
            vec![affected.clone(), remember_objects_selector(value)?],
        ));
    }
    let mut expression = combine_effects(effects, "simple Animate has no typed changes")?;
    match parameters.get("Duration").map(String::as_str) {
        None | Some("EOT") | Some("EndOfTurn") | Some("UntilEndOfTurn") => {
            expression = call(Operation::UntilEndOfTurn, vec![expression]);
        }
        Some("Permanent") => {}
        Some("Perpetual") => {
            expression = call(Operation::Perpetual, vec![expression]);
        }
        Some(value) => return Err(unsupported_value("Duration", value)),
    }
    if let Some(value) = parameters.get("AtEOT") {
        expression = call(
            Operation::Sequence,
            vec![expression, map_at_eot_cleanup(value, &affected)?],
        );
    }
    mapped_direct(prefix, api, parameters, expression)
}

fn map_animated_linked_traits(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    let parameter_map = parameters(expression)?;
    let sub_ability = parameter_map.get("SubAbility").cloned();
    let affected = match api {
        "Animate" => object_selector(&parameter_map, DefaultSelector::Source)?,
        "AnimateAll" => scope_collection_to_target_player(
            affected_selector(required(&parameter_map, "ValidCards")?)?,
            &parameter_map,
            Operation::ControlledBy,
        )?,
        _ => return Err(unsupported_value("API", api)),
    };
    let mut effects = Vec::new();
    if let Some(names) = parameter_map.get("Triggers") {
        for name in names.split(',').map(str::trim) {
            let linked = resolve_trigger_svar(name, context, stack)?;
            let event = linked.event.ok_or_else(|| {
                diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("Triggers SVar `{name}` has no typed event"),
                )
            })?;
            if !linked.costs.is_empty() || linked.timing.is_some() {
                return Err(diagnostic(
                    "UNSUPPORTED_LINK",
                    &format!("Triggers SVar `{name}` has an invalid cost or timing"),
                ));
            }
            effects.push(call(
                Operation::GrantTriggeredAbility,
                vec![affected.clone(), event, linked.expression],
            ));
        }
    }
    for key in ["staticAbilities", "StaticAbilities"] {
        if let Some(names) = parameter_map.get(key) {
            for name in names.split([',', '&']).map(str::trim) {
                let linked = resolve_svar(name, context, stack)?;
                if linked.event.is_some() || linked.timing.is_some() || !linked.costs.is_empty() {
                    return Err(diagnostic(
                        "UNSUPPORTED_LINK",
                        &format!("{key} SVar `{name}` is not a cost-free static ability"),
                    ));
                }
                effects.push(call(
                    Operation::GrantStaticAbility,
                    vec![affected.clone(), linked.expression],
                ));
            }
        }
    }
    for key in ["AddSVar", "sVars", "SVars"] {
        if let Some(names) = parameter_map.get(key) {
            for name in names.split([',', '&']).map(str::trim) {
                if name.is_empty() || !context.svars.contains_key(name) {
                    return Err(diagnostic(
                        "MISSING_SVAR",
                        &format!("{key} dependency `{name}` is not declared"),
                    ));
                }
            }
        }
    }
    let mut base = expression.clone();
    base.fields.retain(|field| {
        !matches!(
            field.key.as_deref(),
            Some(
                "Triggers"
                    | "staticAbilities"
                    | "StaticAbilities"
                    | "AddSVar"
                    | "sVars"
                    | "SVars"
                    | "SubAbility"
            )
        )
    });
    let base_parameters = parameters(&base)?;
    let mut mapped = match api {
        "Animate" => map_animate_with_effects(prefix, api, selector, &base_parameters, effects),
        "AnimateAll" => {
            map_animate_all_with_effects(prefix, api, selector, &base_parameters, effects)
        }
        _ => Err(unsupported_value("API", api)),
    }?;
    if let Some(name) = sub_ability {
        let linked = resolve_svar(&name, context, stack)?;
        if linked.event.is_some() || linked.timing.is_some() || !linked.costs.is_empty() {
            return Err(diagnostic(
                "UNSUPPORTED_LINK",
                &format!("SubAbility `{name}` is not a cost-free effect chain"),
            ));
        }
        mapped.expression = sequence(mapped.expression, linked.expression);
    }
    Ok(mapped)
}

fn map_set_state(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "Mode",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let mode = required(parameters, "Mode")?;
    if mode != "Transform" {
        return Err(unsupported_value("Mode", mode));
    }
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::Transform,
            vec![object_selector(parameters, DefaultSelector::Source)?],
        ),
    )
}

fn map_alternative_cost(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    reject_unknown(
        parameters,
        &[
            "ValidSA",
            "ValidCard",
            "ValidPlayer",
            "Cost",
            "EffectZone",
            "Description",
        ],
    )?;
    if parameters
        .get("EffectZone")
        .is_some_and(|zone| zone != "All")
    {
        return Err(unsupported_value(
            "EffectZone",
            required(parameters, "EffectZone")?,
        ));
    }
    let valid_sa = required(parameters, "ValidSA")?;
    let mut spells = match valid_sa {
        "Spell.Self" => {
            if parameters.contains_key("ValidCard") {
                return Err(diagnostic(
                    "UNSUPPORTED_SELECTOR",
                    "self-spell alternative cost also supplies ValidCard",
                ));
            }
            spell_selector("Card.Self")?
        }
        "Spell" => parameters
            .get("ValidCard")
            .map(|value| spell_selector(value))
            .transpose()?
            .unwrap_or_else(|| call(Operation::Spells, vec![])),
        value => return Err(unsupported_value("ValidSA", value)),
    };
    if let Some(value) = parameters.get("ValidPlayer") {
        spells = add_collection_predicate(
            spells,
            call(
                Operation::ControlledBy,
                vec![draw_player_selector(value, "ValidPlayer")?],
            ),
        )?;
    }
    let costs = parse_simple_cost(parameters.get("Cost"))?;
    if costs.is_empty() {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "AlternativeCost requires a typed non-empty cost",
        ));
    }
    let mut arguments = vec![spells];
    arguments.extend(costs);
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: call(Operation::AlternateCost, arguments),
    })
}

fn map_sacrifice_effect(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "SacValid",
            "Amount",
            "SpellDescription",
            "StackDescription",
            "RememberSacrificed",
        ],
    )?;
    if optional_positive_integer(parameters, "Amount")?.unwrap_or(1) != 1 {
        return Err(unsupported_value("Amount", required(parameters, "Amount")?));
    }
    if parameters.contains_key("Defined") && parameters.contains_key("ValidTgts") {
        return Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            "Sacrifice has both Defined and ValidTgts players",
        ));
    }
    let player = if let Some(value) = parameters.get("ValidTgts") {
        call(
            Operation::Target,
            vec![draw_player_selector(value, "ValidTgts")?],
        )
    } else if let Some(value) = parameters.get("Defined") {
        draw_player_selector(value, "Defined")?
    } else {
        call(Operation::You, vec![])
    };
    let permanents = match parameters.get("SacValid") {
        Some(value) => sacrifice_selector(value)?,
        None if !parameters.contains_key("Defined") && !parameters.contains_key("ValidTgts") => {
            source_permanent_collection()
        }
        None => sacrifice_selector(required(parameters, "SacValid")?)?,
    };
    let permanents =
        add_collection_predicate(permanents, call(Operation::ControlledBy, vec![player]))?;
    let expression = apply_remembered_result(
        call(Operation::SacrificeEffect, vec![permanents]),
        parameters,
        "RememberSacrificed",
        "sacrificed",
    )?;
    mapped_direct(prefix, api, parameters, expression)
}

fn map_sacrifice_all(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "ValidCards",
            "Controller",
            "RememberSacrificed",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    if parameters.contains_key("Defined") && parameters.contains_key("ValidCards") {
        return Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            "SacrificeAll cannot combine Defined and ValidCards",
        ));
    }
    let mut affected = if parameters.contains_key("Defined") || parameters.contains_key("ValidTgts")
    {
        object_selector(parameters, DefaultSelector::Source)?
    } else if let Some(valid) = parameters.get("ValidCards") {
        affected_selector(valid)?
    } else {
        call(Operation::Permanents, vec![])
    };
    if let Some(controller) = parameters.get("Controller") {
        affected = match affected {
            Expression::Call {
                operation: Operation::Cards | Operation::Permanents,
                ..
            } => add_collection_predicate(
                affected,
                call(
                    Operation::ControlledBy,
                    vec![defined_player_selector(controller)?],
                ),
            )?,
            single if controller == "You" => single,
            _ => return Err(unsupported_value("Controller", controller)),
        };
    }
    let expression = apply_remembered_result(
        call(Operation::SacrificeEffect, vec![affected]),
        parameters,
        "RememberSacrificed",
        "sacrificed",
    )?;
    mapped_direct(prefix, api, parameters, expression)
}

fn apply_remembered_result(
    effect: Expression,
    parameters: &BTreeMap<String, String>,
    key: &str,
    domain: &str,
) -> Result<Expression, MappingDiagnostic> {
    let Some(value) = parameters.get(key) else {
        return Ok(effect);
    };
    if value != "True" {
        return Err(unsupported_value(key, value));
    }
    Ok(call(
        Operation::Sequence,
        vec![
            effect,
            call(
                Operation::Remember,
                vec![
                    Expression::Text(domain.to_string()),
                    call(Operation::EffectResult, vec![]),
                ],
            ),
        ],
    ))
}

fn sacrifice_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    if matches!(value, "Self" | "Card.Self" | "Creature.Self") {
        return Ok(source_permanent_collection());
    }
    affected_selector(value)
}

fn source_permanent_collection() -> Expression {
    call(
        Operation::Permanents,
        vec![call(
            Operation::Equals,
            vec![
                call(Operation::Any, vec![]),
                call(Operation::Source, vec![]),
            ],
        )],
    )
}

fn map_gain_control(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "NewController",
            "LoseControl",
            "Untap",
            "AddKWs",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    if parameters
        .get("NewController")
        .is_some_and(|controller| controller != "You")
    {
        return Err(unsupported_value(
            "NewController",
            required(parameters, "NewController")?,
        ));
    }
    let affected = object_selector(parameters, DefaultSelector::Source)?;
    let temporary = match parameters.get("LoseControl").map(String::as_str) {
        None => false,
        Some("EOT") => true,
        Some(value) => return Err(unsupported_value("LoseControl", value)),
    };
    let untap = match parameters.get("Untap").map(String::as_str) {
        None => false,
        Some("True") => true,
        Some(value) => return Err(unsupported_value("Untap", value)),
    };
    let mut lasting = vec![call(
        Operation::ChangeControl,
        vec![affected.clone(), call(Operation::You, vec![])],
    )];
    if let Some(keywords) = parameters.get("AddKWs") {
        for keyword in keywords.split(" & ").map(str::trim) {
            lasting.push(call(
                Operation::GrantKeyword,
                vec![
                    affected.clone(),
                    Expression::Text(normalize_simple_keyword(keyword)?),
                    Expression::Text("until_end_of_turn".to_string()),
                ],
            ));
        }
    }
    let lasting = combine_effects(lasting, "GainControl requires an effect")?;
    let lasting = if temporary {
        call(Operation::UntilEndOfTurn, vec![lasting])
    } else if parameters.contains_key("AddKWs") {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "GainControl AddKWs requires LoseControl EOT",
        ));
    } else {
        lasting
    };
    let expression = if untap {
        call(
            Operation::Sequence,
            vec![call(Operation::Untap, vec![affected]), lasting],
        )
    } else {
        lasting
    };
    mapped_direct(prefix, api, parameters, expression)
}

fn map_prevent_damage(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "Amount",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let amount = positive_integer(required(parameters, "Amount")?, "Amount")?;
    let target = object_selector(parameters, DefaultSelector::Source)?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::PreventDamage,
            vec![
                call(Operation::Any, vec![]),
                target,
                Expression::Integer(amount),
            ],
        ),
    )
}

fn map_protection(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "Gains",
            "Choices",
            "Duration",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let gains = required(parameters, "Gains")?;
    let choices = parameters.get("Choices");
    if gains == "Choice" && choices.is_none() {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "Gains Choice requires closed protection Choices",
        ));
    }
    if gains != "Choice" && choices.is_some() {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "Choices is only valid when Gains is Choice",
        ));
    }
    let duration = match parameters.get("Duration").map(String::as_str) {
        None | Some("UntilEndOfTurn") => "until_end_of_turn",
        Some("Permanent") => "permanent",
        Some(value) => return Err(unsupported_value("Duration", value)),
    };
    let mut arguments = vec![object_selector(parameters, DefaultSelector::Source)?];
    arguments.push(Expression::Text(format!("gains={gains}")));
    if let Some(choices) = choices {
        arguments.push(Expression::Text(format!("choices={choices}")));
    }
    arguments.push(Expression::Text(format!("duration={duration}")));
    mapped_direct(
        prefix,
        api,
        parameters,
        call(Operation::GrantProtection, arguments),
    )
}

fn map_choose_type(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "Type",
            "ValidTypes",
            "InvalidTypes",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let choice_domain = match required(parameters, "Type")? {
        "Card" => "card",
        "Creature" => "creature",
        "Basic Land" => "basic_land",
        "Nonbasic Land" => "nonbasic_land",
        "Land" => "land",
        "Planeswalker" => "planeswalker",
        value => return Err(unsupported_value("Type", value)),
    };
    let valid_types = parameters.get("ValidTypes").map(String::as_str);
    if valid_types.is_some_and(|value| {
        !matches!(
            value,
            "Land,Nonland"
                | "Elemental,Elf,Faerie,Giant,Goblin,Kithkin,Merfolk,Treefolk"
                | "Human,Merfolk,Goblin"
                | "Creature,Land"
                | "Artifact,Enchantment,Instant,Sorcery,Planeswalker"
                | "Artifact,Creature,Land"
                | "Artifact,Creature,Enchantment,Instant,Sorcery"
        )
    }) {
        return Err(unsupported_value(
            "ValidTypes",
            required(parameters, "ValidTypes")?,
        ));
    }
    let invalid_types = parameters.get("InvalidTypes").map(String::as_str);
    if invalid_types.is_some_and(|value| {
        !matches!(
            value,
            "Wall"
                | "Mountain,Forest,Plains"
                | "Instant,Sorcery,Kindred"
                | "Creature,Land"
                | "Creature"
        )
    }) {
        return Err(unsupported_value(
            "InvalidTypes",
            required(parameters, "InvalidTypes")?,
        ));
    }
    let mut arguments = vec![
        player_selector(parameters, DefaultSelector::You)?,
        Expression::Text(format!("domain={choice_domain}")),
        Expression::Text("storage=chosen_type".to_string()),
    ];
    if let Some(value) = valid_types {
        arguments.push(Expression::Text(format!("valid={value}")));
    }
    if let Some(value) = invalid_types {
        arguments.push(Expression::Text(format!("invalid={value}")));
    }
    mapped_direct(
        prefix,
        api,
        parameters,
        call(Operation::ChooseType, arguments),
    )
}

fn map_choose_color(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "Exclude",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    if let Some(excluded) = parameters.get("Exclude") {
        if !matches!(
            excluded.as_str(),
            "white" | "blue" | "black" | "red" | "green"
        ) {
            return Err(unsupported_value("Exclude", excluded));
        }
    }
    let mut arguments = vec![
        player_selector(parameters, DefaultSelector::You)?,
        Expression::Text("domain=color".to_string()),
        Expression::Text("storage=chosen_color".to_string()),
    ];
    if let Some(excluded) = parameters.get("Exclude") {
        arguments.push(Expression::Text(format!("exclude={excluded}")));
    }
    mapped_direct(
        prefix,
        api,
        parameters,
        call(Operation::ChooseType, arguments),
    )
}

fn map_fog(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &["Cost", "SpellDescription", "StackDescription", "AILogic"],
    )?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(Operation::PreventAllCombatDamage, vec![]),
    )
}

fn map_fight(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "ValidTgtsDesc",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let first = defined_selector(required(parameters, "Defined")?)?;
    let second = valid_target_selector(required(parameters, "ValidTgts")?)?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(Operation::Fight, vec![first, second]),
    )
}

fn map_explore_or_connive(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    let (operation, amount_key) = match api {
        "Explore" => (Operation::Explore, "Num"),
        "Connive" => (Operation::Connive, "ConniveNum"),
        _ => return Err(diagnostic("UNMAPPED_API", "unknown explore-like effect")),
    };
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "ValidTgtsDesc",
            amount_key,
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let affected = object_selector(parameters, DefaultSelector::Source)?;
    let amount = optional_positive_integer(parameters, amount_key)?.unwrap_or(1);
    mapped_direct(
        prefix,
        api,
        parameters,
        call(operation, vec![affected, Expression::Integer(amount)]),
    )
}

fn map_put_counter_all(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "ValidCards",
            "ValidTgts",
            "CounterType",
            "CounterNum",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let amount = optional_positive_integer(parameters, "CounterNum")?.unwrap_or(1);
    let affected = scope_collection_to_target_player(
        affected_selector(required(parameters, "ValidCards")?)?,
        parameters,
        Operation::ControlledBy,
    )?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::AddCounter,
            vec![
                affected,
                Expression::Text(required(parameters, "CounterType")?.to_ascii_lowercase()),
                Expression::Integer(amount),
            ],
        ),
    )
}

fn map_copy_spell_ability(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TargetType",
            "TgtPrompt",
            "MayChooseTarget",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    if parameters
        .get("TargetType")
        .is_some_and(|target_type| target_type != "Spell")
    {
        return Err(unsupported_value(
            "TargetType",
            required(parameters, "TargetType")?,
        ));
    }
    if parameters
        .get("MayChooseTarget")
        .is_some_and(|value| value != "True")
    {
        return Err(unsupported_value(
            "MayChooseTarget",
            required(parameters, "MayChooseTarget")?,
        ));
    }
    if parameters.contains_key("Defined") && parameters.contains_key("ValidTgts") {
        return Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            "CopySpellAbility cannot combine Defined and ValidTgts",
        ));
    }
    let target = if let Some(defined) = parameters.get("Defined") {
        match defined.as_str() {
            "TriggeredSpellAbility" => call(Operation::TriggeredStackAbility, vec![]),
            "Parent" => call(Operation::ParentStackAbility, vec![]),
            "Targeted" => call(Operation::Target, vec![call(Operation::Any, vec![])]),
            "Remembered" => call(Operation::Remembered, vec![call(Operation::Any, vec![])]),
            value => return Err(unsupported_value("Defined", value)),
        }
    } else {
        call(
            Operation::Target,
            vec![spell_selector(required(parameters, "ValidTgts")?)?],
        )
    };
    mapped_direct(prefix, api, parameters, call(Operation::Copy, vec![target]))
}

fn map_copy_permanent(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "Populate",
            "NumCopies",
            "TokenTapped",
            "TokenAttacking",
            "AttachedTo",
            "RememberTokens",
            "AtEOT",
            "TgtPrompt",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let expression = match parameters.get("Populate").map(String::as_str) {
        Some("True") => {
            if parameters.contains_key("Defined")
                || parameters.contains_key("ValidTgts")
                || parameters.contains_key("NumCopies")
            {
                return Err(diagnostic(
                    "UNSUPPORTED_SELECTOR",
                    "Populate CopyPermanent cannot also define an explicit copy selector",
                ));
            }
            call(Operation::Populate, vec![])
        }
        Some(value) => return Err(unsupported_value("Populate", value)),
        None => {
            if parameters.contains_key("Defined") && parameters.contains_key("ValidTgts") {
                return Err(diagnostic(
                    "UNSUPPORTED_SELECTOR",
                    "CopyPermanent has both Defined and ValidTgts",
                ));
            }
            let source = if let Some(value) = parameters.get("ValidTgts") {
                valid_target_selector(value)?
            } else if let Some(value) = parameters.get("Defined") {
                defined_selector(value)?
            } else {
                return Err(diagnostic(
                    "MISSING_PARAMETER",
                    "CopyPermanent requires Defined, ValidTgts, or Populate$ True",
                ));
            };
            let copy = call(Operation::Copy, vec![source]);
            match optional_positive_integer(parameters, "NumCopies")? {
                None | Some(1) => copy,
                Some(count @ 2..=10) => call(
                    Operation::Sequence,
                    (0..count).map(|_| copy.clone()).collect(),
                ),
                Some(count) => return Err(unsupported_value("NumCopies", &count.to_string())),
            }
        }
    };
    let expression = apply_token_attacking(expression, parameters)?;
    let expression = apply_created_attachment(expression, parameters)?;
    if parameters.contains_key("TokenTapped")
        && (parameters.contains_key("RememberTokens") || parameters.contains_key("AtEOT"))
    {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "TokenTapped copy metadata cannot yet be combined with RememberTokens or AtEOT",
        ));
    }
    let expression = apply_created_object_metadata(expression, parameters)?;
    let expression = if let Some(value) = parameters.get("TokenTapped") {
        if value != "True" {
            return Err(unsupported_value("TokenTapped", value));
        }
        combine_effects(
            vec![
                expression,
                call(Operation::Tap, vec![call(Operation::EffectResult, vec![])]),
            ],
            "tapped copy requires a copy effect",
        )?
    } else {
        expression
    };
    mapped_direct(prefix, api, parameters, expression)
}

fn map_clone(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtZone",
            "Choices",
            "ChoiceZone",
            "CloneTarget",
            "Duration",
            "Optional",
            "SpellDescription",
            "StackDescription",
            "AILogic",
            "TgtPrompt",
            "ChoiceTitle",
            "AddTypes",
        ],
    )?;
    let target = match parameters.get("CloneTarget").map(String::as_str) {
        None | Some("Self") => call(Operation::Source, vec![]),
        Some(value) => defined_selector(value)?,
    };
    let mut prefix_effects = Vec::new();
    let source = if parameters.contains_key("Defined") || parameters.contains_key("ValidTgts") {
        object_selector(parameters, DefaultSelector::Source)?
    } else if let Some(choices) = parameters.get("Choices") {
        let zone = parameters
            .get("ChoiceZone")
            .map(String::as_str)
            .unwrap_or("Battlefield");
        let candidates = match zone {
            "Battlefield" => affected_selector(choices)?,
            "Graveyard" | "Hand" | "Exile" | "Library" => {
                card_selector_in_zone(choices, &zone.to_ascii_lowercase())?
            }
            value => return Err(unsupported_value("ChoiceZone", value)),
        };
        prefix_effects.push(call(
            Operation::ChooseObjects,
            vec![
                candidates,
                Expression::Integer(1),
                call(Operation::You, vec![]),
                Expression::Text("up_to".to_string()),
            ],
        ));
        call(Operation::EffectResult, vec![])
    } else {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "Clone requires Defined, ValidTgts, or Choices",
        ));
    };
    let mut effect = call(
        Operation::CloneCharacteristics,
        vec![target.clone(), source],
    );
    match parameters.get("Duration").map(String::as_str) {
        None | Some("Permanent") => {}
        Some("UntilEndOfTurn") => effect = call(Operation::UntilEndOfTurn, vec![effect]),
        Some(value) => return Err(unsupported_value("Duration", value)),
    }
    if let Some(types) = parameters.get("AddTypes") {
        let additions = types
            .split(" & ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if additions.is_empty() {
            return Err(unsupported_value("AddTypes", types));
        }
        let mut arguments = vec![target];
        arguments.extend(
            additions
                .into_iter()
                .map(|value| Expression::Text(value.to_ascii_lowercase())),
        );
        effect = sequence(effect, call(Operation::AddType, arguments));
    }
    if closed_true_flag(parameters, "Optional")? {
        effect = call(Operation::ChooseUpTo, vec![Expression::Integer(1), effect]);
    }
    prefix_effects.push(effect);
    mapped_direct(
        prefix,
        api,
        parameters,
        combine_effects(prefix_effects, "Clone requires an effect")?,
    )
}

fn map_choose_card(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "Choices",
            "ChoiceZone",
            "Amount",
            "MinAmount",
            "Mandatory",
            "AtRandom",
            "RememberChosen",
            "Reveal",
            "ChoiceTitle",
            "ChoiceDesc",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let amount = optional_positive_integer(parameters, "Amount")?.unwrap_or(1);
    let minimum = match parameters.get("MinAmount").map(String::as_str) {
        None => None,
        Some(value) => Some(
            value
                .parse::<i64>()
                .ok()
                .filter(|minimum| *minimum >= 0 && *minimum <= amount)
                .ok_or_else(|| unsupported_value("MinAmount", value))?,
        ),
    };
    let mandatory = closed_true_flag(parameters, "Mandatory")?;
    if mandatory && minimum.is_some_and(|minimum| minimum != amount) {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "mandatory ChooseCard requires MinAmount to equal Amount",
        ));
    }
    let random = closed_true_flag(parameters, "AtRandom")?;
    if random && parameters.contains_key("MinAmount") {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "random ChooseCard does not support a separate minimum",
        ));
    }
    let zone = parameters
        .get("ChoiceZone")
        .map(String::as_str)
        .unwrap_or("Battlefield");
    let candidates = match zone {
        "Battlefield" => affected_selector(required(parameters, "Choices")?)?,
        "Graveyard" | "Hand" | "Exile" | "Library" => {
            card_selector_in_zone(required(parameters, "Choices")?, &zone.to_ascii_lowercase())?
        }
        value => return Err(unsupported_value("ChoiceZone", value)),
    };
    let chooser = player_selector(parameters, DefaultSelector::You)?;
    if let Some(value) = parameters.get("Reveal") {
        if value != "True" {
            return Err(unsupported_value("Reveal", value));
        }
    }
    let mode = if random {
        "random"
    } else if mandatory || minimum == Some(amount) {
        "exact"
    } else {
        "up_to"
    };
    let expression = apply_remembered_result(
        call(
            Operation::ChooseObjects,
            vec![
                candidates,
                Expression::Integer(amount),
                chooser,
                Expression::Text(mode.to_string()),
            ],
        ),
        parameters,
        "RememberChosen",
        "chosen",
    )?;
    mapped_direct(prefix, api, parameters, expression)
}

fn map_choose_player(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "Choices",
            "ChoiceTitle",
            "Random",
            "Secretly",
            "RememberChosen",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let chooser = player_selector(parameters, DefaultSelector::You)?;
    let choices = defined_player_selector(required(parameters, "Choices")?)?;
    for key in ["Random", "Secretly", "RememberChosen"] {
        closed_true_flag(parameters, key)?;
    }
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::ChoosePlayer,
            vec![
                chooser,
                choices,
                Expression::Boolean(parameters.contains_key("Random")),
                Expression::Boolean(parameters.contains_key("Secretly")),
                Expression::Boolean(parameters.contains_key("RememberChosen")),
            ],
        ),
    )
}

fn map_play(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "Valid",
            "ValidZone",
            "ValidSA",
            "Amount",
            "Controller",
            "WithoutManaCost",
            "RememberPlayed",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    if parameters.contains_key("Defined") && parameters.contains_key("Valid") {
        return Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            "Play cannot combine Defined and Valid selectors",
        ));
    }
    let cards = if let Some(defined) = parameters.get("Defined") {
        defined_selector(defined)?
    } else {
        let zone = parameters
            .get("ValidZone")
            .map(String::as_str)
            .unwrap_or("Hand");
        if !matches!(zone, "Hand" | "Exile" | "Graveyard" | "Library") {
            return Err(unsupported_value("ValidZone", zone));
        }
        card_selector_in_zone(required(parameters, "Valid")?, &zone.to_ascii_lowercase())?
    };
    if let Some(controller) = parameters.get("Controller") {
        if controller != "You" {
            return Err(unsupported_value("Controller", controller));
        }
    }
    let amount = parameters.get("Amount").map(String::as_str).unwrap_or("1");
    if !matches!(amount, "1" | "All") {
        return Err(unsupported_value("Amount", amount));
    }
    let without_mana = closed_true_flag(parameters, "WithoutManaCost")?;
    let valid_sa = parameters
        .get("ValidSA")
        .map(String::as_str)
        .unwrap_or("Spell");
    if !matches!(
        valid_sa,
        "Spell" | "SpellAbility.Land" | "Spell,SpellAbility.Land"
    ) && !valid_sa
        .strip_prefix("Spell.cmcLE")
        .is_some_and(|value| value.parse::<i64>().is_ok_and(|amount| amount >= 0))
    {
        return Err(unsupported_value("ValidSA", valid_sa));
    }
    let mode = format!(
        "amount={};without_mana_cost={};valid_sa={}",
        amount.to_ascii_lowercase(),
        without_mana,
        valid_sa.to_ascii_lowercase()
    );
    let expression = apply_remembered_result(
        call(Operation::Play, vec![cards, Expression::Text(mode)]),
        parameters,
        "RememberPlayed",
        "played",
    )?;
    mapped_direct(prefix, api, parameters, expression)
}

fn map_add_turn(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "NumTurns",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let turns = positive_integer(required(parameters, "NumTurns")?, "NumTurns")?;
    let player = player_selector(parameters, DefaultSelector::You)?;
    let effects = (0..turns)
        .map(|_| call(Operation::ExtraTurn, vec![player.clone()]))
        .collect::<Vec<_>>();
    mapped_direct(
        prefix,
        api,
        parameters,
        combine_effects(effects, "AddTurn requires at least one turn")?,
    )
}

fn map_untap_all(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "ValidCards",
            "ValidTgts",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let affected = scope_collection_to_target_player(
        affected_selector(required(parameters, "ValidCards")?)?,
        parameters,
        Operation::ControlledBy,
    )?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(Operation::Untap, vec![affected]),
    )
}

fn map_tap_all(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "ValidCards",
            "ValidTgts",
            "TgtPrompt",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let mut affected = affected_selector(required(parameters, "ValidCards")?)?;
    if let Some(player) = parameters.get("ValidTgts") {
        affected = add_collection_predicate(
            affected,
            call(
                Operation::ControlledBy,
                vec![call(
                    Operation::Target,
                    vec![draw_player_selector(player, "ValidTgts")?],
                )],
            ),
        )?;
    }
    mapped_direct(
        prefix,
        api,
        parameters,
        call(Operation::Tap, vec![affected]),
    )
}

fn map_tap_or_untap(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let affected = object_selector(parameters, DefaultSelector::Source)?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::ChooseOne,
            vec![
                call(Operation::Tap, vec![affected.clone()]),
                call(Operation::Untap, vec![affected]),
            ],
        ),
    )
}

fn map_remove_counter(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "CounterType",
            "CounterNum",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let amount = positive_integer(required(parameters, "CounterNum")?, "CounterNum")?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::RemoveCounters,
            vec![
                object_selector(parameters, DefaultSelector::Source)?,
                Expression::Text(required(parameters, "CounterType")?.to_ascii_lowercase()),
                Expression::Integer(amount),
            ],
        ),
    )
}

fn map_proliferate(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &["Cost", "Amount", "SpellDescription", "StackDescription"],
    )?;
    let amount = optional_positive_integer(parameters, "Amount")?.unwrap_or(1);
    let effects = (0..amount)
        .map(|_| call(Operation::Proliferate, vec![]))
        .collect::<Vec<_>>();
    mapped_direct(
        prefix,
        api,
        parameters,
        combine_effects(effects, "Proliferate requires a positive amount")?,
    )
}

fn map_cant_attack_or_block(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    reject_unknown(
        parameters,
        &[
            "ValidCard",
            "EffectZone",
            "Description",
            "Secondary",
            "UnlessDefender",
        ],
    )?;
    require_static_effect_zone(parameters, "EffectZone")?;
    if parameters
        .get("Secondary")
        .is_some_and(|value| value != "True")
    {
        return Err(unsupported_value(
            "Secondary",
            required(parameters, "Secondary")?,
        ));
    }
    let affected = affected_selector(required(parameters, "ValidCard")?)?;
    let restrictions = match api {
        "CantAttack" => vec![Operation::CannotAttack],
        "CantBlock" => vec![Operation::CannotBlock],
        "CantAttack,CantBlock" => vec![Operation::CannotAttack, Operation::CannotBlock],
        _ => return Err(diagnostic("UNMAPPED_API", "unknown combat restriction")),
    };
    let unless_defender = parameters
        .get("UnlessDefender")
        .map(|value| unless_defender_predicate(value))
        .transpose()?;
    let restriction = combine_effects(
        restrictions
            .into_iter()
            .map(|operation| {
                let mut arguments = vec![call(Operation::Any, vec![])];
                if let Some(predicate) = &unless_defender {
                    arguments.push(predicate.clone());
                }
                call(operation, arguments)
            })
            .collect(),
        "combat restriction requires an effect",
    )?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: call(Operation::Continuous, vec![affected, restriction]),
    })
}

fn map_must_attack(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    reject_unknown(
        parameters,
        &["ValidCreature", "EffectZone", "Description", "Secondary"],
    )?;
    require_static_effect_zone(parameters, "EffectZone")?;
    if parameters
        .get("Secondary")
        .is_some_and(|value| value != "True")
    {
        return Err(unsupported_value(
            "Secondary",
            required(parameters, "Secondary")?,
        ));
    }
    let affected = affected_selector(required(parameters, "ValidCreature")?)?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: call(
            Operation::Continuous,
            vec![
                affected,
                call(Operation::MustAttack, vec![call(Operation::Any, vec![])]),
            ],
        ),
    })
}

fn map_min_max_blocker(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    reject_unknown(
        parameters,
        &[
            "ValidCard",
            "Min",
            "Max",
            "EffectZone",
            "Description",
            "Secondary",
        ],
    )?;
    require_static_effect_zone(parameters, "EffectZone")?;
    if parameters
        .get("Secondary")
        .is_some_and(|value| value != "True")
    {
        return Err(unsupported_value(
            "Secondary",
            required(parameters, "Secondary")?,
        ));
    }
    let mut effects = Vec::new();
    if let Some(minimum) = optional_positive_integer(parameters, "Min")? {
        effects.push(call(
            Operation::MinimumBlockers,
            vec![Expression::Integer(minimum)],
        ));
    }
    if let Some(maximum) = optional_positive_integer(parameters, "Max")? {
        effects.push(call(
            Operation::MaximumBlockers,
            vec![Expression::Integer(maximum)],
        ));
    }
    let restriction = combine_effects(effects, "MinMaxBlocker requires Min or Max")?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: call(
            Operation::Continuous,
            vec![
                affected_selector(required(parameters, "ValidCard")?)?,
                restriction,
            ],
        ),
    })
}

fn map_cast_with_flash(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    reject_unknown(
        parameters,
        &[
            "ValidCard",
            "ValidSA",
            "Caster",
            "EffectZone",
            "Description",
        ],
    )?;
    require_static_effect_zone(parameters, "EffectZone")?;
    if required(parameters, "ValidSA")? != "Spell" {
        return Err(unsupported_value(
            "ValidSA",
            required(parameters, "ValidSA")?,
        ));
    }
    let caster = match required(parameters, "Caster")? {
        "You" => call(Operation::You, vec![]),
        "Player" => call(Operation::Any, vec![]),
        value => return Err(unsupported_value("Caster", value)),
    };
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: call(
            Operation::Continuous,
            vec![
                affected_selector(required(parameters, "ValidCard")?)?,
                call(Operation::CastWithFlash, vec![caster]),
            ],
        ),
    })
}

fn unless_defender_predicate(value: &str) -> Result<Expression, MappingDiagnostic> {
    let (negated, value) = value
        .strip_prefix('!')
        .map_or((false, value), |value| (true, value));
    let validity = value
        .strip_prefix("controls")
        .ok_or_else(|| unsupported_value("UnlessDefender", value))?;
    let controlled = add_collection_predicate(
        affected_selector(validity)?,
        call(
            Operation::ControlledBy,
            vec![call(Operation::Opponent, vec![])],
        ),
    )?;
    let predicate = call(
        Operation::Nonzero,
        vec![call(Operation::Count, vec![controlled])],
    );
    Ok(if negated {
        call(Operation::Not, vec![predicate])
    } else {
        predicate
    })
}

fn map_cant_be_cast(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    reject_unknown(
        parameters,
        &["ValidCard", "Caster", "EffectZone", "Description"],
    )?;
    require_static_effect_zone(parameters, "EffectZone")?;
    let mut spells = parameters
        .get("ValidCard")
        .map(|value| spell_selector(value))
        .transpose()?
        .unwrap_or_else(|| call(Operation::Spells, vec![]));
    if let Some(value) = parameters.get("Caster") {
        spells = add_collection_predicate(
            spells,
            call(
                Operation::ControlledBy,
                vec![draw_player_selector(value, "Caster")?],
            ),
        )?;
    }
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: call(
            Operation::Continuous,
            vec![
                spells,
                call(Operation::CannotCast, vec![call(Operation::Any, vec![])]),
            ],
        ),
    })
}

fn map_shuffle(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::Shuffle,
            vec![player_selector(parameters, DefaultSelector::You)?],
        ),
    )
}

fn map_set_life(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "LifeAmount",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let amount = positive_integer(required(parameters, "LifeAmount")?, "LifeAmount")?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::SetLife,
            vec![
                Expression::Integer(amount),
                player_selector(parameters, DefaultSelector::You)?,
            ],
        ),
    )
}

fn map_owner_marker_effect(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let operation = match api {
        "Venture" => Operation::Venture,
        "BecomeMonarch" => Operation::BecomeMonarch,
        "TakeInitiative" => Operation::TakeInitiative,
        _ => return Err(diagnostic("UNMAPPED_API", "unknown owner marker effect")),
    };
    let arguments = if parameters.contains_key("Defined") || parameters.contains_key("ValidTgts") {
        vec![player_selector(parameters, DefaultSelector::You)?]
    } else {
        Vec::new()
    };
    mapped_direct(prefix, api, parameters, call(operation, arguments))
}

fn map_investigate(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "Num",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    let amount = optional_positive_integer(parameters, "Num")?.unwrap_or(1);
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::CreateToken,
            vec![
                Expression::Text("c_a_clue_draw".to_string()),
                Expression::Integer(amount),
                player_selector(parameters, DefaultSelector::You)?,
            ],
        ),
    )
}

fn map_attach(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "Object",
            "Choices",
            "ChoiceZone",
            "Move",
            "TgtPrompt",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    if parameters.get("Move").is_some_and(|value| value != "True") {
        return Err(unsupported_value("Move", required(parameters, "Move")?));
    }
    if parameters
        .get("ChoiceZone")
        .is_some_and(|value| value != "Battlefield")
    {
        return Err(unsupported_value(
            "ChoiceZone",
            required(parameters, "ChoiceZone")?,
        ));
    }
    if let Some(object) = parameters.get("Object") {
        let choices = required(parameters, "Choices")?;
        return mapped_direct(
            prefix,
            api,
            parameters,
            call(
                Operation::AttachChoice,
                vec![defined_selector(object)?, affected_selector(choices)?],
            ),
        );
    }
    if parameters.contains_key("Choices") {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "Attach Choices requires Object",
        ));
    }
    let target = object_selector(parameters, DefaultSelector::Source)?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::Attach,
            vec![call(Operation::Source, vec![]), target],
        ),
    )
}

fn map_reveal_hand(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "Defined",
            "ValidTgts",
            "TgtPrompt",
            "Look",
            "SpellDescription",
            "StackDescription",
            "IsCurse",
            "AIPhyrexianPayment",
        ],
    )?;
    let player = match (
        parameters.get("Defined").map(String::as_str),
        parameters.get("ValidTgts"),
    ) {
        (None | Some("Targeted"), Some(value)) => call(
            Operation::Target,
            vec![draw_player_selector(value, "ValidTgts")?],
        ),
        (Some(value), None) => defined_player_selector(value)?,
        (None, None) => call(Operation::You, vec![]),
        (Some(value), Some(_)) => return Err(unsupported_value("Defined", value)),
    };
    let hand = call(
        Operation::Cards,
        vec![call(
            Operation::And,
            vec![
                call(
                    Operation::ZoneIs,
                    vec![Expression::Text("hand".to_string())],
                ),
                call(Operation::OwnedBy, vec![player]),
            ],
        )],
    );
    let expression = match parameters.get("Look").map(String::as_str) {
        None => call(Operation::Reveal, vec![hand]),
        Some("True") => call(Operation::LookAt, vec![hand, call(Operation::You, vec![])]),
        Some(value) => return Err(unsupported_value("Look", value)),
    };
    mapped_direct(prefix, api, parameters, expression)
}

fn map_animate_all(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    map_animate_all_with_effects(prefix, api, selector, parameters, Vec::new())
}

fn map_animate_all_with_effects(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
    mut effects: Vec<Expression>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "ValidCards",
            "ValidTgts",
            "Power",
            "Toughness",
            "Types",
            "RemoveCardTypes",
            "RemoveCreatureTypes",
            "RemoveLandTypes",
            "Colors",
            "OverwriteColors",
            "Keywords",
            "RemoveKeywords",
            "RemoveAllAbilities",
            "Duration",
            "SpellDescription",
            "StackDescription",
            "IsCurse",
            "AILogic",
            "AtEOT",
        ],
    )?;
    let affected = scope_collection_to_target_player(
        affected_selector(required(parameters, "ValidCards")?)?,
        parameters,
        Operation::ControlledBy,
    )?;
    if let Some(value) = parameters.get("RemoveCardTypes") {
        if value != "True" {
            return Err(unsupported_value("RemoveCardTypes", value));
        }
        effects.push(remove_all_card_types(affected.clone()));
    }
    if let Some(value) = parameters.get("RemoveLandTypes") {
        if value != "True" {
            return Err(unsupported_value("RemoveLandTypes", value));
        }
        effects.push(call(
            Operation::RemoveType,
            vec![
                affected.clone(),
                Expression::Text("land_subtypes".to_string()),
            ],
        ));
    }
    match (parameters.get("Power"), parameters.get("Toughness")) {
        (Some(power), Some(toughness)) => effects.push(call(
            Operation::SetPt,
            vec![
                affected.clone(),
                Expression::Integer(
                    power
                        .parse::<i64>()
                        .map_err(|_| unsupported_value("Power", power))?,
                ),
                Expression::Integer(
                    toughness
                        .parse::<i64>()
                        .map_err(|_| unsupported_value("Toughness", toughness))?,
                ),
            ],
        )),
        (None, None) => {}
        _ => {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "AnimateAll requires Power and Toughness together",
            ));
        }
    }
    if let Some(value) = parameters.get("RemoveCreatureTypes") {
        if value != "True" {
            return Err(unsupported_value("RemoveCreatureTypes", value));
        }
        effects.push(call(
            Operation::RemoveType,
            vec![
                affected.clone(),
                Expression::Text("creature_subtypes".to_string()),
            ],
        ));
    }
    if let Some(types) = parameters.get("Types") {
        for card_type in types.split(',').map(str::trim) {
            if card_type.is_empty() {
                return Err(unsupported_value("Types", types));
            }
            effects.push(call(
                Operation::AddType,
                vec![affected.clone(), Expression::Text(card_type.to_string())],
            ));
        }
    }
    if let Some(colors) = parameters.get("Colors") {
        if parameters.get("OverwriteColors").map(String::as_str) != Some("True") {
            return Err(diagnostic(
                "UNSUPPORTED_PARAMETER",
                "AnimateAll Colors requires OverwriteColors$ True",
            ));
        }
        let mut arguments = vec![affected.clone()];
        arguments.extend(
            parse_animate_colors(colors)?
                .into_iter()
                .map(Expression::Text),
        );
        effects.push(call(Operation::SetColor, arguments));
    } else if parameters.contains_key("OverwriteColors") {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "OverwriteColors requires Colors",
        ));
    }
    if let Some(keywords) = parameters.get("Keywords") {
        for keyword in keywords.split(" & ") {
            effects.push(call(
                Operation::GrantKeyword,
                vec![
                    affected.clone(),
                    Expression::Text(normalize_simple_keyword(keyword)?),
                ],
            ));
        }
    }
    if let Some(keywords) = parameters.get("RemoveKeywords") {
        for keyword in keywords.split(" & ") {
            effects.push(call(
                Operation::RemoveKeyword,
                vec![
                    affected.clone(),
                    Expression::Text(normalize_simple_keyword(keyword)?),
                ],
            ));
        }
    }
    if let Some(value) = parameters.get("RemoveAllAbilities") {
        if value != "True" {
            return Err(unsupported_value("RemoveAllAbilities", value));
        }
        effects.push(call(Operation::RemoveAllAbilities, vec![affected.clone()]));
    }
    let mut expression = combine_effects(effects, "simple AnimateAll has no typed changes")?;
    match parameters.get("Duration").map(String::as_str) {
        None | Some("EOT") | Some("EndOfTurn") | Some("UntilEndOfTurn") => {
            expression = call(Operation::UntilEndOfTurn, vec![expression]);
        }
        Some("Permanent") => {}
        Some("Perpetual") => {
            expression = call(Operation::Perpetual, vec![expression]);
        }
        Some(value) => return Err(unsupported_value("Duration", value)),
    }
    if let Some(value) = parameters.get("AtEOT") {
        expression = call(
            Operation::Sequence,
            vec![expression, map_at_eot_cleanup(value, &affected)?],
        );
    }
    mapped_direct(prefix, api, parameters, expression)
}

fn map_cleanup(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "DB")?;
    const DOMAINS: &[(&str, &str)] = &[
        ("ClearRemembered", "remembered"),
        ("ClearImprinted", "imprinted"),
        ("ClearChosenCard", "chosen_card"),
        ("ClearChosenPlayer", "chosen_player"),
        ("ClearNamedCard", "named_card"),
        ("ClearChosenColor", "chosen_color"),
        ("ClearChosenType", "chosen_type"),
        ("ClearCoinFlips", "coin_flips"),
        ("ClearTriggered", "triggered"),
    ];
    reject_unknown(
        parameters,
        &DOMAINS.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
    )?;
    let mut effects = Vec::new();
    for (key, domain) in DOMAINS {
        match parameters.get(*key).map(String::as_str) {
            Some("True") => effects.push(call(
                Operation::Forget,
                vec![Expression::Text((*domain).to_string())],
            )),
            Some(value) => return Err(unsupported_value(key, value)),
            None => {}
        }
    }
    if effects.is_empty() {
        Err(diagnostic(
            "MISSING_PARAMETER",
            "Cleanup requires at least one closed clear flag",
        ))
    } else {
        mapped_direct(
            prefix,
            api,
            parameters,
            combine_effects(effects, "Cleanup requires a clear effect")?,
        )
    }
}

fn parse_animate_colors(value: &str) -> Result<Vec<String>, MappingDiagnostic> {
    let colors = value.split(',').map(str::trim).collect::<Vec<_>>();
    if colors.is_empty()
        || colors.len() > 2
        || colors.iter().any(|color| {
            !matches!(
                *color,
                "White" | "Blue" | "Black" | "Red" | "Green" | "Colorless"
            )
        })
    {
        return Err(unsupported_value("Colors", value));
    }
    Ok(colors
        .into_iter()
        .map(|color| color.to_ascii_lowercase())
        .collect())
}

fn add_collection_predicate(
    selector: Expression,
    predicate: Expression,
) -> Result<Expression, MappingDiagnostic> {
    let Expression::Call {
        operation,
        mut arguments,
    } = selector
    else {
        return Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            "expected a typed collection selector",
        ));
    };
    if !matches!(
        operation,
        Operation::Cards | Operation::Permanents | Operation::Spells
    ) {
        return Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            "predicate composition requires a card collection",
        ));
    }
    let combined = match arguments.len() {
        0 => predicate,
        1 => call(Operation::And, vec![arguments.remove(0), predicate]),
        _ => {
            return Err(diagnostic(
                "UNSUPPORTED_SELECTOR",
                "card collection has an invalid predicate shape",
            ));
        }
    };
    Ok(call(operation, vec![combined]))
}

fn extract_legacy_timing(
    parameters: &mut BTreeMap<String, String>,
) -> Result<Option<Expression>, MappingDiagnostic> {
    let source_zone = parameters
        .remove("ActivationZone")
        .map(|value| {
            let zone = match value.as_str() {
                "Graveyard" => "graveyard",
                "Hand" => "hand",
                "Command" => "command",
                "Exile" => "exile",
                "Stack" => "stack",
                _ => return Err(unsupported_value("ActivationZone", &value)),
            };
            Ok(call(
                Operation::TimingCondition,
                vec![call(
                    Operation::ZoneIs,
                    vec![Expression::Text(zone.to_string())],
                )],
            ))
        })
        .transpose()?;
    let sorcery = parameters
        .remove("SorcerySpeed")
        .map(|value| {
            if value == "True" {
                Ok(())
            } else {
                Err(unsupported_value("SorcerySpeed", &value))
            }
        })
        .transpose()?
        .is_some();
    let your_turn = parameters
        .remove("PlayerTurn")
        .map(|value| match value.as_str() {
            "True" | "You" => Ok(()),
            _ => Err(unsupported_value("PlayerTurn", &value)),
        })
        .transpose()?
        .is_some();
    let once = parameters
        .remove("ActivationLimit")
        .map(|value| {
            if value == "1" {
                Ok(())
            } else {
                Err(unsupported_value("ActivationLimit", &value))
            }
        })
        .transpose()?
        .is_some();
    let activation = parameters
        .remove("Activation")
        .map(|value| closed_activation_condition(&value))
        .transpose()?;
    let phases = parameters
        .remove("ActivationPhases")
        .map(|value| closed_activation_phases(&value))
        .transpose()?;
    let first_combat = parameters
        .remove("ActivationFirstCombat")
        .map(|value| match value.as_str() {
            "True" => Ok(call(
                Operation::TimingCondition,
                vec![call(
                    Operation::During,
                    vec![Expression::Text("first_combat".to_string())],
                )],
            )),
            _ => Err(unsupported_value("ActivationFirstCombat", &value)),
        })
        .transpose()?;
    let mut timings = Vec::new();
    if let Some(source_zone) = source_zone {
        timings.push(source_zone);
    }
    if sorcery {
        timings.push(call(Operation::TimingSorcery, vec![]));
    }
    if your_turn {
        timings.push(call(Operation::TimingYourTurn, vec![]));
    }
    if once {
        timings.push(call(Operation::TimingOnceEachTurn, vec![]));
    }
    if let Some(activation) = activation {
        timings.push(call(Operation::TimingCondition, vec![activation]));
    }
    if let Some(phases) = phases {
        timings.push(phases);
    }
    if let Some(first_combat) = first_combat {
        timings.push(first_combat);
    }
    Ok(combine_timings(timings))
}

fn closed_activation_condition(value: &str) -> Result<Expression, MappingDiagnostic> {
    let you = call(Operation::You, vec![]);
    let controlled = |kind: &str| {
        call(
            Operation::Permanents,
            vec![call(
                Operation::And,
                vec![
                    call(Operation::TypeIs, vec![Expression::Text(kind.to_string())]),
                    call(Operation::ControlledBy, vec![you.clone()]),
                ],
            )],
        )
    };
    let owned_in = |zone: &str| {
        call(
            Operation::Cards,
            vec![call(
                Operation::And,
                vec![
                    call(Operation::ZoneIs, vec![Expression::Text(zone.to_string())]),
                    call(Operation::OwnedBy, vec![you.clone()]),
                ],
            )],
        )
    };
    match value {
        "Threshold" => Ok(call(
            Operation::AtLeast,
            vec![
                call(Operation::Count, vec![owned_in("graveyard")]),
                Expression::Integer(7),
            ],
        )),
        "Metalcraft" => Ok(call(
            Operation::AtLeast,
            vec![
                call(Operation::Count, vec![controlled("artifact")]),
                Expression::Integer(3),
            ],
        )),
        "Hellbent" => Ok(call(
            Operation::Equals,
            vec![
                call(Operation::Count, vec![owned_in("hand")]),
                Expression::Integer(0),
            ],
        )),
        "Delirium" => Ok(call(
            Operation::AtLeast,
            vec![
                call(
                    Operation::DistinctCount,
                    vec![
                        owned_in("graveyard"),
                        Expression::Text("card_types".to_string()),
                    ],
                ),
                Expression::Integer(4),
            ],
        )),
        "Blessing" => Ok(call(
            Operation::DesignationIs,
            vec![Expression::Text("citys_blessing".to_string())],
        )),
        "Solved" => Ok(call(
            Operation::DesignationIs,
            vec![Expression::Text("solved".to_string())],
        )),
        _ => Err(unsupported_value("Activation", value)),
    }
}

fn closed_activation_phases(value: &str) -> Result<Expression, MappingDiagnostic> {
    const PHASES: &[&str] = &[
        "Upkeep",
        "Draw",
        "Main1",
        "BeginCombat",
        "Declare Attackers",
        "Declare Blockers",
        "Combat Damage",
        "EndCombat",
        "Main2",
        "End of Turn",
        "Cleanup",
    ];
    let valid_part = |part: &str| PHASES.contains(&part.trim());
    for range in value.split(',') {
        let mut bounds = range.split("->");
        let Some(start) = bounds.next() else {
            return Err(unsupported_value("ActivationPhases", value));
        };
        if !valid_part(start)
            || bounds.next().is_some_and(|end| !valid_part(end))
            || bounds.next().is_some()
        {
            return Err(unsupported_value("ActivationPhases", value));
        }
    }
    Ok(call(
        Operation::TimingCondition,
        vec![call(
            Operation::During,
            vec![Expression::Text(format!(
                "phase_window:{}",
                value.to_ascii_lowercase().replace(' ', "_")
            ))],
        )],
    ))
}

fn combine_timings(mut timings: Vec<Expression>) -> Option<Expression> {
    match timings.len() {
        0 => None,
        1 => timings.pop(),
        _ => Some(call(Operation::TimingAll, timings)),
    }
}

fn normalize_legacy_defaults(parameters: &mut BTreeMap<String, String>) {
    if let Some(maximum) = parameters.get("TargetMax") {
        let minimum = parameters.get("TargetMin").map(String::as_str);
        if maximum == "1" && matches!(minimum, None | Some("1")) {
            parameters.remove("TargetMax");
            parameters.remove("TargetMin");
        }
    } else if parameters.get("TargetMin").map(String::as_str) == Some("1") {
        parameters.remove("TargetMin");
    }
    for key in ["ActivationZone", "TgtZone"] {
        if parameters.get(key).map(String::as_str) == Some("Battlefield") {
            parameters.remove(key);
        }
    }
}

fn extract_target_range(
    parameters: &mut BTreeMap<String, String>,
) -> Result<Option<(i64, i64)>, MappingDiagnostic> {
    let minimum = parameters.get("TargetMin").cloned();
    let maximum = parameters.get("TargetMax").cloned();
    if minimum.is_none() && maximum.is_none() {
        return Ok(None);
    }
    if !parameters.contains_key("ValidTgts") {
        return Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            "target cardinality requires ValidTgts in the same ability",
        ));
    }
    let minimum = minimum.as_deref().unwrap_or("1");
    let maximum = maximum.as_deref().unwrap_or("1");
    let minimum = minimum
        .parse::<i64>()
        .map_err(|_| unsupported_value("TargetMin", minimum))?;
    let maximum = maximum
        .parse::<i64>()
        .map_err(|_| unsupported_value("TargetMax", maximum))?;
    if minimum < 0 || maximum < 1 || minimum > maximum {
        return Err(diagnostic(
            "UNSUPPORTED_VALUE",
            "target range must satisfy 0 <= TargetMin <= TargetMax and TargetMax >= 1",
        ));
    }
    parameters.remove("TargetMin");
    parameters.remove("TargetMax");
    if minimum == 1 && maximum == 1 {
        Ok(None)
    } else {
        Ok(Some((minimum, maximum)))
    }
}

fn apply_target_range(
    expression: &mut Expression,
    minimum: i64,
    maximum: i64,
) -> Result<usize, MappingDiagnostic> {
    let mut declared = false;
    apply_target_range_node(expression, minimum, maximum, &mut declared)
}

fn apply_target_range_node(
    expression: &mut Expression,
    minimum: i64,
    maximum: i64,
    declared: &mut bool,
) -> Result<usize, MappingDiagnostic> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        return Ok(0);
    };
    if *operation == Operation::Target {
        if arguments.len() != 1 {
            return Err(diagnostic(
                "TARGET_RANGE_MISMATCH",
                "typed target selector must have exactly one restriction argument",
            ));
        }
        if !*declared {
            let selector = arguments.remove(0);
            *operation = Operation::TargetRange;
            *arguments = vec![
                selector,
                Expression::Integer(minimum),
                Expression::Integer(maximum),
            ];
            *declared = true;
            return Ok(1);
        }
        return Ok(0);
    }
    let mut replaced = 0;
    for argument in arguments {
        replaced += apply_target_range_node(argument, minimum, maximum, declared)?;
    }
    Ok(replaced)
}

fn mapped_direct(
    prefix: LegacyAbilityPrefix,
    api: &str,
    parameters: &BTreeMap<String, String>,
    expression: Expression,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression,
    })
}

fn parameters(
    expression: &LegacyExpression,
) -> Result<BTreeMap<String, String>, MappingDiagnostic> {
    let mut parameters = BTreeMap::new();
    for field in expression.fields.iter().skip(1) {
        let Some(key) = field.key.as_ref() else {
            return Err(diagnostic(
                "MALFORMED_PARAMETER",
                "ability parameter has no `$` key separator",
            ));
        };
        if parameters
            .insert(key.clone(), field.value.clone())
            .is_some()
        {
            return Err(diagnostic(
                "DUPLICATE_PARAMETER",
                &format!("parameter `{key}` appears more than once"),
            ));
        }
    }
    Ok(parameters)
}

pub(crate) fn parse_simple_cost(
    value: Option<&String>,
) -> Result<Vec<Expression>, MappingDiagnostic> {
    parse_cost_with_controller(value, call(Operation::You, vec![]))
}

fn parse_cost_with_controller(
    value: Option<&String>,
    controller: Expression,
) -> Result<Vec<Expression>, MappingDiagnostic> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let mut mana = String::new();
    let mut costs = Vec::new();
    for token in split_cost_tokens(value)? {
        if token == "T" {
            costs.push(call(Operation::TapSelf, vec![]));
        } else if token.chars().all(|character| character.is_ascii_digit())
            || matches!(
                token.as_str(),
                "W" | "U" | "B" | "R" | "G" | "C" | "X" | "Y" | "Z"
            )
        {
            mana.push('{');
            mana.push_str(&token);
            mana.push('}');
        } else if let Some(payload) = cost_payload(&token, "Sac") {
            let (amount, validity) = parse_counted_cost(payload, "Sac")?;
            if amount == 1 && validity == "CARDNAME" {
                costs.push(call(Operation::SacrificeSelf, vec![]));
            } else {
                let selector =
                    affected_selector(validity).map_err(|_| unsupported_value("Cost", value))?;
                let selector = add_collection_predicate(
                    selector,
                    call(Operation::ControlledBy, vec![controller.clone()]),
                )
                .map_err(|_| unsupported_value("Cost", value))?;
                costs.push(call(
                    Operation::Sacrifice,
                    vec![selector, Expression::Integer(amount)],
                ));
            }
        } else if let Some(payload) = cost_payload(&token, "PayLife") {
            costs.push(call(
                Operation::PayLife,
                vec![Expression::Integer(positive_integer(payload, "Cost")?)],
            ));
        } else if let Some(payload) = cost_payload(&token, "Discard") {
            let (amount, validity) = parse_counted_cost(payload, "Discard")?;
            costs.push(call(
                Operation::DiscardCost,
                vec![
                    Expression::Integer(amount),
                    cost_card_selector(validity, None)
                        .map_err(|_| unsupported_value("Cost", value))?,
                ],
            ));
        } else if let Some(payload) = cost_payload(&token, "ExileFromGrave") {
            let (amount, validity) = parse_counted_cost(payload, "ExileFromGrave")?;
            costs.push(call(
                Operation::ExileCost,
                vec![
                    cost_card_selector(validity, Some("graveyard"))
                        .map_err(|_| unsupported_value("Cost", value))?,
                    Expression::Integer(amount),
                ],
            ));
        } else if let Some(payload) = cost_payload(&token, "ExileFromHand") {
            let (amount, validity) = parse_counted_cost(payload, "ExileFromHand")?;
            costs.push(call(
                Operation::ExileCost,
                vec![
                    cost_card_selector(validity, Some("hand"))
                        .map_err(|_| unsupported_value("Cost", value))?,
                    Expression::Integer(amount),
                ],
            ));
        } else if let Some(payload) = cost_payload(&token, "Exile") {
            let (amount, validity) = parse_counted_cost(payload, "Exile")?;
            if validity != "CARDNAME" {
                return Err(unsupported_value("Cost", value));
            }
            costs.push(call(
                Operation::ExileCost,
                vec![call(Operation::Source, vec![]), Expression::Integer(amount)],
            ));
        } else if let Some(payload) = cost_payload(&token, "AddCounter") {
            let (amount, counter) = parse_counted_cost_nonnegative(payload, "AddCounter")?;
            if counter != "LOYALTY" {
                return Err(unsupported_value("Cost", value));
            }
            costs.push(call(
                Operation::LoyaltyCost,
                vec![Expression::Integer(amount)],
            ));
        } else if let Some(payload) = cost_payload(&token, "SubCounter") {
            let (amount, counter) = parse_counted_cost(payload, "SubCounter")?;
            if counter == "LOYALTY" {
                costs.push(call(
                    Operation::LoyaltyCost,
                    vec![Expression::Integer(-amount)],
                ));
            } else {
                for _ in 0..amount {
                    costs.push(call(
                        Operation::RemoveCounterCost,
                        vec![
                            call(Operation::Source, vec![]),
                            Expression::Text(counter.to_ascii_lowercase()),
                        ],
                    ));
                }
            }
        } else {
            return Err(unsupported_value("Cost", value));
        }
    }
    if !mana.is_empty() {
        costs.insert(0, call(Operation::ManaCost, vec![Expression::Text(mana)]));
    }
    Ok(costs)
}

fn cost_card_selector(validity: &str, zone: Option<&str>) -> Result<Expression, MappingDiagnostic> {
    if validity == "Hand" || validity == "Card" {
        let selector = call(Operation::Cards, vec![]);
        return zone.map_or(Ok(selector.clone()), |zone| {
            add_collection_predicate(
                selector,
                call(Operation::ZoneIs, vec![Expression::Text(zone.to_string())]),
            )
        });
    }
    if validity == "CARDNAME" && zone.is_none() {
        return Ok(call(Operation::Source, vec![]));
    }
    let mut selector = if validity == "CARDNAME" {
        call(
            Operation::Cards,
            vec![call(
                Operation::Equals,
                vec![
                    call(Operation::Any, vec![]),
                    call(Operation::Source, vec![]),
                ],
            )],
        )
    } else {
        let Expression::Call {
            operation,
            arguments,
        } = affected_selector(validity)?
        else {
            return Err(unsupported_value("Cost", validity));
        };
        if !matches!(operation, Operation::Cards | Operation::Permanents) {
            return Err(unsupported_value("Cost", validity));
        }
        call(Operation::Cards, arguments)
    };
    if let Some(zone) = zone {
        selector = add_collection_predicate(
            selector,
            call(Operation::ZoneIs, vec![Expression::Text(zone.to_string())]),
        )?;
    }
    Ok(selector)
}

fn split_cost_tokens(value: &str) -> Result<Vec<String>, MappingDiagnostic> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0_u8;
    for character in value.chars() {
        match character {
            '<' => {
                depth = depth.saturating_add(1);
                current.push(character);
            }
            '>' => {
                if depth == 0 {
                    return Err(unsupported_value("Cost", value));
                }
                depth -= 1;
                current.push(character);
            }
            character if character.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if depth != 0 {
        return Err(unsupported_value("Cost", value));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn cost_payload<'a>(token: &'a str, name: &str) -> Option<&'a str> {
    token
        .strip_prefix(name)
        .and_then(|value| value.strip_prefix('<'))
        .and_then(|value| value.strip_suffix('>'))
}

fn parse_counted_cost<'a>(
    payload: &'a str,
    key: &str,
) -> Result<(i64, &'a str), MappingDiagnostic> {
    let mut fields = payload.splitn(3, '/');
    let amount_text = fields.next().unwrap_or_default();
    let validity = fields.next().unwrap_or_default();
    if validity.is_empty() {
        return Err(unsupported_value("Cost", payload));
    }
    Ok((positive_integer(amount_text, key)?, validity))
}

fn parse_counted_cost_nonnegative<'a>(
    payload: &'a str,
    key: &str,
) -> Result<(i64, &'a str), MappingDiagnostic> {
    let mut fields = payload.splitn(3, '/');
    let amount_text = fields.next().unwrap_or_default();
    let validity = fields.next().unwrap_or_default();
    if validity.is_empty() {
        return Err(unsupported_value("Cost", payload));
    }
    let amount = amount_text
        .parse::<i64>()
        .map_err(|_| unsupported_value(key, amount_text))?;
    if amount < 0 {
        return Err(unsupported_value(key, amount_text));
    }
    Ok((amount, validity))
}

fn normalize_mana(value: &str, amount: i64) -> Result<String, MappingDiagnostic> {
    let normalized = if matches!(value, "W" | "U" | "B" | "R" | "G" | "C") {
        format!("{{{value}}}")
    } else if value == "Any" {
        "any_color".to_string()
    } else if value == "Combo ColorIdentity" {
        "command_color_identity".to_string()
    } else if let Some(colors) = value.strip_prefix("Combo ") {
        let choices = colors
            .split_whitespace()
            .filter(|color| matches!(*color, "W" | "U" | "B" | "R" | "G" | "C"))
            .map(|color| format!("{{{color}}}"))
            .collect::<Vec<_>>();
        if choices.is_empty() || choices.len() != colors.split_whitespace().count() {
            return Err(unsupported_value("Produced", value));
        }
        choices.join(" or ")
    } else {
        return Err(unsupported_value("Produced", value));
    };
    Ok(if amount == 1 {
        normalized
    } else {
        format!("{amount} x {normalized}")
    })
}

fn normalize_mana_restriction(value: &str) -> Result<String, MappingDiagnostic> {
    const CLOSED_BRANCHES: &[&str] = &[
        "Activated",
        "Activated.Alien+inZoneBattlefield",
        "Activated.Ally",
        "Activated.Artifact",
        "Activated.Artifact+inZoneBattlefield",
        "Activated.Assassin",
        "Activated.ChosenType",
        "Activated.ClassLevelUp",
        "Activated.Cleric+inZoneBattlefield",
        "Activated.Creature",
        "Activated.Creature+ChosenType",
        "Activated.Creature+inZoneBattlefield",
        "Activated.Dinosaur",
        "Activated.Dragon+inZoneBattlefield",
        "Activated.Eldrazi+Colorless+inZoneBattlefield",
        "Activated.Elemental",
        "Activated.Elemental+inZoneBattlefield",
        "Activated.Equip",
        "Activated.Hero",
        "Activated.Land",
        "Activated.Myr+inZoneBattlefield",
        "Activated.Outlaw",
        "Activated.Permanent+Colorless+inZoneBattlefield",
        "Activated.PowerUp",
        "Activated.Rogue+inZoneBattlefield",
        "Activated.Time Lord+inZoneBattlefield",
        "Activated.Villain",
        "Activated.Warrior+inZoneBattlefield",
        "Activated.Wizard+inZoneBattlefield",
        "CantCastNonArtifactSpells",
        "CantCastSpellFromHand",
        "CantPayGenericCosts",
        "CostContainsC",
        "CostContainsX",
        "CumulativeUpkeep",
        "Spell",
        "Spell.!wasCastFromYourHand",
        "Spell.Alien",
        "Spell.Ally",
        "Spell.Angel",
        "Spell.Artifact",
        "Spell.Assassin",
        "Spell.Aura",
        "Spell.ChosenColor+MonoColor",
        "Spell.ChosenType",
        "Spell.Cleric",
        "Spell.Colorless",
        "Spell.Creature",
        "Spell.Creature+Blue",
        "Spell.Creature+ChosenType",
        "Spell.Creature+Dragon",
        "Spell.Creature+Elf",
        "Spell.Creature+Legendary",
        "Spell.Creature+NoAbilities",
        "Spell.Creature+Phyrexian",
        "Spell.Creature+cmcGE4",
        "Spell.Creature+hasXCost",
        "Spell.Demon",
        "Spell.Dinosaur",
        "Spell.Disturb",
        "Spell.Dragon",
        "Spell.Eldrazi+Colorless",
        "Spell.Elemental",
        "Spell.Enchantment",
        "Spell.Equipment",
        "Spell.Hero",
        "Spell.Instant",
        "Spell.IsCommander+YouOwn",
        "Spell.IsRemembered",
        "Spell.Kicked",
        "Spell.Knight",
        "Spell.Legendary",
        "Spell.Lesson",
        "Spell.Mount",
        "Spell.MultiColor",
        "Spell.Myr",
        "Spell.Ninja",
        "Spell.Omen",
        "Spell.Outlaw",
        "Spell.Pilot",
        "Spell.Planeswalker",
        "Spell.Planeswalker+Chandra",
        "Spell.Rogue",
        "Spell.Room",
        "Spell.Shrine",
        "Spell.Sliver",
        "Spell.Sorcery",
        "Spell.Spirit",
        "Spell.Time Lord",
        "Spell.Turtle",
        "Spell.Vampire",
        "Spell.Vehicle",
        "Spell.Villain",
        "Spell.Warrior",
        "Spell.Wizard",
        "Spell.YouDontOwn",
        "Spell.cmcGE4",
        "Spell.cmcGE5",
        "Spell.hasXCost",
        "Spell.isCastFaceDown",
        "Spell.isCastFaceDown+Creature",
        "Spell.nonColorless+!hasXCost",
        "Spell.nonCreature",
        "Spell.numColorsEQ3",
        "Spell.wasCastFromExile",
        "Spell.wasCastFromGraveyard+withFlashback",
        "Spell.wasCastFromYourGraveyard",
        "Spell.withDevoid",
        "Spell.withForetell",
        "Spell.withFreerunning",
        "Static.Foretelling",
        "Static.ManifestUp+Creature",
        "Static.MorphUp",
        "Static.Unlock",
        "Static.isTurnFaceUp",
        "Static.isTurnFaceUp+Creature",
        "nonSpell",
    ];
    let mut normalized = Vec::new();
    for branch in value.split(',').map(str::trim) {
        if CLOSED_BRANCHES.binary_search(&branch).is_err() {
            return Err(unsupported_value("RestrictValid", value));
        }
        normalized.push(branch);
    }
    if normalized.is_empty() {
        return Err(unsupported_value("RestrictValid", value));
    }
    Ok(normalized.join(","))
}

fn normalize_dig_zone(value: &str, key: &str) -> Result<String, MappingDiagnostic> {
    if matches!(
        value,
        "Hand" | "Library" | "Graveyard" | "Exile" | "Battlefield"
    ) {
        Ok(value.to_ascii_lowercase())
    } else {
        Err(unsupported_value(key, value))
    }
}

fn dig_library_position(
    parameters: &BTreeMap<String, String>,
    key: &str,
) -> Result<i64, MappingDiagnostic> {
    match parameters.get(key).map(String::as_str) {
        None | Some("-1") => Ok(-1),
        Some("0") => Ok(0),
        Some(value) => Err(unsupported_value(key, value)),
    }
}

fn closed_true_flag(
    parameters: &BTreeMap<String, String>,
    key: &str,
) -> Result<bool, MappingDiagnostic> {
    match parameters.get(key).map(String::as_str) {
        None => Ok(false),
        Some("True") => Ok(true),
        Some(value) => Err(unsupported_value(key, value)),
    }
}

#[derive(Clone, Copy)]
enum DefaultSelector {
    Source,
    You,
}

fn object_selector(
    parameters: &BTreeMap<String, String>,
    default: DefaultSelector,
) -> Result<Expression, MappingDiagnostic> {
    if parameters.contains_key("Defined") && parameters.contains_key("ValidTgts") {
        return Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            "simultaneous Defined and ValidTgts requires an explicit mapper",
        ));
    }
    if let Some(value) = parameters.get("ValidTgts") {
        if let Some(zones) = parameters.get("TgtZone") {
            let mut candidates = Vec::new();
            for zone in normalize_zone_list("TgtZone", zones)? {
                candidates.push(match zone.as_str() {
                    "battlefield" => affected_selector(value)?,
                    "stack" => spell_selector(value)?,
                    "graveyard" | "hand" | "exile" | "library" | "command" => {
                        card_selector_in_zone(value, &zone)?
                    }
                    _ => return Err(unsupported_value("TgtZone", zones)),
                });
            }
            let candidates = match candidates.len() {
                0 => return Err(unsupported_value("TgtZone", zones)),
                1 => candidates.remove(0),
                _ => call(Operation::All, candidates),
            };
            return Ok(call(Operation::Target, vec![candidates]));
        }
        return valid_target_selector(value);
    }
    if parameters.contains_key("TgtZone") {
        return Err(diagnostic(
            "MISSING_PARAMETER",
            "TgtZone requires ValidTgts",
        ));
    }
    if let Some(value) = parameters.get("Defined") {
        return defined_selector(value);
    }
    Ok(default_selector(default))
}

fn player_selector(
    parameters: &BTreeMap<String, String>,
    default: DefaultSelector,
) -> Result<Expression, MappingDiagnostic> {
    if parameters.contains_key("Defined") && parameters.contains_key("ValidTgts") {
        return Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            "simultaneous Defined and ValidTgts player selectors are ambiguous",
        ));
    }
    if let Some(value) = parameters.get("ValidTgts") {
        return targeted_player_selector(value, "ValidTgts");
    }
    parameters
        .get("Defined")
        .map(|value| defined_player_selector(value))
        .unwrap_or_else(|| Ok(default_selector(default)))
}

fn targeted_player_selector(value: &str, key: &str) -> Result<Expression, MappingDiagnostic> {
    Ok(call(
        Operation::Target,
        vec![draw_player_selector(value, key)?],
    ))
}

fn scope_collection_to_target_player(
    selector: Expression,
    parameters: &BTreeMap<String, String>,
    relation: Operation,
) -> Result<Expression, MappingDiagnostic> {
    let Some(value) = parameters.get("ValidTgts") else {
        return Ok(selector);
    };
    if !matches!(relation, Operation::ControlledBy | Operation::OwnedBy) {
        return Err(diagnostic(
            "MAPPING_CONFIGURATION",
            "target-player collection scope requires a controller or owner relation",
        ));
    }
    add_collection_predicate(
        selector,
        call(
            relation,
            vec![targeted_player_selector(value, "ValidTgts")?],
        ),
    )
}

fn default_selector(default: DefaultSelector) -> Expression {
    match default {
        DefaultSelector::Source => call(Operation::Source, vec![]),
        DefaultSelector::You => call(Operation::You, vec![]),
    }
}

fn defined_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    match value {
        "Self" | "EffectSource" | "OriginalHost" => Ok(call(Operation::Source, vec![])),
        "Remembered" | "RememberedCard" => Ok(call(
            Operation::Remembered,
            vec![call(Operation::Any, vec![])],
        )),
        "Imprinted" | "ImprintedCard" => Ok(call(
            Operation::Imprinted,
            vec![call(Operation::Any, vec![])],
        )),
        "RememberedLKI" | "DelayTriggerRememberedLKI" => Ok(call(Operation::RememberedLki, vec![])),
        "ChosenCard" => Ok(call(Operation::Chosen, vec![call(Operation::Any, vec![])])),
        "Targeted" => Ok(call(Operation::Target, vec![call(Operation::Any, vec![])])),
        "You" => Ok(call(Operation::You, vec![])),
        "Opponent" | "Player.Opponent" => Ok(call(Operation::Opponent, vec![])),
        "Equipped" => Ok(call(
            Operation::EquippedObject,
            vec![call(Operation::Source, vec![])],
        )),
        "Enchanted" => Ok(call(
            Operation::EnchantedObject,
            vec![call(Operation::Source, vec![])],
        )),
        "TriggeredCard" => Ok(call(Operation::Triggered, vec![])),
        "TriggeredSource" | "TriggeredSourceLKICopy" => Ok(call(Operation::Triggered, vec![])),
        "TriggeredCardLKICopy" | "TriggeredNewCardLKICopy" => {
            Ok(call(Operation::Triggered, vec![]))
        }
        "TriggeredAttacker" | "TriggeredAttackerLKICopy" => {
            Ok(call(Operation::TriggeredAttacker, vec![]))
        }
        "TriggeredBlocker" | "TriggeredBlockerLKICopy" => {
            Ok(call(Operation::TriggeredBlocker, vec![]))
        }
        "TriggeredCardController" => Ok(call(
            Operation::ControllerOf,
            vec![call(Operation::Triggered, vec![])],
        )),
        "ParentTarget" => Ok(call(Operation::ParentTarget, vec![])),
        "TriggeredPlayer" => Ok(call(Operation::TriggeredPlayer, vec![])),
        "TriggeredTarget" | "TriggeredTargetLKICopy" => {
            Ok(call(Operation::TriggeredTarget, vec![]))
        }
        "TriggeredActivator" => Ok(call(Operation::TriggeredActivator, vec![])),
        "TriggeredDefendingPlayer" => Ok(call(Operation::TriggeredDefendingPlayer, vec![])),
        "TargetedController" => Ok(call(
            Operation::ControllerOf,
            vec![call(Operation::Target, vec![call(Operation::Any, vec![])])],
        )),
        "ReplacedCard" => Ok(call(Operation::Triggered, vec![])),
        _ => Err(unsupported_value("Defined", value)),
    }
}

fn defined_player_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    match value {
        "You" => Ok(call(Operation::You, vec![])),
        "Opponent" | "Player.Opponent" => Ok(call(Operation::Opponent, vec![])),
        "Player" => Ok(call(Operation::Any, vec![])),
        "Targeted" | "TargetedPlayer" => {
            Ok(call(Operation::Target, vec![call(Operation::Any, vec![])]))
        }
        "TargetedController" => Ok(call(
            Operation::ControllerOf,
            vec![call(Operation::Target, vec![call(Operation::Any, vec![])])],
        )),
        "TriggeredCardController" => Ok(call(
            Operation::ControllerOf,
            vec![call(Operation::Triggered, vec![])],
        )),
        "TriggeredPlayer" => Ok(call(Operation::TriggeredPlayer, vec![])),
        "TriggeredTarget" | "TriggeredTargetLKICopy" => {
            Ok(call(Operation::TriggeredTarget, vec![]))
        }
        "TriggeredActivator" => Ok(call(Operation::TriggeredActivator, vec![])),
        "TriggeredDefendingPlayer" => Ok(call(Operation::TriggeredDefendingPlayer, vec![])),
        "ParentTarget" => Ok(call(Operation::ParentTarget, vec![])),
        _ => Err(unsupported_value("Defined", value)),
    }
}

pub(crate) fn valid_target_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    if value == "Any" {
        return Ok(call(Operation::Target, vec![call(Operation::Any, vec![])]));
    }
    Ok(call(
        Operation::Target,
        vec![affected_selector(value).map_err(|_| unsupported_value("ValidTgts", value))?],
    ))
}

pub(crate) fn card_selector_in_zone(
    value: &str,
    zone: &str,
) -> Result<Expression, MappingDiagnostic> {
    let mut predicates = Vec::new();
    for branch in value.split(',') {
        if branch == "Basic" {
            predicates.push(call(
                Operation::SupertypeIs,
                vec![Expression::Text("basic".to_string())],
            ));
            continue;
        }
        if branch == "Card.IsRemembered" {
            predicates.push(
                object_marker_predicate("IsRemembered")
                    .unwrap_or_else(|| unreachable!("closed remembered marker must lower")),
            );
            continue;
        }
        let branch = if zone == "battlefield" {
            branch.to_string()
        } else {
            branch
                .replace("YouCtrl", "YouOwn")
                .replace("OppCtrl", "OppOwn")
        };
        let Expression::Call {
            operation,
            mut arguments,
        } = affected_selector_branch(&branch)?
        else {
            return Err(unsupported_value("ValidCards", value));
        };
        if !matches!(operation, Operation::Cards | Operation::Permanents) || arguments.len() > 1 {
            return Err(unsupported_value("ValidCards", value));
        }
        predicates.push(arguments.pop().unwrap_or(Expression::Boolean(true)));
    }
    let predicate = match predicates.len() {
        0 => return Err(unsupported_value("ValidCards", value)),
        1 => predicates.remove(0),
        _ => call(Operation::Or, predicates),
    };
    let selector = call(Operation::Cards, vec![predicate]);
    add_collection_predicate(
        selector,
        call(Operation::ZoneIs, vec![Expression::Text(zone.to_string())]),
    )
}

fn valid_cards_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    if value == "Permanent" {
        return Ok(call(Operation::Permanents, vec![]));
    }
    let mut predicates = Vec::new();
    for card_type in value.split(',') {
        if card_type.is_empty()
            || card_type.contains(['.', '+', ' '])
            || !matches!(
                card_type,
                "Artifact" | "Battle" | "Creature" | "Enchantment" | "Land" | "Planeswalker"
            )
        {
            return Err(unsupported_value("ValidCards", value));
        }
        predicates.push(call(
            Operation::TypeIs,
            vec![Expression::Text(card_type.to_ascii_lowercase())],
        ));
    }
    let predicate = match predicates.len() {
        0 => return Err(unsupported_value("ValidCards", value)),
        1 => predicates.remove(0),
        _ => call(Operation::Or, predicates),
    };
    Ok(call(Operation::Permanents, vec![predicate]))
}

fn spell_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    let mut alternatives = Vec::new();
    for branch in value.split(',') {
        alternatives.push(spell_predicate(branch)?);
    }
    if alternatives
        .iter()
        .any(|predicate| matches!(predicate, Expression::Boolean(true)))
    {
        return Ok(call(Operation::Spells, vec![]));
    }
    let predicate = match alternatives.len() {
        0 => return Err(unsupported_value("ValidCard", value)),
        1 => alternatives.remove(0),
        _ => call(Operation::Or, alternatives),
    };
    Ok(call(Operation::Spells, vec![predicate]))
}

fn spell_predicate(value: &str) -> Result<Expression, MappingDiagnostic> {
    let mut pieces = value.split('.');
    let base = pieces.next().unwrap_or_default();
    let modifiers = pieces
        .flat_map(|part| part.split('+'))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if base.is_empty() {
        return Err(unsupported_value("ValidCard", value));
    }
    let mut predicates = Vec::new();
    if matches!(base, "Card" | "Spell") {
        if modifiers.is_empty() {
            return Ok(Expression::Boolean(true));
        }
    } else {
        predicates.push(type_or_subtype_predicate(base, "ValidCard", value)?);
    }
    for modifier in modifiers {
        let predicate = if matches!(
            modifier,
            "Artifact"
                | "Battle"
                | "Creature"
                | "Enchantment"
                | "Instant"
                | "Land"
                | "Planeswalker"
                | "Sorcery"
        ) {
            call(
                Operation::TypeIs,
                vec![Expression::Text(modifier.to_ascii_lowercase())],
            )
        } else if matches!(modifier, "White" | "Blue" | "Black" | "Red" | "Green") {
            call(
                Operation::ColorIs,
                vec![Expression::Text(modifier.to_ascii_lowercase())],
            )
        } else if modifier == "nonCreature" {
            call(
                Operation::Not,
                vec![call(
                    Operation::TypeIs,
                    vec![Expression::Text("creature".to_string())],
                )],
            )
        } else if modifier == "Self" {
            call(
                Operation::Equals,
                vec![
                    call(Operation::Any, vec![]),
                    call(Operation::Source, vec![]),
                ],
            )
        } else if modifier == "YouCtrl" {
            call(Operation::ControlledBy, vec![call(Operation::You, vec![])])
        } else if modifier == "YouOwn" {
            call(Operation::OwnedBy, vec![call(Operation::You, vec![])])
        } else if modifier == "OppCtrl" {
            call(
                Operation::ControlledBy,
                vec![call(Operation::Opponent, vec![])],
            )
        } else if modifier == "OppOwn" {
            call(Operation::OwnedBy, vec![call(Operation::Opponent, vec![])])
        } else if modifier == "ControlledBy TriggeredDefendingPlayer" {
            call(
                Operation::ControlledBy,
                vec![call(Operation::TriggeredDefendingPlayer, vec![])],
            )
        } else if modifier == "OwnedBy TriggeredDefendingPlayer" {
            call(
                Operation::OwnedBy,
                vec![call(Operation::TriggeredDefendingPlayer, vec![])],
            )
        } else if let Some(predicate) = kicked_predicate(modifier) {
            predicate
        } else if let Some(predicate) = object_marker_predicate(modifier) {
            predicate
        } else if modifier
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
        {
            call(
                Operation::SubtypeIs,
                vec![Expression::Text(modifier.to_ascii_lowercase())],
            )
        } else {
            return Err(unsupported_value("ValidCard", value));
        };
        predicates.push(predicate);
    }
    match predicates.len() {
        0 => Ok(Expression::Boolean(true)),
        1 => Ok(predicates.remove(0)),
        _ => Ok(call(Operation::And, predicates)),
    }
}

fn type_or_subtype_predicate(
    value: &str,
    key: &str,
    original: &str,
) -> Result<Expression, MappingDiagnostic> {
    let operation = if matches!(
        value,
        "Artifact"
            | "Battle"
            | "Creature"
            | "Enchantment"
            | "Instant"
            | "Land"
            | "Planeswalker"
            | "Sorcery"
    ) {
        Operation::TypeIs
    } else if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric())
    {
        Operation::SubtypeIs
    } else {
        return Err(unsupported_value(key, original));
    };
    Ok(call(
        operation,
        vec![Expression::Text(value.to_ascii_lowercase())],
    ))
}

pub(crate) fn affected_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    let mut selectors = Vec::new();
    for branch in value.split(',') {
        selectors.push(affected_selector_branch(branch)?);
    }
    match selectors.len() {
        0 => Err(unsupported_value("Affected", value)),
        1 => Ok(selectors.remove(0)),
        _ => Ok(call(Operation::All, selectors)),
    }
}

fn affected_selector_branch(value: &str) -> Result<Expression, MappingDiagnostic> {
    if value == "You" {
        return Ok(call(Operation::You, vec![]));
    }
    if value == "Opponent" {
        return Ok(call(Operation::Opponent, vec![]));
    }
    if matches!(value, "Any" | "Player") {
        return Ok(call(Operation::Any, vec![]));
    }
    if value == "Card.IsRemembered" {
        return Ok(call(
            Operation::Remembered,
            vec![call(Operation::Any, vec![])],
        ));
    }
    if matches!(value, "Card.Self" | "Creature.Self" | "Self") {
        return Ok(call(Operation::Source, vec![]));
    }
    if matches!(value, "Card.EquippedBy" | "Creature.EquippedBy") {
        return Ok(call(
            Operation::EquippedObject,
            vec![call(Operation::Source, vec![])],
        ));
    }
    if matches!(value, "Card.EnchantedBy" | "Creature.EnchantedBy") {
        return Ok(call(
            Operation::EnchantedObject,
            vec![call(Operation::Source, vec![])],
        ));
    }

    let (base, modifiers) = value.split_once('.').unwrap_or((value, ""));
    if base.is_empty() {
        return Err(unsupported_value("Affected", value));
    }
    let mut predicates = Vec::new();
    if base != "Card" && base != "Permanent" {
        let operation = if matches!(
            base,
            "Artifact" | "Battle" | "Creature" | "Enchantment" | "Land" | "Planeswalker"
        ) {
            Operation::TypeIs
        } else if base
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
        {
            Operation::SubtypeIs
        } else {
            return Err(unsupported_value("Affected", value));
        };
        predicates.push(call(
            operation,
            vec![Expression::Text(base.to_ascii_lowercase())],
        ));
    }
    if !modifiers.is_empty() {
        for modifier in modifiers.split('+') {
            let predicate = match modifier {
                "YouCtrl" => call(Operation::ControlledBy, vec![call(Operation::You, vec![])]),
                "YouOwn" => call(Operation::OwnedBy, vec![call(Operation::You, vec![])]),
                "OppCtrl" => call(
                    Operation::ControlledBy,
                    vec![call(Operation::Opponent, vec![])],
                ),
                "OppOwn" => call(Operation::OwnedBy, vec![call(Operation::Opponent, vec![])]),
                "ControlledBy TriggeredDefendingPlayer" => call(
                    Operation::ControlledBy,
                    vec![call(Operation::TriggeredDefendingPlayer, vec![])],
                ),
                "OwnedBy TriggeredDefendingPlayer" => call(
                    Operation::OwnedBy,
                    vec![call(Operation::TriggeredDefendingPlayer, vec![])],
                ),
                "YouDontCtrl" => call(
                    Operation::Not,
                    vec![call(
                        Operation::ControlledBy,
                        vec![call(Operation::You, vec![])],
                    )],
                ),
                "Other" | "StrictlyOther" => call(
                    Operation::Not,
                    vec![call(
                        Operation::Equals,
                        vec![
                            call(Operation::Any, vec![]),
                            call(Operation::Source, vec![]),
                        ],
                    )],
                ),
                "Artifact" | "Battle" | "Creature" | "Enchantment" | "Instant" | "Land"
                | "Planeswalker" | "Sorcery" => call(
                    Operation::TypeIs,
                    vec![Expression::Text(modifier.to_ascii_lowercase())],
                ),
                "White" | "Blue" | "Black" | "Red" | "Green" | "Colorless" => call(
                    Operation::ColorIs,
                    vec![Expression::Text(modifier.to_ascii_lowercase())],
                ),
                "inZoneBattlefield" | "inRealZoneBattlefield" => call(
                    Operation::ZoneIs,
                    vec![Expression::Text("battlefield".to_string())],
                ),
                "Basic" | "Legendary" => call(
                    Operation::SupertypeIs,
                    vec![Expression::Text(modifier.to_ascii_lowercase())],
                ),
                "IsCommander" => call(
                    Operation::DesignationIs,
                    vec![Expression::Text("commander".to_string())],
                ),
                "nonLand" => call(
                    Operation::Not,
                    vec![call(
                        Operation::TypeIs,
                        vec![Expression::Text("land".to_string())],
                    )],
                ),
                "nonCreature" => call(
                    Operation::Not,
                    vec![call(
                        Operation::TypeIs,
                        vec![Expression::Text("creature".to_string())],
                    )],
                ),
                "nonHuman" => call(
                    Operation::Not,
                    vec![call(
                        Operation::SubtypeIs,
                        vec![Expression::Text("human".to_string())],
                    )],
                ),
                "nonArtifact" => call(
                    Operation::Not,
                    vec![call(
                        Operation::TypeIs,
                        vec![Expression::Text("artifact".to_string())],
                    )],
                ),
                "withFlying" => call(
                    Operation::KeywordIs,
                    vec![Expression::Text("flying".to_string())],
                ),
                "withoutFlying" => call(
                    Operation::Not,
                    vec![call(
                        Operation::KeywordIs,
                        vec![Expression::Text("flying".to_string())],
                    )],
                ),
                "attacking" | "blocking" | "tapped" => call(
                    Operation::DesignationIs,
                    vec![Expression::Text(modifier.to_string())],
                ),
                "kicked" | "kicked 1" | "kicked 2" => kicked_predicate(modifier)
                    .unwrap_or_else(|| unreachable!("closed kicked value must lower")),
                "ChosenType" => call(Operation::ChosenTypeIs, vec![]),
                "token" | "!token" | "IsRemembered" => object_marker_predicate(modifier)
                    .unwrap_or_else(|| unreachable!("closed object marker must lower")),
                literal_subtype
                    if literal_subtype
                        .chars()
                        .next()
                        .is_some_and(char::is_uppercase)
                        && literal_subtype.chars().all(|character| {
                            character.is_ascii_alphanumeric() || character == '-'
                        })
                        && !literal_subtype.starts_with("Is")
                        && !literal_subtype.starts_with("Chosen") =>
                {
                    call(
                        Operation::SubtypeIs,
                        vec![Expression::Text(literal_subtype.to_ascii_lowercase())],
                    )
                }
                _ => {
                    if let Some(predicate) = closed_numeric_predicate(modifier) {
                        predicate?
                    } else if let Some(predicate) = closed_negated_predicate(modifier) {
                        predicate
                    } else if let Some(predicate) = closed_keyword_predicate(modifier) {
                        predicate
                    } else {
                        return Err(unsupported_value("Affected", value));
                    }
                }
            };
            predicates.push(predicate);
        }
    }
    let predicate = match predicates.len() {
        0 => None,
        1 => Some(predicates.remove(0)),
        _ => Some(call(Operation::And, predicates)),
    };
    let operation = if base == "Card" {
        Operation::Cards
    } else {
        Operation::Permanents
    };
    Ok(call(operation, predicate.into_iter().collect()))
}

fn kicked_predicate(value: &str) -> Option<Expression> {
    let designation = match value {
        "kicked" => "kicked",
        "kicked 1" => "kicked_1",
        "kicked 2" => "kicked_2",
        _ => return None,
    };
    Some(call(
        Operation::DesignationIs,
        vec![Expression::Text(designation.to_string())],
    ))
}

fn object_marker_predicate(value: &str) -> Option<Expression> {
    let token = || {
        call(
            Operation::DesignationIs,
            vec![Expression::Text("token".to_string())],
        )
    };
    match value {
        "token" => Some(token()),
        "!token" => Some(call(Operation::Not, vec![token()])),
        "IsRemembered" => Some(call(
            Operation::Equals,
            vec![
                call(Operation::Any, vec![]),
                call(Operation::Remembered, vec![call(Operation::Any, vec![])]),
            ],
        )),
        _ => None,
    }
}

fn closed_numeric_predicate(value: &str) -> Option<Result<Expression, MappingDiagnostic>> {
    let (operation, comparison) = [
        ("power", Operation::Power),
        ("toughness", Operation::Toughness),
        ("cmc", Operation::ManaValue),
    ]
    .into_iter()
    .find_map(|(prefix, operation)| {
        value
            .strip_prefix(prefix)
            .map(|comparison| (operation, comparison))
    })?;
    let (comparison, amount_text) =
        ["GE", "LE", "EQ", "GT", "LT"]
            .into_iter()
            .find_map(|prefix| {
                comparison
                    .strip_prefix(prefix)
                    .map(|amount| (prefix, amount))
            })?;
    let amount = match amount_text.parse::<i64>() {
        Ok(amount) => amount,
        Err(_) => return Some(Err(unsupported_value("Affected", value))),
    };
    let subject = call(operation, vec![call(Operation::Any, vec![])]);
    let predicate = match comparison {
        "GE" => call(
            Operation::AtLeast,
            vec![subject, Expression::Integer(amount)],
        ),
        "LE" => match amount.checked_add(1) {
            Some(exclusive) => call(
                Operation::LessThan,
                vec![subject, Expression::Integer(exclusive)],
            ),
            None => return Some(Err(unsupported_value("Affected", value))),
        },
        "EQ" => call(
            Operation::Equals,
            vec![subject, Expression::Integer(amount)],
        ),
        "GT" => call(
            Operation::GreaterThan,
            vec![subject, Expression::Integer(amount)],
        ),
        "LT" => call(
            Operation::LessThan,
            vec![subject, Expression::Integer(amount)],
        ),
        _ => return Some(Err(unsupported_value("Affected", value))),
    };
    Some(Ok(predicate))
}

fn closed_negated_predicate(value: &str) -> Option<Expression> {
    let excluded = value.strip_prefix("non")?;
    if !excluded.chars().next().is_some_and(char::is_uppercase)
        || !excluded
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return None;
    }
    let operation = if matches!(excluded, "White" | "Blue" | "Black" | "Red" | "Green") {
        Operation::ColorIs
    } else if matches!(
        excluded,
        "Artifact"
            | "Battle"
            | "Creature"
            | "Enchantment"
            | "Instant"
            | "Land"
            | "Planeswalker"
            | "Sorcery"
    ) {
        Operation::TypeIs
    } else {
        Operation::SubtypeIs
    };
    Some(call(
        Operation::Not,
        vec![call(
            operation,
            vec![Expression::Text(excluded.to_ascii_lowercase())],
        )],
    ))
}

fn closed_keyword_predicate(value: &str) -> Option<Expression> {
    let (keyword, negated) = value
        .strip_prefix("without")
        .map(|keyword| (keyword, true))
        .or_else(|| value.strip_prefix("with").map(|keyword| (keyword, false)))?;
    let normalized = normalize_simple_keyword(keyword).ok()?;
    let predicate = call(Operation::KeywordIs, vec![Expression::Text(normalized)]);
    Some(if negated {
        call(Operation::Not, vec![predicate])
    } else {
        predicate
    })
}

fn normalize_simple_keyword(value: &str) -> Result<String, MappingDiagnostic> {
    let normalized = value.trim().to_ascii_lowercase().replace(' ', "_");
    if matches!(
        normalized.as_str(),
        "deathtouch"
            | "defender"
            | "double_strike"
            | "fear"
            | "first_strike"
            | "flash"
            | "flying"
            | "haste"
            | "hexproof"
            | "indestructible"
            | "intimidate"
            | "lifelink"
            | "menace"
            | "prowess"
            | "reach"
            | "shadow"
            | "shroud"
            | "skulk"
            | "trample"
            | "vigilance"
    ) {
        Ok(normalized)
    } else {
        Err(unsupported_value("AddKeyword", value))
    }
}

fn pump_duration(
    parameters: &BTreeMap<String, String>,
) -> Result<Option<&'static str>, MappingDiagnostic> {
    match parameters.get("Duration").map(String::as_str) {
        None | Some("UntilEndOfTurn") => Ok(Some("until_end_of_turn")),
        Some("UntilYourNextTurn") => Ok(Some("until_your_next_turn")),
        Some("Permanent") => Ok(None),
        Some(value) => Err(unsupported_value("Duration", value)),
    }
}

fn append_keyword_grants(
    effects: &mut Vec<Expression>,
    affected: &Expression,
    keywords: Option<&String>,
    duration: Option<&str>,
) -> Result<(), MappingDiagnostic> {
    let Some(keywords) = keywords else {
        return Ok(());
    };
    for keyword in keywords.split(" & ") {
        let restrictions = match keyword {
            "HIDDEN CARDNAME can't attack." => Some([Operation::CannotAttack].as_slice()),
            "HIDDEN CARDNAME can't block." => Some([Operation::CannotBlock].as_slice()),
            "HIDDEN CARDNAME can't attack or block." => {
                Some([Operation::CannotAttack, Operation::CannotBlock].as_slice())
            }
            _ => None,
        };
        if is_next_untap_restriction(keyword) {
            effects.push(call(
                Operation::CannotUntap,
                vec![
                    affected.clone(),
                    Expression::Text("next_untap_step".to_string()),
                ],
            ));
            continue;
        }
        if let Some(restrictions) = restrictions {
            effects.extend(
                restrictions
                    .iter()
                    .map(|operation| call(*operation, vec![affected.clone()])),
            );
        } else {
            let mut arguments = vec![
                affected.clone(),
                Expression::Text(normalize_simple_keyword(keyword)?),
            ];
            if let Some(duration) = duration {
                arguments.push(Expression::Text(duration.to_string()));
            }
            effects.push(call(Operation::GrantKeyword, arguments));
        }
    }
    Ok(())
}

fn is_next_untap_restriction(value: &str) -> bool {
    matches!(
        value,
        "HIDDEN This card doesn't untap during your next untap step."
            | "HIDDEN CARDNAME doesn't untap during your next untap step."
    )
}

fn optional_number_or_value(
    parameters: &BTreeMap<String, String>,
    key: &str,
    fallback: Operation,
) -> Result<Expression, MappingDiagnostic> {
    parameters.get(key).map_or_else(
        || Ok(call(fallback, vec![call(Operation::Any, vec![])])),
        |value| {
            value
                .parse::<i64>()
                .map(Expression::Integer)
                .map_err(|_| unsupported_value(key, value))
        },
    )
}

fn append_text_effects(
    effects: &mut Vec<Expression>,
    operation: Operation,
    value: &str,
    key: &str,
) -> Result<(), MappingDiagnostic> {
    let values = value.split(" & ").map(str::trim).collect::<Vec<_>>();
    if values.is_empty()
        || values
            .iter()
            .any(|value| value.is_empty() || value.contains(','))
    {
        return Err(unsupported_value(key, value));
    }
    for value in values {
        effects.push(call(
            operation,
            vec![
                call(Operation::Any, vec![]),
                Expression::Text(value.to_string()),
            ],
        ));
    }
    Ok(())
}

fn parse_closed_colors(value: &str) -> Result<Vec<String>, MappingDiagnostic> {
    let colors = value.split(" & ").map(str::trim).collect::<Vec<_>>();
    if colors.is_empty()
        || colors.len() > 2
        || colors.iter().any(|color| {
            !matches!(
                *color,
                "White" | "Blue" | "Black" | "Red" | "Green" | "Colorless"
            )
        })
    {
        return Err(unsupported_value("SetColor", value));
    }
    Ok(colors
        .into_iter()
        .map(|color| color.to_ascii_lowercase())
        .collect())
}

fn combine_effects(
    mut effects: Vec<Expression>,
    missing_message: &str,
) -> Result<Expression, MappingDiagnostic> {
    match effects.len() {
        0 => Err(diagnostic("MISSING_PARAMETER", missing_message)),
        1 => Ok(effects.remove(0)),
        _ => Ok(call(Operation::Sequence, effects)),
    }
}

fn require_battlefield_zone(
    parameters: &BTreeMap<String, String>,
    key: &str,
) -> Result<(), MappingDiagnostic> {
    if parameters
        .get(key)
        .map_or(true, |zone| zone == "Battlefield")
    {
        Ok(())
    } else {
        Err(unsupported_value(key, required(parameters, key)?))
    }
}

fn require_static_effect_zone(
    parameters: &BTreeMap<String, String>,
    key: &str,
) -> Result<(), MappingDiagnostic> {
    if parameters.get(key).map_or(true, |zone| {
        matches!(zone.as_str(), "Battlefield" | "All" | "Command")
    }) {
        Ok(())
    } else {
        Err(unsupported_value(key, required(parameters, key)?))
    }
}

fn reject_unknown(
    parameters: &BTreeMap<String, String>,
    allowed: &[&str],
) -> Result<(), MappingDiagnostic> {
    if let Some(key) = parameters
        .keys()
        .find(|key| !allowed.contains(&key.as_str()) && !is_nonsemantic_metadata(key))
    {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            &format!("parameter `{key}` has no typed mapper"),
        ));
    }
    Ok(())
}

fn is_nonsemantic_metadata(key: &str) -> bool {
    // Legacy observes Hidden only in ChangeZoneEffect; that mapper preserves it explicitly.
    matches!(
        key,
        "AILogic"
            | "AITgts"
            | "CostDesc"
            | "IsCurse"
            | "Hidden"
            | "Planeswalker"
            | "PreCostDesc"
            | "PrecostDesc"
            | "SpellDescription"
            | "StackDescription"
            | "TgtPrompt"
            | "Ultimate"
            | "ValidDescription"
            | "ValidTgtsDesc"
    )
}

fn required<'a>(
    parameters: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, MappingDiagnostic> {
    parameters.get(key).map(String::as_str).ok_or_else(|| {
        diagnostic(
            "MISSING_PARAMETER",
            &format!("required parameter `{key}` is absent"),
        )
    })
}

fn optional_positive_integer(
    parameters: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<i64>, MappingDiagnostic> {
    parameters
        .get(key)
        .map(|value| positive_integer(value, key))
        .transpose()
}

fn optional_signed_integer(
    parameters: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<i64>, MappingDiagnostic> {
    parameters
        .get(key)
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| unsupported_value(key, value))
        })
        .transpose()
}

fn positive_integer(value: &str, key: &str) -> Result<i64, MappingDiagnostic> {
    value
        .parse::<i64>()
        .ok()
        .filter(|amount| *amount > 0)
        .ok_or_else(|| unsupported_value(key, value))
}

fn require_selector(actual: &str, expected: &str) -> Result<(), MappingDiagnostic> {
    require_selector_one_of(actual, &[expected])
}

fn require_selector_one_of(actual: &str, expected: &[&str]) -> Result<(), MappingDiagnostic> {
    if expected.contains(&actual) {
        Ok(())
    } else {
        Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            &format!("selector `{actual}` is not supported by this mapper"),
        ))
    }
}

fn call(operation: Operation, arguments: Vec<Expression>) -> Expression {
    Expression::Call {
        operation,
        arguments,
    }
}

fn unsupported_value(key: &str, value: &str) -> MappingDiagnostic {
    diagnostic(
        "UNSUPPORTED_VALUE",
        &format!("parameter `{key}` value `{value}` has no exact lowering"),
    )
}

fn diagnostic(code: &str, message: &str) -> MappingDiagnostic {
    MappingDiagnostic {
        code: code.to_string(),
        message: message.to_string(),
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{
        audit_legacy_mappings, collect_script_mapping_blockers, map_legacy_ability,
        map_legacy_ability_in_context, MappingContext,
    };
    use crate::legacy::{parse_legacy_script, LegacyLineKind};
    use forge_carddef::{Expression, Operation};
    use std::{fs, path::Path, process::Command};

    #[test]
    fn maps_simple_mana_draw_and_damage_abilities() {
        assert_operation(
            "A:AB$ Mana | Cost$ 2 G T | Produced$ Combo G W | SpellDescription$ Add mana.",
            Operation::AddMana,
            2,
        );
        let command_identity = map_line(
            "A:AB$ Mana | Cost$ T | Produced$ Combo ColorIdentity | SpellDescription$ Add commander-identity mana.",
        )
        .unwrap_or_else(|error| panic!("commander identity mana should map: {}", error.message));
        assert!(matches!(
            command_identity.expression,
            Expression::Call {
                operation: Operation::AddMana,
                ref arguments,
            } if arguments == &vec![
                Expression::Text("command_color_identity".to_string()),
                super::call(Operation::You, vec![]),
            ]
        ));
        assert_operation(
            "A:SP$ Draw | NumCards$ 2 | SpellDescription$ Draw two cards.",
            Operation::Draw,
            0,
        );
        assert_operation(
            "A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3 | SpellDescription$ Deal 3 damage.",
            Operation::DealDamage,
            0,
        );
        assert_operation(
            "A:DB$ DealDamage | Defined$ You | NumDmg$ 1",
            Operation::DealDamage,
            0,
        );
    }

    #[test]
    fn lowers_restricted_mana_library_dig_and_graveyard_self_moves() {
        let restricted = map_line(
            "A:AB$ Mana | Cost$ T | Produced$ Any | Amount$ 2 | RestrictValid$ Spell.Instant,Spell.Sorcery | SpellDescription$ Add mana.",
        )
        .unwrap_or_else(|error| panic!("restricted mana should map: {}", error.message));
        assert!(matches!(
            restricted.expression,
            Expression::Call {
                operation: Operation::AddRestrictedMana,
                ref arguments,
            } if arguments == &vec![
                Expression::Text("any_color".to_string()),
                super::call(Operation::You, vec![]),
                Expression::Text("Spell.Instant,Spell.Sorcery".to_string()),
                Expression::Integer(2),
            ]
        ));

        let dig = map_line(
            "A:SP$ Dig | DigNum$ 5 | ChangeNum$ 1 | Optional$ True | ForceRevealToController$ True | ChangeValid$ Card.Creature | RestRandomOrder$ True | SpellDescription$ Look.",
        )
        .unwrap_or_else(|error| panic!("closed Dig should map: {}", error.message));
        let Expression::Call {
            operation: Operation::LibraryDig,
            arguments,
        } = dig.expression
        else {
            panic!("Dig should lower to library_dig");
        };
        assert_eq!(arguments.len(), 7);
        assert_eq!(arguments[1], Expression::Integer(5));
        assert_eq!(arguments[2], Expression::Integer(1));
        assert_eq!(arguments[4], Expression::Text("hand".to_string()));
        assert_eq!(arguments[5], Expression::Text("library".to_string()));

        map_line(
            "A:SP$ Dig | DigNum$ 5 | ChangeNum$ 1 | ChangeValid$ Creature | DestinationZone$ Battlefield | DestinationZone2$ Library | LibraryPosition$ -1 | RestRandomOrder$ True | SpellDescription$ Look.",
        )
        .unwrap_or_else(|error| panic!("default ignored Dig position should map: {}", error.message));
        assert!(matches!(
            &arguments[6],
            Expression::Text(options)
                if options.contains("optional=true")
                    && options.contains("forcerevealtocontroller=true")
                    && options.contains("restrandomorder=true")
        ));

        let graveyard = map_line(
            "A:AB$ ChangeZone | Cost$ 2 B | Origin$ Graveyard | Destination$ Hand | ActivationZone$ Graveyard | SpellDescription$ Return this card.",
        )
        .unwrap_or_else(|error| panic!("graveyard self move should map: {}", error.message));
        assert!(matches!(
            graveyard.expression,
            Expression::Call {
                operation: Operation::MoveZoneFrom,
                ref arguments,
            } if arguments == &vec![
                super::call(Operation::Source, vec![]),
                Expression::Text("graveyard".to_string()),
                Expression::Text("hand".to_string()),
            ]
        ));
        assert!(matches!(
            graveyard.timing,
            Some(Expression::Call {
                operation: Operation::TimingCondition,
                ..
            })
        ));
    }

    #[test]
    fn maps_closed_dig_until_partitions() {
        for line in [
            "A:DB$ DigUntil | Valid$ Card.Land | FoundDestination$ Hand | RevealedDestination$ Library | RevealedLibraryPosition$ -1 | RevealRandomOrder$ True",
            "A:SP$ DigUntil | ValidTgts$ Opponent | Amount$ 4 | Valid$ Land | RevealedDestination$ Graveyard | IsCurse$ True",
            "A:DB$ DigUntil | Defined$ Opponent | Valid$ Card.nonLand | FoundDestination$ Exile | RevealedDestination$ Exile | RememberFound$ True",
        ] {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("DigUntil should map: {}", error.message));
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::LibraryDigUntil
            ));
        }
    }

    #[test]
    fn maps_closed_library_seeks() {
        for line in [
            "A:DB$ Seek | Type$ Card.nonLand",
            "A:DB$ Seek | Num$ 2 | Type$ Forest | RememberFound$ True",
            "A:SP$ Seek | Defined$ Opponent | Type$ Instant,Sorcery | ImprintFound$ True",
        ] {
            let mapped =
                map_line(line).unwrap_or_else(|error| panic!("Seek should map: {}", error.message));
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::SeekLibrary
            ));
        }
    }

    #[test]
    fn source_bound_closed_zone_moves_retain_their_origin_guard() {
        let mapped = map_line(
            "A:DB$ ChangeZone | Origin$ Graveyard | Destination$ Exile | SpellDescription$ Exile this card.",
        )
        .unwrap_or_else(|error| panic!("closed source move should map: {}", error.message));
        assert!(matches!(
            mapped.expression,
            Expression::Call {
                operation: Operation::MoveZoneFrom,
                ref arguments,
            } if arguments == &vec![
                super::call(Operation::Source, vec![]),
                Expression::Text("graveyard".to_string()),
                Expression::Text("exile".to_string()),
            ]
        ));
    }

    #[test]
    fn defined_closed_zone_moves_retain_their_origin_guards() {
        for line in [
            "A:DB$ ChangeZone | Origin$ Exile | Destination$ Hand | Defined$ ChosenCard",
            "A:DB$ ChangeZone | Origin$ Graveyard | Destination$ Battlefield | Defined$ Remembered",
            "A:DB$ ChangeZone | Origin$ Stack | Destination$ Exile | Defined$ ParentTarget",
            "A:DB$ ChangeZone | Origin$ Library | Destination$ Hand | Defined$ Remembered",
        ] {
            let mapped = map_line(line).unwrap_or_else(|error| {
                panic!("closed defined move should map: {}", error.message)
            });
            assert!(matches!(
                mapped.expression,
                Expression::Call {
                    operation: Operation::MoveZoneFrom,
                    ..
                }
            ));
        }
    }

    #[test]
    fn lowers_regeneration_targeted_casts_and_generic_optional_effects() {
        assert_operation(
            "A:AB$ Regenerate | Cost$ G | SpellDescription$ Regenerate this permanent.",
            Operation::RegenerateShield,
            1,
        );

        let targeted = map_script_root(concat!(
            "Name:Heroic\n",
            "T:Mode$ SpellCast | ValidCard$ Instant,Sorcery | ValidActivatingPlayer$ You | TargetsValid$ Card.Self | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
            "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
        ))
        .unwrap_or_else(|error| panic!("targeted cast trigger should map: {}", error.message));
        assert!(matches!(
            targeted.event,
            Some(Expression::Call {
                operation: Operation::EventCastTargeting,
                ref arguments,
            }) if arguments.len() == 4
                && arguments[3] == Expression::Text("cast".to_string())
        ));

        let optional = map_script_root(concat!(
            "Name:Optional Move\n",
            "A:SP$ ChangeZone | Origin$ Battlefield | Destination$ Hand | ValidTgts$ Creature | Optional$ True | SubAbility$ DBDraw | SpellDescription$ Return.\n",
            "SVar:DBDraw:DB$ Draw | Defined$ You\n",
        ))
        .unwrap_or_else(|error| panic!("optional move should map: {}", error.message));
        assert!(matches!(
            optional.expression,
            Expression::Call {
                operation: Operation::Sequence,
                ref arguments,
            } if matches!(
                arguments.first(),
                Some(Expression::Call {
                    operation: Operation::ChooseUpTo,
                    ..
                })
            ) && matches!(
                arguments.get(1),
                Some(Expression::Call {
                    operation: Operation::Draw,
                    ..
                })
            )
        ));

        let optional_decider =
            map_line("A:DB$ GainLife | Defined$ You | LifeAmount$ 2 | OptionalDecider$ You")
                .unwrap_or_else(|error| {
                    panic!("owner optional effect should map: {}", error.message)
                });
        assert!(expression_contains_operation(
            &optional_decider.expression,
            Operation::ChooseUpTo
        ));
        let optional_you = map_line(
            "A:AB$ ChangeZone | Cost$ T | Origin$ Hand | Destination$ Battlefield | ChangeType$ Land | ChangeNum$ 1 | Optional$ You | SpellDescription$ You may put a land onto the battlefield.",
        )
        .unwrap_or_else(|error| panic!("Optional You should map: {}", error.message));
        assert!(expression_contains_operation(
            &optional_you.expression,
            Operation::ChooseUpTo
        ));
        assert!(map_line(
            "A:DB$ GainLife | Defined$ You | LifeAmount$ 2 | OptionalDecider$ Opponent"
        )
        .is_err());

        let dig = map_line(
            "A:SP$ Dig | DigNum$ 2 | ChangeNum$ 1 | Optional$ True | SpellDescription$ Dig.",
        )
        .unwrap_or_else(|error| {
            panic!(
                "optional Dig should retain Dig semantics: {}",
                error.message
            )
        });
        assert!(matches!(
            dig.expression,
            Expression::Call {
                operation: Operation::LibraryDig,
                ..
            }
        ));

        let error =
            map_line("A:SP$ Draw | Defined$ You | Optional$ False | SpellDescription$ Draw.")
                .err()
                .unwrap_or_else(|| panic!("non-true Optional must fail closed"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");

        let secondary = map_line(
            "A:SP$ Draw | Defined$ You | NumCards$ 1 | Secondary$ True | SpellDescription$ Draw.",
        )
        .unwrap_or_else(|error| {
            panic!("secondary composition marker should map: {}", error.message)
        });
        assert!(expression_contains_operation(
            &secondary.expression,
            Operation::Draw
        ));
        assert!(map_line(
            "A:SP$ Draw | Defined$ You | NumCards$ 1 | Secondary$ False | SpellDescription$ Draw."
        )
        .is_err());
        let named = map_line(
            "A:SP$ Draw | Name$ Display-only ability label | Defined$ You | NumCards$ 1 | SpellDescription$ Draw.",
        )
        .unwrap_or_else(|error| panic!("ability display name should map: {}", error.message));
        assert!(expression_contains_operation(
            &named.expression,
            Operation::Draw
        ));
    }

    #[test]
    fn scopes_closed_zone_moves_to_the_defined_player() {
        let mapped = map_line(
            "A:SP$ ChangeZone | Origin$ Hand | Destination$ Exile | ValidTgts$ Opponent | DefinedPlayer$ Targeted | ChangeType$ Card | ChangeNum$ 1 | Hidden$ True | SpellDescription$ Exile.",
        )
        .unwrap_or_else(|error| panic!("defined-player move should map: {}", error.message));
        for operation in [
            Operation::HiddenInformation,
            Operation::MoveZone,
            Operation::OwnedBy,
            Operation::Target,
        ] {
            assert!(expression_contains_operation(&mapped.expression, operation));
        }

        let error = map_line(
            "A:SP$ ChangeZone | Origin$ Hand | Destination$ Exile | DefinedPlayer$ Remembered | ChangeType$ Card | ChangeNum$ 1",
        )
        .err()
        .unwrap_or_else(|| panic!("open DefinedPlayer binding must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");

        let error = map_line(
            "A:SP$ ChangeZone | Origin$ Hand | Destination$ Exile | DefinedPlayer$ Targeted | ChangeType$ Card | ChangeNum$ 1",
        )
        .err()
        .unwrap_or_else(|| panic!("unbound targeted player must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_SELECTOR");

        let error = map_line(
            "A:SP$ ChangeZone | Origin$ Library | Destination$ Hand | DefinedPlayer$ Targeted | ChangeType$ Card | ChangeNum$ 1",
        )
        .err()
        .unwrap_or_else(|| panic!("unbound targeted library owner must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_SELECTOR");

        let error = map_line(
            "A:SP$ ChangeZone | Origin$ Hand | Destination$ Exile | DefinedPlayer$ Player | ChangeType$ Card | ChangeNum$ 2",
        )
        .err()
        .unwrap_or_else(|| panic!("per-player cardinality must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");

        for player in ["Opponent", "Player.Opponent"] {
            let error = map_line(&format!(
                "A:SP$ ChangeZone | Origin$ Hand | Destination$ Battlefield | DefinedPlayer$ {player} | ChangeType$ Creature | ChangeNum$ 1"
            ))
            .err()
            .unwrap_or_else(|| panic!("aggregate opponent cardinality must quarantine"));
            assert_eq!(error.code, "UNSUPPORTED_VALUE");
        }

        let battlefield = map_line(
            "A:SP$ ChangeZone | Origin$ Battlefield | Destination$ Hand | ValidTgts$ Opponent | DefinedPlayer$ Targeted | ChangeType$ Creature | ChangeNum$ 1",
        )
        .unwrap_or_else(|error| panic!("target-player battlefield move should map: {}", error.message));
        assert!(expression_contains_operation(
            &battlefield.expression,
            Operation::ControlledBy
        ));
        assert!(!expression_contains_operation(
            &battlefield.expression,
            Operation::OwnedBy
        ));
    }

    #[test]
    fn lowers_dynamic_restricted_mana_and_dig_counts() {
        let mana = map_script_root(concat!(
            "Name:Restricted Dynamic Mana\n",
            "A:AB$ Mana | Cost$ T | Produced$ G | Amount$ X | RestrictValid$ Spell.Creature | SpellDescription$ Mana.\n",
            "SVar:X:Count$Valid Elf.YouCtrl\n",
        ))
        .unwrap_or_else(|error| panic!("dynamic restricted mana should map: {}", error.message));
        assert!(matches!(
            mana.expression,
            Expression::Call {
                operation: Operation::AddRestrictedMana,
                ref arguments,
            } if arguments.get(3).is_some_and(|value| {
                expression_contains_operation(value, Operation::Count)
            })
        ));

        let dig = map_script_root(concat!(
            "Name:Dynamic Dig\n",
            "A:SP$ Dig | DigNum$ X | ChangeNum$ Y | ChangeValid$ Card.Creature | SpellDescription$ Dig.\n",
            "SVar:X:Count$Valid Creature.YouCtrl\n",
            "SVar:Y:Count$Valid Land.YouCtrl\n",
        ))
        .unwrap_or_else(|error| panic!("dynamic Dig should map: {}", error.message));
        assert!(matches!(
            dig.expression,
            Expression::Call {
                operation: Operation::LibraryDig,
                ref arguments,
            } if [1_usize, 2].into_iter().all(|index| {
                arguments.get(index).is_some_and(|value| {
                    expression_contains_operation(value, Operation::Count)
                })
            })
        ));
    }

    #[test]
    fn maps_closed_reflected_mana() {
        let mapped = map_line(
            "A:AB$ ManaReflected | Cost$ T | ColorOrType$ Color | Valid$ Land.OppCtrl | ReflectProperty$ Produce | SpellDescription$ Add reflected mana.",
        )
        .unwrap_or_else(|error| panic!("reflected mana should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::AddReflectedMana
        ));

        let error = map_line(
            "A:AB$ ManaReflected | Cost$ T | ColorOrType$ Color | Valid$ Land.OppCtrl | ReflectProperty$ Is | SpellDescription$ Add reflected mana.",
        )
        .err()
        .unwrap_or_else(|| panic!("open reflected property must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn maps_closed_library_reordering() {
        let mapped = map_line(
            "A:SP$ RearrangeTopOfLibrary | Defined$ You | NumCards$ 5 | MayShuffle$ True | SpellDescription$ Reorder.",
        )
        .unwrap_or_else(|error| panic!("library reorder should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::ReorderLibraryTop
        ));

        let dynamic = map_script_root(concat!(
            "Name:Dynamic Reorder\n",
            "A:AB$ RearrangeTopOfLibrary | Cost$ T | Defined$ You | NumCards$ X | SpellDescription$ Reorder.\n",
            "SVar:X:Count$Valid Wizard\n",
        ))
        .unwrap_or_else(|error| panic!("dynamic reorder should map: {}", error.message));
        assert!(expression_contains_operation(
            &dynamic.expression,
            Operation::Count
        ));
    }

    #[test]
    fn maps_literal_card_conjuring() {
        let mapped = map_line(
            "A:DB$ MakeCard | Conjure$ True | Name$ Mox Ruby | Zone$ Hand | RememberMade$ True",
        )
        .unwrap_or_else(|error| panic!("literal conjure should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::ConjureCard
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Remember
        ));

        let repeated = map_line(
            "A:DB$ MakeCard | TokenCard$ True | Name$ Reassembling Skeleton | Zone$ Graveyard | Amount$ 4",
        )
        .unwrap_or_else(|error| panic!("repeated token-card creation should map: {}", error.message));
        assert!(matches!(
            repeated.expression,
            Expression::Call {
                operation: Operation::Sequence,
                ref arguments,
            } if arguments.len() == 4 && arguments.iter().all(|argument| {
                expression_contains_operation(argument, Operation::ConjureCard)
            })
        ));

        let error = map_line("A:DB$ MakeCard | Conjure$ True | Name$ ChosenName | Zone$ Hand")
            .err()
            .unwrap_or_else(|| panic!("dynamic conjure name must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn maps_closed_card_attributes() {
        for line in [
            "A:DB$ AlterAttribute | Attributes$ Suspected",
            "A:DB$ AlterAttribute | Defined$ Remembered | Attributes$ Plotted",
            "A:DB$ AlterAttribute | Defined$ Enchanted | Attributes$ Suspected | Activate$ False",
        ] {
            assert_operation(line, Operation::AlterAttribute, 0);
        }
        let error = map_line("A:DB$ AlterAttribute | Attributes$ Unknown")
            .err()
            .unwrap_or_else(|| panic!("unknown attribute must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn maps_kicked_legacy_condition() {
        let mapped = map_line(
            "A:DB$ GainLife | Defined$ You | LifeAmount$ 2 | Condition$ Kicked | SpellDescription$ Gain life.",
        )
        .unwrap_or_else(|error| panic!("kicked condition should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::TimesKicked
        ));
    }

    #[test]
    fn maps_triggered_pay_to_apply_effects() {
        let mapped = map_script_root(concat!(
            "Name:Triggered Payment\n",
            "T:Mode$ LifeGained | ValidPlayer$ You | TriggerZones$ Battlefield | OptionalDecider$ You | Execute$ TrigDraw | TriggerDescription$ Pay to draw.\n",
            "SVar:TrigDraw:AB$ Draw | Cost$ 2 | Defined$ You | NumCards$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("triggered payment should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::PayToApply
        ));
    }

    #[test]
    fn maps_linked_exile_play_permissions() {
        let mapped = map_script_root(concat!(
            "Name:Linked Play Permission\n",
            "A:DB$ Effect | RememberObjects$ RememberedCard | StaticAbilities$ STPlay | ForgetOnMoved$ Exile | Duration$ UntilTheEndOfYourNextTurn | SubAbility$ DBCleanup\n",
            "SVar:STPlay:Mode$ Continuous | MayPlay$ True | Affected$ Card.IsRemembered | AffectedZone$ Exile | Description$ Play it.\n",
            "SVar:DBCleanup:DB$ Cleanup | ClearRemembered$ True\n",
        ))
        .unwrap_or_else(|error| panic!("linked play permission should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::PlayPermission
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Forget
        ));

        let static_effect = map_script_root(concat!(
            "Name:Linked Static Effect\n",
            "A:SP$ Effect | StaticAbilities$ STHandSize | Duration$ Permanent | SpellDescription$ No maximum hand size.\n",
            "SVar:STHandSize:Mode$ Continuous | Affected$ You | SetMaxHandSize$ Unlimited | Description$ No maximum hand size.\n",
        ))
        .unwrap_or_else(|error| panic!("linked static effect should map: {}", error.message));
        assert!(expression_contains_operation(
            &static_effect.expression,
            Operation::RegisterEffectStatic
        ));
        assert!(expression_contains_operation(
            &static_effect.expression,
            Operation::NoMaximumHandSize
        ));
    }

    #[test]
    fn maps_linked_continuous_activated_abilities() {
        let mapped = map_script_root(concat!(
            "Name:Granted Activated Ability\n",
            "S:Mode$ Continuous | Affected$ Creature.YouCtrl | AddAbility$ ABPing | AddSVar$ DBGain | Description$ Creatures gain an activated ability.\n",
            "SVar:ABPing:AB$ DealDamage | Cost$ T | ValidTgts$ Any | NumDmg$ 1 | SubAbility$ DBGain | SpellDescription$ Ping.\n",
            "SVar:DBGain:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("linked granted ability should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GrantActivatedAbility
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::DealDamage
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GainLife
        ));
    }

    #[test]
    fn maps_linked_continuous_triggered_abilities() {
        let mapped = map_script_root(concat!(
            "Name:Granted Triggered Ability\n",
            "S:Mode$ Continuous | Affected$ Creature.YouCtrl | AddTrigger$ TrigUpkeep | AddSVar$ DBLife | Description$ Creatures gain a trigger.\n",
            "SVar:TrigUpkeep:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | TriggerZones$ Battlefield | Execute$ DBLife | TriggerDescription$ Gain life.\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("linked granted trigger should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GrantTriggeredAbility
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GainLife
        ));
    }

    #[test]
    fn maps_linked_continuous_static_abilities_across_all_zones() {
        let mapped = map_script_root(concat!(
            "Name:Granted Static Ability\n",
            "S:Mode$ Continuous | Affected$ Creature.YouCtrl | AffectedZone$ All | AddStaticAbility$ STVigilance | Description$ Creatures have vigilance.\n",
            "SVar:STVigilance:Mode$ Continuous | Affected$ Card.Self | AddKeyword$ Vigilance | Description$ This creature has vigilance.\n",
        ))
        .unwrap_or_else(|error| panic!("linked granted static should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GrantStaticAbility
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GrantKeyword
        ));
        assert!(matches!(
            mapped.expression,
            Expression::Call {
                operation: Operation::Continuous,
                ref arguments,
            } if arguments.get(2) == Some(&Expression::Text("all".to_string()))
        ));
    }

    #[test]
    fn maps_linked_continuous_replacement_abilities() {
        let mapped = map_script_root(concat!(
            "Name:Granted Replacement Ability\n",
            "S:Mode$ Continuous | Affected$ Creature.EnchantedBy | AddReplacementEffect$ RepDamage | Description$ It has a replacement.\n",
            "SVar:RepDamage:Event$ DamageDone | ActiveZones$ Battlefield | Prevent$ True | ValidTarget$ Card.Self | Description$ Prevent damage.\n",
        ))
        .unwrap_or_else(|error| panic!("linked replacement should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GrantReplacementAbility
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::PreventDamage
        ));
    }

    #[test]
    fn maps_animated_static_abilities() {
        let mapped = map_script_root(concat!(
            "Name:Animated Static Ability\n",
            "A:AB$ Animate | Cost$ 1 | Defined$ Self | Types$ Artifact,Creature,Construct | staticAbilities$ STPower | SpellDescription$ Animate.\n",
            "SVar:STPower:Mode$ Continuous | Affected$ Card.Self | SetPower$ 3 | SetToughness$ 3 | Description$ It is 3/3.\n",
        ))
        .unwrap_or_else(|error| panic!("animated static should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GrantStaticAbility
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::SetPt
        ));
    }

    #[test]
    fn maps_command_effect_triggers_with_lifetimes() {
        let mapped = map_script_root(concat!(
            "Name:Temporary Trigger Effect\n",
            "A:AB$ Effect | Cost$ 1 W B | Triggers$ TrigLife | SpellDescription$ Whenever you gain life this turn, an opponent loses that much life.\n",
            "SVar:TrigLife:Mode$ LifeGained | ValidPlayer$ You | TriggerZones$ Battlefield | Execute$ DBLose | TriggerDescription$ Lose life.\n",
            "SVar:DBLose:DB$ LoseLife | Defined$ Opponent | LifeAmount$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("temporary trigger effect should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::RegisterEffectTrigger
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::LoseLife
        ));

        let remembered = map_script_root(concat!(
            "Name:Remembered Trigger Effect\n",
            "A:SP$ Effect | ValidTgts$ Creature.YouCtrl | RememberObjects$ Targeted | ExileOnMoved$ Battlefield | Triggers$ TrigDamage | Duration$ Permanent | SpellDescription$ Remember it.\n",
            "SVar:TrigDamage:Mode$ DamageDone | ValidSource$ Card.IsRemembered | ValidTarget$ Player | TriggerZones$ Battlefield | Execute$ DBDraw | TriggerDescription$ Draw.\n",
            "SVar:DBDraw:DB$ Draw | Defined$ You | NumCards$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("remembered trigger effect should map: {}", error.message));
        assert!(expression_contains_operation(
            &remembered.expression,
            Operation::RegisterEffectTrigger
        ));
        assert!(expression_contains_operation(
            &remembered.expression,
            Operation::Target
        ));

        let replacement = map_script_root(concat!(
            "Name:Replacement Effect\n",
            "A:SP$ Effect | ReplacementEffects$ RPrevent | Duration$ UntilEndOfTurn | SpellDescription$ Prevent damage.\n",
            "SVar:RPrevent:Event$ DamageDone | Prevent$ True | IsCombat$ True | ValidTarget$ You | ActiveZones$ Command | Description$ Prevent combat damage.\n",
        ))
        .unwrap_or_else(|error| panic!("replacement effect should map: {}", error.message));
        assert!(expression_contains_operation(
            &replacement.expression,
            Operation::RegisterEffectReplacement
        ));
        assert!(expression_contains_operation(
            &replacement.expression,
            Operation::PreventDamage
        ));
    }

    #[test]
    fn maps_animated_trigger_grants_and_perpetual_duration() {
        let mapped = map_script_root(concat!(
            "Name:Animated Trigger\n",
            "A:SP$ Animate | Defined$ Self | Types$ Creature | Power$ 3 | Toughness$ 3 | Triggers$ TrigDies | Duration$ Permanent | SpellDescription$ Animate.\n",
            "SVar:TrigDies:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self | TriggerZones$ Battlefield | Execute$ DBDraw | TriggerDescription$ Draw.\n",
            "SVar:DBDraw:DB$ Draw | Defined$ You | NumCards$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("animated trigger should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GrantTriggeredAbility
        ));

        let perpetual = map_script_root(concat!(
            "Name:Perpetual Trigger\n",
            "A:SP$ AnimateAll | ValidCards$ Creature.YouCtrl | Triggers$ TrigUpkeep | Duration$ Perpetual | SpellDescription$ Grant perpetually.\n",
            "SVar:TrigUpkeep:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | TriggerZones$ Battlefield | Execute$ DBLife | TriggerDescription$ Gain life.\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("perpetual trigger should map: {}", error.message));
        assert!(expression_contains_operation(
            &perpetual.expression,
            Operation::Perpetual
        ));
        assert!(expression_contains_operation(
            &perpetual.expression,
            Operation::GrantTriggeredAbility
        ));
    }

    #[test]
    fn maps_target_binding_pump_shells() {
        let mapped = map_script_root(concat!(
            "Name:Target Binding Shell\n",
            "A:SP$ Pump | ValidTgts$ Instant.YouOwn,Sorcery.YouOwn | TgtZone$ Graveyard | SubAbility$ DBLife | SpellDescription$ Choose a card, then gain life.\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("target-binding shell should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::BindTargets
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Target
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GainLife
        ));
    }

    #[test]
    fn maps_nonbattlefield_targets_for_shared_effects() {
        for (line, operation) in [
            (
                "A:SP$ PutCounter | ValidTgts$ Card | TgtZone$ Exile | CounterType$ TIME | CounterNum$ 1",
                Operation::AddCounter,
            ),
            (
                "A:DB$ Animate | ValidTgts$ Card.nonLand | TgtZone$ Hand | Keywords$ Flying | Duration$ Perpetual",
                Operation::GrantKeyword,
            ),
            (
                "A:SP$ ChangeZone | ValidTgts$ Card | TgtZone$ Exile | Origin$ Exile | Destination$ Hand",
                Operation::MoveZoneFrom,
            ),
        ] {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("nonbattlefield target should map: {}", error.message));
            assert!(expression_contains_operation(&mapped.expression, operation));
        }
    }

    #[test]
    fn maps_closed_damage_replacements() {
        let prevent = map_script_root(concat!(
            "Name:Damage Prevention\n",
            "R:Event$ DamageDone | ActiveZones$ Battlefield | Prevent$ True | IsCombat$ True | ValidTarget$ Creature.YouCtrl | Description$ Prevent combat damage.\n",
        ))
        .unwrap_or_else(|error| panic!("damage prevention should map: {}", error.message));
        assert!(matches!(
            prevent.event,
            Some(Expression::Call {
                operation: Operation::EventDamage,
                ..
            })
        ));
        assert!(expression_contains_operation(
            &prevent.expression,
            Operation::PreventDamage
        ));

        let replaced = map_script_root(concat!(
            "Name:Damage Replacement\n",
            "R:Event$ DamageDone | ActiveZones$ Battlefield | ValidTarget$ Card.Self | ReplaceWith$ DBLife | ReplacementResult$ Updated | Description$ Gain life instead.\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("linked damage replacement should map: {}", error.message));
        assert!(expression_contains_operation(
            &replaced.expression,
            Operation::GainLife
        ));

        let replace_dying = map_line(
            "A:SP$ DealDamage | ValidTgts$ Creature | NumDmg$ 5 | ReplaceDyingDefined$ Targeted",
        )
        .unwrap_or_else(|error| panic!("replace-dying damage should map: {}", error.message));
        assert!(expression_contains_operation(
            &replace_dying.expression,
            Operation::ExileIfDies
        ));
    }

    #[test]
    fn maps_closed_replacement_amount_updates() {
        for script in [
            "A:DB$ ReplaceEffect | VarName$ DamageAmount | VarValue$ 2\n",
            "A:DB$ ReplaceEffect | VarName$ DamageAmount | VarValue$ ReplaceCount$DamageAmount/Twice\n",
            "A:DB$ ReplaceEffect | VarName$ Number | VarValue$ X\nSVar:X:ReplaceCount$Number/Plus.1\n",
        ] {
            let mapped = map_script_root(script)
                .unwrap_or_else(|error| panic!("ReplaceEffect should map: {}", error.message));
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::UpdateReplacementAmount
            ));
        }
    }

    #[test]
    fn maps_closed_dice_rolls() {
        for line in [
            "A:AB$ RollDice | Cost$ 2 | Sides$ 6 | ResultSVar$ Result | SpellDescription$ Roll a d6.",
            "A:SP$ RollDice | Amount$ 2 | Sides$ 12 | ChosenSVar$ X | OtherSVar$ Y | SpellDescription$ Choose a result.",
            "A:DB$ RollDice | Sides$ 6 | ToVisitYourAttractions$ True",
        ] {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("closed dice roll should map: {}", error.message));
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::RollDice
            ));
        }
        assert!(map_line("A:AB$ RollDice | Sides$ 1").is_err());
        assert!(map_line("A:AB$ RollDice | Amount$ 3 | ChosenSVar$ X | OtherSVar$ Y").is_err());

        let dynamic = map_script_root(concat!(
            "Name:Roll Result Binding\n",
            "A:AB$ RollDice | Cost$ T | Sides$ 6 | ResultSVar$ X | SubAbility$ DBLife | SpellDescription$ Gain life equal to the result.\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ X\n",
        ))
        .unwrap_or_else(|error| panic!("roll result binding should map: {}", error.message));
        assert!(expression_contains_operation(
            &dynamic.expression,
            Operation::RollResult
        ));
        assert!(expression_contains_operation(
            &dynamic.expression,
            Operation::GainLife
        ));

        let table = map_script_root(concat!(
            "Name:Roll Table\n",
            "A:SP$ RollDice | Sides$ 20 | ResultSubAbilities$ 1-9:DBLife,10-19:DBDraw,20:DBBoth | SpellDescription$ Roll a d20.\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
            "SVar:DBDraw:DB$ Draw | Defined$ You | NumCards$ 1\n",
            "SVar:DBBoth:DB$ GainLife | Defined$ You | LifeAmount$ 2 | SubAbility$ DBDraw\n",
        ))
        .unwrap_or_else(|error| panic!("roll table should map: {}", error.message));
        assert!(expression_contains_operation(
            &table.expression,
            Operation::RollDiceTable
        ));
        assert!(expression_contains_operation(
            &table.expression,
            Operation::Draw
        ));
    }

    #[test]
    fn maps_closed_peek_and_reveal() {
        let mapped = map_script_root(concat!(
            "Name:Peek and Reveal\n",
            "A:SP$ PeekAndReveal | PeekAmount$ 5 | NoPeek$ True | RememberRevealed$ True | SubAbility$ DBLife | SpellDescription$ Reveal five.\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("peek and reveal should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::PeekLibrary
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GainLife
        ));

        let look = map_line(
            "A:AB$ PeekAndReveal | Cost$ 1 | ValidTgts$ Player | NoReveal$ True | RememberPeeked$ True | SpellDescription$ Look.",
        )
        .unwrap_or_else(|error| panic!("private peek should map: {}", error.message));
        assert!(expression_contains_operation(
            &look.expression,
            Operation::PeekLibrary
        ));
    }

    #[test]
    fn preserves_static_trigger_execution() {
        let mapped = map_script_root(concat!(
            "Name:Static Trigger\n",
            "T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self | Static$ True | Execute$ DBCleanup | TriggerDescription$ Clean up immediately.\n",
            "SVar:DBCleanup:DB$ Cleanup | ClearRemembered$ True\n",
        ))
        .unwrap_or_else(|error| panic!("static trigger should map: {}", error.message));
        assert!(matches!(
            mapped.event,
            Some(Expression::Call {
                operation: Operation::EventStatic,
                ..
            })
        ));
    }

    #[test]
    fn maps_variant_format_trigger_events() {
        for (script, operation) in [
            (
                concat!(
                    "T:Mode$ ChaosEnsues | TriggerZones$ Command | Execute$ DBDraw\n",
                    "SVar:DBDraw:DB$ Draw | Defined$ You | NumCards$ 1\n",
                ),
                Operation::EventChaosEnsues,
            ),
            (
                concat!(
                    "T:Mode$ SetInMotion | ValidCard$ Card.Self | TriggerZones$ Command | Execute$ DBDraw\n",
                    "SVar:DBDraw:DB$ Draw | Defined$ You | NumCards$ 1\n",
                ),
                Operation::EventSetInMotion,
            ),
        ] {
            let mapped = map_script_root(script)
                .unwrap_or_else(|error| panic!("variant trigger should map: {}", error.message));
            assert!(mapped.event.as_ref().is_some_and(|event| {
                expression_contains_operation(event, operation)
            }));
        }
    }

    #[test]
    fn maps_target_groups_alone_attacks_and_created_attachments() {
        let grouped = map_script_root(
            "A:SP$ Destroy | ValidTgts$ Creature | TargetMin$ 2 | TargetMax$ 2 | TargetUnique$ True | TargetsWithSameController$ True\n",
        )
        .unwrap_or_else(|error| panic!("grouped targets should map: {}", error.message));
        assert!(expression_contains_operation(
            &grouped.expression,
            Operation::UniqueTarget
        ));
        assert!(expression_contains_operation(
            &grouped.expression,
            Operation::TargetGroup
        ));

        let alone = map_script_root(concat!(
            "T:Mode$ Attacks | ValidCard$ Creature.YouCtrl | Alone$ True | Execute$ DBPump\n",
            "SVar:DBPump:DB$ Pump | Defined$ TriggeredAttacker | NumAtt$ 1 | NumDef$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("alone attack should map: {}", error.message));
        assert!(alone
            .event
            .as_ref()
            .is_some_and(|event| { expression_contains_operation(event, Operation::EventAlone) }));

        let token = map_line(
            "A:DB$ Token | TokenScript$ role_royal | TokenOwner$ You | AttachedTo$ Targeted | ValidTgts$ Creature.YouCtrl",
        )
        .unwrap_or_else(|error| panic!("attached token should map: {}", error.message));
        assert!(expression_contains_operation(
            &token.expression,
            Operation::AttachCreated
        ));

        let attach =
            map_line("A:DB$ Attach | Object$ ParentTarget | Move$ True | Choices$ Permanent")
                .unwrap_or_else(|error| panic!("chosen attachment should map: {}", error.message));
        assert!(expression_contains_operation(
            &attach.expression,
            Operation::AttachChoice
        ));
    }

    #[test]
    fn maps_source_filtered_triggers_reduced_costs_switched_payments_and_etb_effects() {
        let trigger = map_script_root(concat!(
            "T:Mode$ BecomesTarget | ValidTarget$ Card.Self | ValidSource$ SpellAbility.OppCtrl | FirstTime$ True | Execute$ DBDraw\n",
            "SVar:DBDraw:DB$ Draw | Defined$ You | NumCards$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("source-filtered trigger should map: {}", error.message));
        assert!(trigger.event.as_ref().is_some_and(|event| {
            expression_contains_operation(event, Operation::EventSource)
                && expression_contains_operation(event, Operation::EventLimit)
        }));

        let reduced = map_script_root(concat!(
            "A:AB$ Scry | Cost$ 5 T | ScryNum$ 1 | ReduceCost$ X\n",
            "SVar:X:Count$Valid Creature.YouCtrl\n",
        ))
        .unwrap_or_else(|error| panic!("reduced activation should map: {}", error.message));
        assert!(reduced
            .costs
            .iter()
            .any(|cost| expression_contains_operation(cost, Operation::ReduceCostBy)));

        let switched = map_script_root(
            "A:SP$ Draw | NumCards$ 1 | UnlessCost$ Discard<1/Card> | UnlessPayer$ You | UnlessSwitched$ True\n",
        )
        .unwrap_or_else(|error| panic!("switched optional cost should map: {}", error.message));
        assert!(expression_contains_operation(
            &switched.expression,
            Operation::PayToApply
        ));

        let etb = map_script_root(
            "A:DB$ PutCounter | ETB$ True | Defined$ ReplacedCard | CounterType$ P1P1 | CounterNum$ 1\n",
        )
        .unwrap_or_else(|error| panic!("ETB counter effect should map: {}", error.message));
        assert!(expression_contains_operation(
            &etb.expression,
            Operation::EtbEffect
        ));
    }

    #[test]
    fn maps_turn_bound_triggers_power_ups_reordering_and_repeated_effects() {
        let turn_trigger = map_script_root(concat!(
            "Name:Turn Trigger\n",
            "T:Mode$ LifeGained | ValidPlayer$ You | TriggerZones$ Battlefield | PlayerTurn$ True | Execute$ DBLife\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("turn-bound trigger should map: {}", error.message));
        assert!(turn_trigger
            .event
            .as_ref()
            .is_some_and(|event| { expression_contains_operation(event, Operation::EventWhen) }));

        let power_up = map_script_root(
            "A:AB$ PutCounter | Cost$ 5 G | Defined$ Self | CounterType$ P1P1 | CounterNum$ 1 | PowerUp$ True\n",
        )
        .unwrap_or_else(|error| panic!("power-up should map: {}", error.message));
        assert!(power_up
            .costs
            .iter()
            .any(|cost| expression_contains_operation(cost, Operation::PowerUpCost)));

        let exhaust =
            map_script_root("A:AB$ Draw | Cost$ 2 | Defined$ You | NumCards$ 1 | Exhaust$ True\n")
                .unwrap_or_else(|error| panic!("exhaust ability should map: {}", error.message));
        assert!(exhaust
            .costs
            .iter()
            .any(|cost| expression_contains_operation(cost, Operation::ExhaustCost)));

        let revolt =
            map_script_root("A:DB$ GainLife | Defined$ You | LifeAmount$ 2 | Revolt$ True\n")
                .unwrap_or_else(|error| panic!("revolt condition should map: {}", error.message));
        assert!(expression_contains_operation(
            &revolt.expression,
            Operation::RevoltOccurred
        ));

        let reordered = map_script_root(
            "A:DB$ ChangeZone | Origin$ Hand | Destination$ Library | ChangeNum$ 2 | Mandatory$ True | Reorder$ True\n",
        )
        .unwrap_or_else(|error| panic!("ordered library move should map: {}", error.message));
        assert!(expression_contains_operation(
            &reordered.expression,
            Operation::ReorderMoved
        ));

        let repeated = map_script_root(concat!(
            "Name:Repeated Copy\n",
            "A:DB$ CopySpellAbility | Defined$ TriggeredSpellAbility | Amount$ X | MayChooseTarget$ True\n",
            "SVar:X:Count$Valid Creature.YouCtrl\n",
        ))
        .unwrap_or_else(|error| panic!("dynamic repeated copy should map: {}", error.message));
        assert!(expression_contains_operation(
            &repeated.expression,
            Operation::RepeatEffect
        ));
        assert!(expression_contains_operation(
            &repeated.expression,
            Operation::Count
        ));

        let untap = map_line("A:DB$ Untap | UntapUpTo$ True | UntapType$ Land.YouCtrl | Amount$ 3")
            .unwrap_or_else(|error| panic!("bounded untap choice should map: {}", error.message));
        assert!(expression_contains_operation(
            &untap.expression,
            Operation::ChooseObjects
        ));

        let clone = map_line(
            "A:DB$ Clone | Choices$ Creature | ChoiceZone$ Battlefield | AddTypes$ Phyrexian & Artifact",
        )
        .unwrap_or_else(|error| panic!("clone type additions should map: {}", error.message));
        assert!(expression_contains_operation(
            &clone.expression,
            Operation::AddType
        ));

        let search = map_script_root(concat!(
            "Name:Dynamic Search\n",
            "A:SP$ ChangeZone | Origin$ Library | Destination$ Hand | ChangeType$ Card | ChangeNum$ X\n",
            "SVar:X:Count$Valid Creature.YouCtrl\n",
        ))
        .unwrap_or_else(|error| panic!("dynamic search count should map: {}", error.message));
        assert!(expression_contains_operation(
            &search.expression,
            Operation::SearchLibrary
        ));
        assert!(expression_contains_operation(
            &search.expression,
            Operation::Count
        ));
    }

    #[test]
    fn maps_closed_card_name_choices() {
        let mapped = map_script_root(concat!(
            "A:SP$ NameCard | Defined$ You | ValidCards$ Card.nonLand | ValidDescription$ nonland | SubAbility$ DBDraw\n",
            "SVar:DBDraw:DB$ Draw | Defined$ You | NumCards$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("card-name choice should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::ChooseCardName
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Draw
        ));

        let random = map_line("A:DB$ NameCard | AtRandom$ True | ValidCards$ Creature")
            .unwrap_or_else(|error| panic!("random card name should map: {}", error.message));
        assert!(expression_contains_operation(
            &random.expression,
            Operation::ChooseCardName
        ));
    }

    #[test]
    fn maps_closed_amass_effects() {
        let mapped = map_line("A:DB$ Amass | Type$ Zombie | Num$ 2 | RememberAmass$ True")
            .unwrap_or_else(|error| panic!("amass should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Amass
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Remember
        ));

        let dynamic = map_script_root(concat!(
            "Name:Dynamic Amass\n",
            "A:SP$ Amass | Type$ Orc | Num$ X | SpellDescription$ Amass.\n",
            "SVar:X:Count$xPaid\n",
        ))
        .unwrap_or_else(|error| panic!("dynamic amass should map: {}", error.message));
        assert!(expression_contains_operation(
            &dynamic.expression,
            Operation::PaidX
        ));
    }

    #[test]
    fn maps_remember_targets_before_effects() {
        let mapped = map_script_root(concat!(
            "Name:Remember Targets\n",
            "A:SP$ Destroy | ValidTgts$ Creature | RememberTargets$ True | SpellDescription$ Destroy and remember.\n",
        ))
        .unwrap_or_else(|error| panic!("remember targets should map: {}", error.message));
        assert!(matches!(
            mapped.expression,
            Expression::Call {
                operation: Operation::Sequence,
                ref arguments,
            } if matches!(
                arguments.first(),
                Some(Expression::Call {
                    operation: Operation::Remember,
                    ..
                })
            ) && expression_contains_operation(&arguments[1], Operation::Destroy)
        ));

        let error = map_script_root(concat!(
            "Name:Invalid Remember Targets\n",
            "A:SP$ Destroy | ValidTgts$ Creature | RememberTargets$ Sometimes | SpellDescription$ Invalid.\n",
        ))
        .err()
        .unwrap_or_else(|| panic!("open remember-targets value must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn maps_remembered_library_partition_without_researching() {
        let mapped = map_script_root(concat!(
            "Name:Remembered Search Partition\n",
            "A:SP$ ChangeZone | Origin$ Library | Destination$ Library | ChangeType$ Land.Basic | ChangeNum$ 2 | RememberChanged$ True | Reveal$ True | Shuffle$ False | SubAbility$ DBOne | SpellDescription$ Search.\n",
            "SVar:DBOne:DB$ ChangeZone | Origin$ Library | Destination$ Battlefield | ChangeType$ Land.IsRemembered | ChangeNum$ 1 | Mandatory$ True | NoLooking$ True | Tapped$ True | Shuffle$ False | SubAbility$ DBTwo\n",
            "SVar:DBTwo:DB$ ChangeZone | Origin$ Library | Destination$ Graveyard | ChangeType$ Land.IsRemembered | Mandatory$ True | NoLooking$ True\n",
        ))
        .unwrap_or_else(|error| panic!("remembered partition should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::ChooseObjects
        ));
    }

    #[test]
    fn replaces_creature_subtypes_before_adding_new_types() {
        let mapped = map_line(
            "S:Mode$ Continuous | Affected$ Creature.EnchantedBy | AddType$ Demon & Spirit | RemoveCreatureTypes$ True | Description$ Replace creature types.",
        )
        .unwrap_or_else(|error| panic!("creature type replacement should map: {}", error.message));
        let Expression::Call {
            operation: Operation::Continuous,
            arguments,
        } = mapped.expression
        else {
            panic!("continuous type replacement should remain continuous");
        };
        let Expression::Call {
            operation: Operation::Sequence,
            arguments: effects,
        } = &arguments[1]
        else {
            panic!("type replacement should be an ordered sequence");
        };
        assert!(matches!(
            effects.first(),
            Some(Expression::Call {
                operation: Operation::RemoveType,
                arguments,
            }) if arguments.get(1) == Some(&Expression::Text("creature_subtypes".to_string()))
        ));
        assert!(matches!(
            effects.get(1),
            Some(Expression::Call {
                operation: Operation::AddType,
                ..
            })
        ));

        for line in [
            "A:SP$ Animate | Defined$ Self | Types$ Demon,Spirit | RemoveCreatureTypes$ True | Duration$ Permanent",
            "A:SP$ AnimateAll | ValidCards$ Creature.YouCtrl | Types$ Demon,Spirit | RemoveCreatureTypes$ True | Duration$ Permanent",
        ] {
            let mapped = map_script_root(line).unwrap_or_else(|error| {
                panic!("animated type replacement should map: {}", error.message)
            });
            assert!(has_ordered_type_replacement(&mapped.expression));
        }
    }

    #[test]
    fn new_batch_values_still_fail_closed() {
        for (line, key) in [
            (
                "A:AB$ Mana | Produced$ G | RestrictValid$ Runtime.Arbitrary",
                "RestrictValid",
            ),
            (
                "A:AB$ Mana | Produced$ G | RestrictValid$ Spell.Runtime.Arbitrary",
                "RestrictValid",
            ),
            (
                "A:AB$ Mana | Produced$ G | RestrictValid$ Activated.Unknown_Property",
                "RestrictValid",
            ),
            (
                "A:SP$ Dig | DigNum$ 3 | Tapped$ True | SpellDescription$ Invalid destination.",
                "Tapped",
            ),
            (
                "S:Mode$ Continuous | Affected$ Card.Self | AddType$ Elf | RemoveCreatureTypes$ False",
                "RemoveCreatureTypes",
            ),
        ] {
            let error = map_line(line)
                .err()
                .unwrap_or_else(|| panic!("{key} fixture must quarantine"));
            assert!(error.message.contains(key), "{}", error.message);
        }
    }

    #[test]
    fn maps_closed_protection_effects_and_rejects_open_forms() {
        for (line, expected_costs) in [
            (
                "A:SP$ Protection | ValidTgts$ Creature.YouCtrl | Gains$ Choice | Choices$ AnyColor",
                0,
            ),
            (
                "A:AB$ Protection | Cost$ R | Defined$ Self | Gains$ red",
                1,
            ),
            (
                "A:DB$ Protection | Defined$ Self | Gains$ green,white | Duration$ Permanent",
                0,
            ),
        ] {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("closed protection should map: {}", error.message));
            assert_eq!(mapped.costs.len(), expected_costs);
            assert!(matches!(
                mapped.expression,
                Expression::Call {
                    operation: Operation::GrantProtection,
                    ..
                }
            ));
        }

        for line in [
            "A:SP$ Protection | Defined$ Self | Gains$ Choice",
            "A:SP$ Protection | Defined$ Self | Gains$ red | Choices$ AnyColor",
            "A:SP$ Protection | Defined$ Self | Gains$ red | Duration$ UntilYourNextTurn",
        ] {
            assert!(
                map_line(line).is_err(),
                "open protection form must quarantine"
            );
        }
    }

    #[test]
    fn maps_closed_type_choices_and_rejects_open_domains() {
        for line in [
            "A:DB$ ChooseType | Defined$ You | Type$ Creature",
            "A:DB$ ChooseType | Type$ Basic Land",
            "A:AB$ ChooseType | Cost$ 1 | Defined$ You | Type$ Card",
            "A:DB$ ChooseType | Defined$ You | Type$ Card | ValidTypes$ Artifact,Enchantment,Instant,Sorcery,Planeswalker",
            "A:DB$ ChooseType | Defined$ You | Type$ Card | InvalidTypes$ Creature,Land",
            "A:DB$ ChooseColor | Defined$ You",
            "A:DB$ ChooseColor | Defined$ You | Exclude$ green",
        ] {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("closed type choice should map: {}", error.message));
            assert!(matches!(
                mapped.expression,
                Expression::Call {
                    operation: Operation::ChooseType,
                    ..
                }
            ));
        }

        for line in [
            "A:DB$ ChooseType | Defined$ You | Type$ Shared",
            "A:DB$ ChooseType | Defined$ You | Type$ Creature | ValidTypes$ Elf,RuntimeArbitrary",
            "A:DB$ ChooseType | Defined$ You | Type$ Creature | ChooseType2$ True",
            "A:DB$ ChooseColor | Defined$ You | Exclude$ colorless",
        ] {
            assert!(map_line(line).is_err(), "open type choice must quarantine");
        }
    }

    #[test]
    fn maps_simple_primitive_effect_pack() {
        for (line, operation, costs) in [
            (
                "A:AB$ Pump | Cost$ 1 G | Defined$ Self | NumAtt$ +2 | NumDef$ -1 | SpellDescription$ Pump.",
                Operation::ModifyPt,
                1,
            ),
            (
                "A:SP$ PumpAll | ValidCards$ Creature | NumAtt$ +1 | NumDef$ +1 | SpellDescription$ Pump all.",
                Operation::ModifyPt,
                0,
            ),
            (
                "A:SP$ DestroyAll | ValidCards$ Creature | NoRegen$ True | SpellDescription$ Destroy all.",
                Operation::Destroy,
                0,
            ),
            (
                "A:SP$ GainLife | Defined$ You | LifeAmount$ 3 | SpellDescription$ Gain life.",
                Operation::GainLife,
                0,
            ),
            (
                "A:AB$ LoseLife | Cost$ B | Defined$ Opponent | LifeAmount$ 1 | SpellDescription$ Lose life.",
                Operation::LoseLife,
                1,
            ),
            (
                "A:SP$ Mill | Defined$ You | NumCards$ 4 | SpellDescription$ Mill.",
                Operation::Mill,
                0,
            ),
            (
                "A:AB$ Tap | Cost$ W T | ValidTgts$ Creature | SpellDescription$ Tap.",
                Operation::Tap,
                2,
            ),
            (
                "A:AB$ Untap | Cost$ T | ValidTgts$ Land | SpellDescription$ Untap.",
                Operation::Untap,
                1,
            ),
            (
                "A:SP$ Destroy | ValidTgts$ Artifact,Enchantment | SpellDescription$ Destroy.",
                Operation::Destroy,
                0,
            ),
            (
                "A:AB$ PutCounter | Cost$ 2 | Defined$ Self | CounterType$ P1P1 | CounterNum$ 2 | SpellDescription$ Counters.",
                Operation::AddCounter,
                1,
            ),
            (
                "A:SP$ Pump | ValidTgts$ Creature.YouCtrl | KW$ Flying & Vigilance | SpellDescription$ Keywords.",
                Operation::Sequence,
                0,
            ),
            (
                "A:SP$ DealDamage | ValidTgts$ Creature.YouCtrl | NumDmg$ 2 | SpellDescription$ Damage.",
                Operation::DealDamage,
                0,
            ),
        ] {
            assert_operation(line, operation, costs);
        }

        let no_regen = map_line(
            "A:SP$ DestroyAll | ValidCards$ Creature | NoRegen$ True | SpellDescription$ Destroy all.",
        )
        .unwrap_or_else(|error| panic!("NoRegen destroy should map: {}", error.message));
        assert!(matches!(
            no_regen.expression,
            Expression::Call {
                operation: Operation::Destroy,
                ref arguments,
            } if arguments.get(1) == Some(&Expression::Text("cannot_regenerate".to_string()))
        ));
        let error = map_line(
            "A:SP$ DestroyAll | ValidCards$ Creature | NoRegen$ False | SpellDescription$ Destroy all.",
        )
        .err()
        .unwrap_or_else(|| panic!("non-true NoRegen must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn maps_simple_continuous_effects() {
        for line in [
            "S:Mode$ Continuous | Affected$ Card.Self | AddPower$ 2 | AddToughness$ 1 | Description$ Self gets +2/+1.",
            "S:Mode$ Continuous | Affected$ Creature.YouCtrl+Other | AddPower$ 1 | AddToughness$ 1 | Description$ Other creatures get +1/+1.",
            "S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddKeyword$ First Strike | Description$ Equipped creature has first strike.",
            "S:Mode$ Continuous | Affected$ Spirit.YouCtrl | AddPower$ 1 | AddKeyword$ Flying & Vigilance | Description$ Spirits get +1/+0 and keywords.",
            "S:Mode$ ReduceCost | ValidCard$ Instant,Sorcery | Type$ Spell | Activator$ You | Amount$ 1 | Description$ Reduce costs.",
            "S:Mode$ ReduceCost | ValidCard$ Card.Self | Type$ Spell | Amount$ 1 | EffectZone$ All | Description$ Reduce this spell.",
            "S:Mode$ RaiseCost | ValidCard$ Card.nonCreature | Type$ Spell | Amount$ 1 | Description$ Noncreature spells cost more.",
            "S:Mode$ RaiseCost | ValidTarget$ Card.Self | Activator$ Opponent | Type$ Spell | Amount$ 2 | Description$ Opposing spells targeting this cost more.",
            "S:Mode$ CantBlockBy | ValidAttacker$ Creature.Self | Description$ This creature can't be blocked.",
            "S:Mode$ CantBeCast | ValidCard$ Spell | Caster$ Opponent | EffectZone$ All | Description$ Opponents can't cast spells.",
            "S:Mode$ Continuous | Affected$ Card.Self | SetPower$ 4 | SetToughness$ 5 | AddType$ Creature | SetColor$ Blue | Description$ Becomes a creature.",
            "S:Mode$ Continuous | Affected$ Creature.YouCtrl | RemoveAllAbilities$ True | Description$ Remove abilities.",
            "S:Mode$ Continuous | Affected$ Creature.ChosenType+YouCtrl | AddPower$ 1 | Description$ Chosen type gets +1/+0.",
            "S:Mode$ Continuous | Affected$ Creature.EnchantedBy | GainControl$ You | Description$ Gain control.",
            "S:Mode$ Continuous | Affected$ You | SetMaxHandSize$ Unlimited | Description$ No maximum hand size.",
            "S:Mode$ Continuous | Affected$ Card.Self | AddKeyword$ Flying | EffectZone$ All | Description$ Flying.",
            "S:Mode$ Continuous | Affected$ Instant.YouCtrl,Sorcery.YouCtrl | AffectedZone$ Stack | AddKeyword$ Lifelink | Description$ Spells have lifelink.",
            "S:Mode$ Continuous | Affected$ Creature.OppCtrl | RemoveKeyword$ Trample | CantHaveKeyword$ Trample | Description$ Opposing creatures cannot have trample.",
            "S:Mode$ Continuous | Affected$ You | AdjustLandPlays$ 1 | Description$ Play an additional land.",
            "S:Mode$ Continuous | Affected$ Creature | AddHiddenKeyword$ CARDNAME can't block. | Description$ Creatures cannot block.",
            "S:Mode$ Continuous | Affected$ Creature.EnchantedBy | AddHiddenKeyword$ All creatures able to block CARDNAME do so. | Description$ Must be blocked.",
            "S:Mode$ Continuous | CharacteristicDefining$ True | SetPower$ 4 | SetToughness$ 4 | Description$ Characteristic power and toughness.",
        ] {
            assert_operation(line, Operation::Continuous, 0);
        }

        for line in [
            "S:Mode$ Continuous | CharacteristicDefining$ False | SetPower$ 1 | Description$ Invalid CDA.",
            "S:Mode$ Continuous | CharacteristicDefining$ True | Affected$ Card.Self | SetPower$ 1 | Description$ Ambiguous CDA.",
            "S:Mode$ Continuous | CharacteristicDefining$ True | AddKeyword$ Flying | Description$ Invalid CDA.",
        ] {
            let error = map_line(line)
                .err()
                .unwrap_or_else(|| panic!("open characteristic-defining form must quarantine"));
            assert!(matches!(
                error.code.as_str(),
                "UNSUPPORTED_VALUE" | "MISSING_PARAMETER"
            ));
        }

        let play_exiled = map_line(
            "S:Mode$ Continuous | Affected$ Card.IsRemembered | AffectedZone$ Exile | MayPlay$ True | Description$ Play remembered exile.",
        )
        .unwrap_or_else(|error| panic!("closed PlayExiled continuous should map: {}", error.message));
        assert!(expression_contains_operation(
            &play_exiled.expression,
            Operation::PlayExiled
        ));

        let look = map_line(
            "S:Mode$ Continuous | Affected$ Card.TopLibrary+YouCtrl | AffectedZone$ Library | MayLookAt$ You | Description$ Look at the top card.",
        )
        .unwrap_or_else(|error| panic!("closed look permission should map: {}", error.message));
        assert!(expression_contains_operation(
            &look.expression,
            Operation::LookPermission
        ));

        let play_and_look = map_line(
            "S:Mode$ Continuous | Affected$ Card.TopLibrary+YouCtrl | AffectedZone$ Library | MayPlay$ True | MayLookAt$ You | Description$ Play the top card.",
        )
        .unwrap_or_else(|error| panic!("closed library permission should map: {}", error.message));
        assert!(expression_contains_operation(
            &play_and_look.expression,
            Operation::PlayFromZone
        ));
        assert!(expression_contains_operation(
            &play_and_look.expression,
            Operation::LookPermission
        ));

        for (line, code) in [
            (
                "S:Mode$ Continuous | Affected$ Card.IsRemembered | AffectedZone$ Battlefield | MayPlay$ True | Description$ Open play zone.",
                "UNSUPPORTED_VALUE",
            ),
            (
                "S:Mode$ Continuous | Affected$ Card.IsRemembered | AffectedZone$ Exile | MayPlay$ True | MayPlayIgnoreType$ True | Description$ Open play permission.",
                "UNSUPPORTED_PARAMETER",
            ),
        ] {
            let error = map_line(line)
                .err()
                .unwrap_or_else(|| panic!("open PlayExiled form must quarantine"));
            assert_eq!(error.code, code);
        }

        let command = map_script_root(
            "S:Mode$ ReduceCost | ValidCard$ Card.Self | Type$ Spell | Amount$ 1 | EffectZone$ Command | Description$ Reduce this spell.\n",
        )
        .unwrap_or_else(|error| panic!("command-zone static should map: {}", error.message));
        assert!(expression_contains_operation(
            &command.expression,
            Operation::ActiveInZone
        ));

        let error = map_line(
            "S:Mode$ ReduceCost | ValidCard$ Card.Self | Type$ Spell | Amount$ 1 | EffectZone$ Hand | Description$ Reduce this spell.",
        )
        .err()
        .unwrap_or_else(|| panic!("non-closed static EffectZone must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn lowers_dynamic_characteristic_defining_power_and_toughness() {
        let mapped = map_script_root(concat!(
            "Name:Dynamic Characteristic\n",
            "S:Mode$ Continuous | CharacteristicDefining$ True | SetPower$ X | SetToughness$ X | Description$ Dynamic characteristic.\n",
            "SVar:X:Count$Valid Creature.YouCtrl\n",
        ))
        .unwrap_or_else(|error| panic!("dynamic characteristic should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::SetPt
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Count
        ));
    }

    #[test]
    fn maps_closed_first_spell_reduce_cost() {
        let mapped = map_line(
            "S:Mode$ ReduceCost | EffectZone$ Battlefield | ValidCard$ Card.Creature | Activator$ You | Type$ Spell | OnlyFirstSpell$ True | Amount$ 2 | Description$ First creature spell.",
        )
        .unwrap_or_else(|error| panic!("first-spell reducer should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::WhileCondition
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::HistoryCount
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::CostReduction
        ));

        let valid_spell = map_line(
            "S:Mode$ ReduceCost | OnlyFirstSpell$ True | Type$ Spell | ValidSpell$ Spell.Instant,Spell.Sorcery | Activator$ You | Amount$ 3 | Description$ First instant or sorcery.",
        )
        .unwrap_or_else(|error| panic!("closed ValidSpell reducer should map: {}", error.message));
        assert!(expression_contains_operation(
            &valid_spell.expression,
            Operation::Or
        ));
        assert!(expression_contains_operation(
            &valid_spell.expression,
            Operation::HistoryCount
        ));
    }

    #[test]
    fn rejects_open_first_spell_reduce_cost_forms() {
        for (line, code) in [
            (
                "S:Mode$ ReduceCost | ValidCard$ Card | Type$ Spell | ValidSpell$ Spell.IsTargeting Valid Creature | Activator$ You | OnlyFirstSpell$ True | Amount$ 1 | Description$ Targeting reducer.",
                "UNSUPPORTED_VALUE",
            ),
            (
                "S:Mode$ ReduceCost | OnlyFirstSpell$ False | Type$ Spell | ValidCard$ Creature | Activator$ You | Amount$ 1 | Description$ Bad flag.",
                "UNSUPPORTED_VALUE",
            ),
            (
                "S:Mode$ ReduceCost | OnlyFirstSpell$ True | Type$ Spell | ValidSpell$ Spell.Kicked | Activator$ You | Amount$ 1 | Description$ Kicked spell.",
                "UNSUPPORTED_VALUE",
            ),
        ] {
            let error = map_line(line)
                .err()
                .unwrap_or_else(|| panic!("open first-spell reducer must quarantine"));
            assert_eq!(error.code, code);
        }
    }

    #[test]
    fn maps_closed_selector_predicates() {
        for line in [
            "A:AB$ Pump | ValidTgts$ Creature.OppCtrl+Legendary | NumAtt$ 1 | NumDef$ 1 | SpellDescription$ Pump.",
            "A:AB$ Destroy | ValidTgts$ Permanent.YouDontCtrl+nonLand | SpellDescription$ Destroy.",
            "A:AB$ Tap | ValidTgts$ Creature.Blue+withoutFlying | SpellDescription$ Tap.",
            "A:AB$ Pump | ValidTgts$ Creature.Sliver+YouCtrl | NumAtt$ 1 | NumDef$ 1 | SpellDescription$ Pump.",
            "A:SP$ Counter | TargetType$ Spell | ValidTgts$ Card.nonCreature | Destination$ Graveyard | SpellDescription$ Counter.",
            "A:DB$ ChangeZone | Defined$ TriggeredCard | Origin$ Graveyard | Destination$ Battlefield | SpellDescription$ Return.",
            "S:Mode$ AlternativeCost | ValidSA$ Spell.Self | EffectZone$ All | Cost$ 2 W W | Description$ Alternative.",
            "A:AB$ Pump | ValidTgts$ Creature.nonHuman+powerGE4+toughnessLE6 | NumAtt$ 1 | NumDef$ 1 | SpellDescription$ Pump.",
            "A:SP$ LoseLife | ValidTgts$ Opponent | LifeAmount$ 2 | SpellDescription$ Lose life.",
            "A:SP$ Mill | ValidTgts$ Player | NumCards$ 2 | SpellDescription$ Mill.",
            "A:SP$ Discard | ValidTgts$ Player | NumCards$ 1 | Mode$ TgtChoose | SpellDescription$ Discard.",
            "A:SP$ Destroy | ValidTgts$ Creature | TargetMin$ 1 | TargetMax$ 1 | NoRegen$ True | SpellDescription$ Destroy.",
            "A:AB$ Tap | ValidTgts$ Permanent | ActivationZone$ Battlefield | TgtZone$ Battlefield | SpellDescription$ Tap.",
            "A:DB$ ChangeZone | Defined$ TriggeredNewCardLKICopy | Origin$ Graveyard | Destination$ Battlefield | SpellDescription$ Return.",
            "A:DB$ ChangeZone | Defined$ TriggeredCardLKICopy | Origin$ Graveyard | Destination$ Hand | SpellDescription$ Return.",
            "A:DB$ ChangeZone | Defined$ Self | Origin$ Command | Destination$ Exile | SpellDescription$ Exile self from the command zone.",
            "A:AB$ Pump | Defined$ Remembered | NumAtt$ 1 | NumDef$ 1 | SpellDescription$ Remembered pump.",
            "A:AB$ Pump | ValidTgts$ Card.IsRemembered | NumAtt$ 1 | NumDef$ 1 | SpellDescription$ Remembered target pump.",
            "A:AB$ Pump | ValidTgts$ Creature.IsCommander+YouCtrl | NumAtt$ 1 | NumDef$ 1 | SpellDescription$ Commander pump.",
            "A:AB$ Pump | ValidTgts$ Creature.attacking | NumAtt$ 1 | NumDef$ 1 | SpellDescription$ Pump.",
            "A:AB$ Pump | ValidTgts$ Creature.ControlledBy TriggeredDefendingPlayer | NumAtt$ 1 | NumDef$ 1 | SpellDescription$ Pump.",
            "S:Mode$ Continuous | Affected$ Land.OwnedBy TriggeredDefendingPlayer | AddType$ Creature | Description$ Animate defending player's land.",
            "S:Mode$ Continuous | Affected$ Creature.token+YouCtrl | AddPower$ 1 | Description$ Tokens get +1/+0.",
            "S:Mode$ Continuous | Affected$ Creature.!token+YouCtrl | AddPower$ 1 | Description$ Nontokens get +1/+0.",
            "S:Mode$ Continuous | Affected$ Card.IsRemembered+nonLand | AddKeyword$ Flying | Description$ Remembered cards have flying.",
            "S:Mode$ Continuous | Affected$ Card.Self+kicked | AddKeyword$ Flying | Description$ Kicked flying.",
            "S:Mode$ Continuous | Affected$ Card.Self+kicked 1 | AddKeyword$ Flying | Description$ First kicker flying.",
        ] {
            map_line(line).unwrap_or_else(|error| {
                panic!("closed selector should map: {}", error.message);
            });
        }
        let kicked_trigger = map_script_root(concat!(
            "Name:Kicked Trigger\n",
            "T:Mode$ ChangesZone | ValidCard$ Card.Self+kicked | Origin$ Any | Destination$ Battlefield | Execute$ TrigLife | TriggerDescription$ Kicked.\n",
            "SVar:TrigLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("kicked trigger should map: {}", error.message));
        assert!(expression_contains_operation(
            kicked_trigger
                .event
                .as_ref()
                .unwrap_or(&kicked_trigger.expression),
            Operation::DesignationIs
        ));
        let remembered_lki = map_line(
            "A:AB$ Pump | Defined$ RememberedLKI | NumAtt$ 1 | SpellDescription$ LKI pump.",
        )
        .unwrap_or_else(|error| panic!("typed remembered LKI should map: {}", error.message));
        assert!(expression_contains_operation(
            &remembered_lki.expression,
            Operation::RememberedLki
        ));

        let error = map_line(
            "A:DB$ ChangeZone | Defined$ Remembered | Origin$ Command | Destination$ Exile | SpellDescription$ Open command move.",
        )
        .err()
        .unwrap_or_else(|| panic!("remembered command-zone move must remain quarantined"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn preserves_static_target_cardinality_and_rejects_open_ranges() {
        for (minimum, maximum) in [(0, 1), (0, 2), (1, 2), (2, 2)] {
            let mapped = map_line(&format!(
                "A:SP$ Destroy | ValidTgts$ Creature | TargetMin$ {minimum} | TargetMax$ {maximum} | SpellDescription$ Destroy."
            ))
            .unwrap_or_else(|error| panic!("static target range should map: {}", error.message));
            assert!(matches!(
                mapped.expression,
                Expression::Call {
                    operation: Operation::Destroy,
                    ref arguments,
                } if matches!(
                    arguments.first(),
                    Some(Expression::Call {
                        operation: Operation::TargetRange,
                        arguments: range,
                    }) if range.get(1) == Some(&Expression::Integer(minimum))
                        && range.get(2) == Some(&Expression::Integer(maximum))
                )
            ));
        }

        let default = map_line(
            "A:SP$ Destroy | ValidTgts$ Creature | TargetMin$ 1 | TargetMax$ 1 | SpellDescription$ Destroy.",
        )
        .unwrap_or_else(|error| panic!("default target range should map: {}", error.message));
        assert!(expression_contains_operation(
            &default.expression,
            Operation::Target
        ));
        assert!(!expression_contains_operation(
            &default.expression,
            Operation::TargetRange
        ));

        for line in [
            "A:SP$ Destroy | ValidTgts$ Creature | TargetMax$ 2 | SpellDescription$ Destroy.",
            "A:SP$ Destroy | ValidTgts$ Creature | TargetMin$ 0 | SpellDescription$ Destroy.",
            "A:SP$ Destroy | ValidTgts$ Creature | TargetMin$ 0 | TargetMax$ 1 | Optional$ True | SpellDescription$ Destroy.",
        ] {
            let mapped = map_script_root(line).unwrap_or_else(|error| {
                panic!("one-sided or optional range should map: {}", error.message)
            });
            assert_eq!(
                expression_operation_count(&mapped.expression, Operation::TargetRange),
                1
            );
        }

        let repeated = map_line(
            "A:SP$ Pump | ValidTgts$ Creature | TargetMin$ 0 | TargetMax$ 2 | NumAtt$ 1 | KW$ Flying | SpellDescription$ Pump.",
        )
        .unwrap_or_else(|error| panic!("repeated target references should map: {}", error.message));
        assert_eq!(
            expression_operation_count(&repeated.expression, Operation::TargetRange),
            1
        );
        assert_eq!(
            expression_operation_count(&repeated.expression, Operation::Target),
            1
        );

        for line in [
            "A:SP$ Destroy | ValidTgts$ Creature | TargetMin$ X | TargetMax$ X | SpellDescription$ Destroy.",
            "A:SP$ Destroy | ValidTgts$ Creature | TargetMin$ X | TargetMax$ 2 | SpellDescription$ Destroy.",
            "A:SP$ Destroy | ValidTgts$ Creature | TargetMin$ 0 | TargetMax$ X | SpellDescription$ Destroy.",
            "A:SP$ Destroy | ValidTgts$ Creature | TargetMin$ 2 | TargetMax$ 1 | SpellDescription$ Destroy.",
            "A:SP$ Destroy | Defined$ Self | TargetMin$ 0 | TargetMax$ 1 | SpellDescription$ Destroy.",
        ] {
            assert!(map_line(line).is_err(), "open target range must quarantine");
        }
    }

    #[test]
    fn preserves_divided_target_allocations_and_rejects_mismatches() {
        for (script_text, expected_effect, expected_amount) in [
            (
                "A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 4 | TargetMin$ 0 | TargetMax$ 4 | DividedAsYouChoose$ 4 | SpellDescription$ Damage.",
                Operation::DealDamage,
                4,
            ),
            (
                "A:SP$ PutCounter | ValidTgts$ Creature | CounterType$ P1P1 | CounterNum$ 3 | TargetMin$ 1 | TargetMax$ 3 | DividedAsYouChoose$ 3 | SpellDescription$ Counters.",
                Operation::AddCounter,
                3,
            ),
            (
                "A:SP$ PreventDamage | ValidTgts$ Any | Amount$ 2 | TargetMin$ 0 | TargetMax$ 2 | DividedAsYouChoose$ 2 | SpellDescription$ Prevent.",
                Operation::PreventDamage,
                2,
            ),
        ] {
            let mapped = map_script_root(script_text).unwrap_or_else(|error| {
                panic!(
                    "divided allocation should map for `{script_text}`: {}",
                    error.message
                )
            });
            assert!(expression_contains_operation(
                &mapped.expression,
                expected_effect
            ));
            let allocation_index = if expected_effect == Operation::PreventDamage {
                1
            } else {
                0
            };
            assert!(matches!(
                &mapped.expression,
                Expression::Call { arguments, .. }
                    if matches!(
                        arguments.get(allocation_index),
                        Some(Expression::Call {
                            operation: Operation::TargetAllocation,
                            arguments: allocation,
                        }) if allocation.get(1) == Some(&Expression::Integer(expected_amount))
                            && matches!(
                                allocation.first(),
                                Some(Expression::Call {
                                    operation: Operation::TargetRange,
                                    ..
                                })
                            )
                    )
            ));
        }

        for script_text in [
            "A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 4 | TargetMin$ 0 | TargetMax$ 4 | DividedAsYouChoose$ 3",
            "A:SP$ DealDamage | Defined$ You | NumDmg$ 4 | DividedAsYouChoose$ 4",
            "A:SP$ Draw | Defined$ You | NumCards$ 4 | DividedAsYouChoose$ 4",
        ] {
            assert!(
                map_script_root(script_text).is_err(),
                "open divided allocation must quarantine"
            );
        }
    }

    #[test]
    fn resolves_charm_mode_graph() {
        let script = parse_legacy_script(
            "charm.txt",
            concat!(
                "A:SP$ Charm | Choices$ ModeDraw,ModeLife | CharmNum$ 1\n",
                "SVar:ModeDraw:DB$ Draw | Defined$ You\n",
                "SVar:ModeLife:DB$ GainLife | Defined$ You | LifeAmount$ 2\n",
            ),
        )
        .unwrap_or_else(|error| panic!("charm fixture should parse: {error}"));
        let context = MappingContext::from_script(&script);
        let (prefix, expression) = script
            .lines
            .iter()
            .find_map(|line| match &line.kind {
                LegacyLineKind::Ability { prefix, expression } => Some((*prefix, expression)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("charm fixture has no root ability"));
        let mapped = map_legacy_ability_in_context(prefix, expression, &context)
            .unwrap_or_else(|error| panic!("charm graph should map: {}", error.message));
        assert!(matches!(
            mapped.expression,
            Expression::Call {
                operation: Operation::ChooseOne,
                ..
            }
        ));
    }

    #[test]
    fn resolves_moved_replacement_graphs() {
        for (script_text, expected_event, expected_effect) in [
            (
                concat!(
                    "R:Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield | ReplacementResult$ Updated | ReplaceWith$ ETBTapped | Description$ Enters tapped.\n",
                    "SVar:ETBTapped:DB$ Tap | Defined$ Self | ETB$ True\n",
                ),
                Operation::EventEnters,
                Operation::Tap,
            ),
            (
                concat!(
                    "R:Event$ Moved | ValidCard$ Card.Self | Destination$ Graveyard | ReplaceWith$ Exile | Description$ Exile instead.\n",
                    "SVar:Exile:DB$ ChangeZone | Defined$ ReplacedCard | Origin$ All | Destination$ Exile\n",
                ),
                Operation::EventZoneChange,
                Operation::Exile,
            ),
        ] {
            let script = parse_legacy_script("replacement.txt", script_text)
                .unwrap_or_else(|error| panic!("replacement fixture should parse: {error}"));
            let context = MappingContext::from_script(&script);
            let (prefix, expression) = script
                .lines
                .iter()
                .find_map(|line| match &line.kind {
                    LegacyLineKind::Ability { prefix, expression } => Some((*prefix, expression)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("replacement fixture has no root ability"));
            let mapped = map_legacy_ability_in_context(prefix, expression, &context)
                .unwrap_or_else(|error| panic!("replacement should map: {}", error.message));
            assert!(matches!(
                mapped.event,
                Some(Expression::Call { operation, .. }) if operation == expected_event
            ));
            assert!(expression_contains_operation(
                &mapped.expression,
                expected_effect
            ));
        }
    }

    #[test]
    fn maps_simple_zone_token_and_utility_effects() {
        for (line, operation) in [
            (
                "A:SP$ ChangeZone | Origin$ Battlefield | Destination$ Exile | ValidTgts$ Creature | SpellDescription$ Exile.",
                Operation::Exile,
            ),
            (
                "A:SP$ ChangeZone | Origin$ Library | Destination$ Battlefield | ChangeType$ Land.Basic | ChangeNum$ 1 | Tapped$ True | ShuffleNonMandatory$ True | SpellDescription$ Search.",
                Operation::Sequence,
            ),
            (
                "A:SP$ ChangeZone | Origin$ Library | Destination$ Library | ChangeType$ Instant,Sorcery | SpellDescription$ Put it on top.",
                Operation::Sequence,
            ),
            (
                "A:SP$ ChangeZone | Origin$ Graveyard | Destination$ Hand | ValidTgts$ Card.YouCtrl | SpellDescription$ Return.",
                Operation::MoveZoneFrom,
            ),
            (
                "A:SP$ Token | TokenScript$ g_1_1_saproling | TokenOwner$ You | TokenAmount$ 2 | SpellDescription$ Tokens.",
                Operation::CreateToken,
            ),
            (
                "A:SP$ Token | ValidTgts$ Opponent | TokenScript$ g_2_2_beast | TokenOwner$ You | SpellDescription$ Create a token for yourself.",
                Operation::CreateToken,
            ),
            (
                "A:SP$ Token | TokenScript$ c_3_3_wurm_deathtouch,c_3_3_wurm_lifelink | TokenOwner$ You | SpellDescription$ Two tokens.",
                Operation::Sequence,
            ),
            (
                "A:SP$ DestroyAll | ValidCards$ Creature | SpellDescription$ Destroy all.",
                Operation::Destroy,
            ),
            (
                "A:SP$ DamageAll | ValidCards$ Creature | ValidPlayers$ Opponent | NumDmg$ 2 | SpellDescription$ Damage all.",
                Operation::DealDamage,
            ),
            (
                "A:SP$ Discard | Defined$ You | Mode$ TgtChoose | NumCards$ 2 | SpellDescription$ Discard.",
                Operation::DiscardCards,
            ),
            (
                "A:SP$ Counter | TargetType$ Spell | ValidTgts$ Spell | SpellDescription$ Counter.",
                Operation::CounterSpell,
            ),
            (
                "A:SP$ Scry | ScryNum$ 2 | SpellDescription$ Scry.",
                Operation::Scry,
            ),
            (
                "A:SP$ Surveil | Amount$ 1 | SpellDescription$ Surveil.",
                Operation::Surveil,
            ),
            (
                "A:SP$ ChangeZoneAll | ChangeType$ Creature | Origin$ Battlefield | Destination$ Exile | SpellDescription$ Exile all creatures.",
                Operation::Exile,
            ),
            (
                "A:SP$ ChangeZoneAll | ChangeType$ Card | Origin$ Graveyard | Destination$ Exile | SpellDescription$ Exile graveyards.",
                Operation::Exile,
            ),
            (
                "A:AB$ Animate | Defined$ Self | Power$ 3 | Toughness$ 3 | Types$ Creature,Elemental | Colors$ Blue | OverwriteColors$ True | Keywords$ Flying | SpellDescription$ Animate.",
                Operation::UntilEndOfTurn,
            ),
        ] {
            assert_operation(line, operation, 0);
        }

        for line in [
            "A:SP$ ChangeZone | Origin$ Graveyard | Destination$ Hand | Defined$ Self | RememberChanged$ True",
            "A:SP$ ChangeZoneAll | ChangeType$ Creature | Origin$ Battlefield | Destination$ Exile | RememberChanged$ True",
            "A:SP$ ChangeZone | Origin$ Library | Destination$ Hand | ChangeType$ Creature | ChangeNum$ 1 | RememberChanged$ True",
        ] {
            let mapped = map_line(line).unwrap_or_else(|error| {
                panic!("remembered zone move should map: {}", error.message)
            });
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::Remember
            ));
        }
        assert!(map_line(
            "A:SP$ ChangeZone | Origin$ Graveyard | Destination$ Hand | Defined$ Self | RememberChanged$ False"
        )
        .is_err());

        for line in [
            "A:SP$ Destroy | ValidTgts$ Creature | RememberDestroyed$ True",
            "A:SP$ DestroyAll | ValidCards$ Creature | RememberDestroyed$ True",
            "A:SP$ Discard | Defined$ You | Mode$ TgtChoose | NumCards$ 1 | RememberDiscarded$ True",
            "A:SP$ Sacrifice | Defined$ You | SacValid$ Creature | RememberSacrificed$ True",
            "A:SP$ Mill | Defined$ You | NumCards$ 3 | RememberMilled$ True",
        ] {
            let mapped = map_line(line).unwrap_or_else(|error| {
                panic!("remembered result should map: {}", error.message)
            });
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::Remember
            ));
        }
        assert!(
            map_line("A:SP$ Destroy | ValidTgts$ Creature | RememberDestroyed$ False").is_err()
        );
        assert!(
            map_line("A:SP$ DestroyAll | ValidCards$ Creature | RememberDestroyed$ False").is_err()
        );
        assert!(
            map_line("A:SP$ Mill | Defined$ You | NumCards$ 3 | RememberMilled$ False").is_err()
        );

        let tapped = map_line(
            "A:SP$ Token | TokenScript$ c_a_powerstone | TokenTapped$ True | SpellDescription$ Token.",
        )
        .unwrap_or_else(|error| panic!("tapped token should map: {}", error.message));
        assert!(matches!(
            tapped.expression,
            Expression::Call {
                operation: Operation::CreateToken,
                ref arguments,
            } if arguments.get(3) == Some(&Expression::Text("tapped".to_string()))
        ));
        let attacking = map_line(
            "A:SP$ Token | TokenScript$ w_1_1_soldier | TokenTapped$ True | TokenAttacking$ True",
        )
        .unwrap_or_else(|error| panic!("attacking token should map: {}", error.message));
        assert!(expression_contains_operation(
            &attacking.expression,
            Operation::PutCreatedAttacking
        ));
        let error = map_line(
            "A:SP$ Token | TokenScript$ c_a_powerstone | TokenTapped$ False | SpellDescription$ Token.",
        )
        .err()
        .unwrap_or_else(|| panic!("non-true TokenTapped must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");

        let controlled_reanimation = map_line(
            "A:SP$ ChangeZone | Origin$ Graveyard | Destination$ Battlefield | ValidTgts$ Creature | GainControl$ True | SpellDescription$ Reanimate.",
        )
        .unwrap_or_else(|error| panic!("controlled reanimation should map: {}", error.message));
        assert!(matches!(
            controlled_reanimation.expression,
            Expression::Call {
                operation: Operation::Sequence,
                ref arguments,
            } if arguments.iter().any(|argument| matches!(
                argument,
                Expression::Call {
                    operation: Operation::ChangeControl,
                    ..
                }
            ))
        ));

        let controlled_mass_reanimation = map_line(
            "A:SP$ ChangeZoneAll | ChangeType$ Creature | Origin$ Graveyard | Destination$ Battlefield | GainControl$ True | SpellDescription$ Reanimate all.",
        )
        .unwrap_or_else(|error| {
            panic!("controlled mass reanimation should map: {}", error.message)
        });
        assert!(matches!(
            controlled_mass_reanimation.expression,
            Expression::Call {
                operation: Operation::Sequence,
                ref arguments,
            } if arguments.iter().any(|argument| matches!(
                argument,
                Expression::Call {
                    operation: Operation::ChangeControl,
                    ..
                }
            ))
        ));

        let error = map_line(
            "A:SP$ ChangeZone | Origin$ Graveyard | Destination$ Battlefield | ValidTgts$ Creature | GainControl$ ChosenPlayer | SpellDescription$ Reanimate.",
        )
        .err()
        .unwrap_or_else(|| panic!("chosen-player control must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");

        let error = map_line(
            "A:SP$ ChangeZoneAll | ChangeType$ Creature | Origin$ Graveyard | Destination$ Exile | GainControl$ True | SpellDescription$ Exile all.",
        )
        .err()
        .unwrap_or_else(|| panic!("control transfer outside battlefield must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn maps_closed_effect_unblockable_bindings() {
        let targeted = map_line(
            "A:AB$ Effect | Cost$ 1 U | ValidTgts$ Creature.powerLE2 | RememberObjects$ Targeted | ExileOnMoved$ Battlefield | StaticAbilities$ Unblockable | SpellDescription$ Target creature can't be blocked this turn.",
        )
        .unwrap_or_else(|error| panic!("closed Unblockable Effect should map: {}", error.message));
        assert_eq!(targeted.costs.len(), 1);
        assert!(expression_contains_operation(
            &targeted.expression,
            Operation::UntilEndOfTurn
        ));
        assert!(expression_contains_operation(
            &targeted.expression,
            Operation::CannotBeBlockedBy
        ));

        let source = map_line(
            "A:AB$ Effect | Defined$ Self | RememberObjects$ Self | StaticAbilities$ Unblockable | SpellDescription$ This creature can't be blocked this turn.",
        )
        .unwrap_or_else(|error| panic!("source-bound Unblockable Effect should map: {}", error.message));
        assert!(expression_contains_operation(
            &source.expression,
            Operation::Source
        ));

        let equipped = map_line(
            "A:AB$ Effect | RememberObjects$ Equipped | ExileOnMoved$ Battlefield | StaticAbilities$ Unblockable | SpellDescription$ Equipped creature can't be blocked this turn.",
        )
        .unwrap_or_else(|error| panic!("equipped Unblockable Effect should map: {}", error.message));
        assert!(expression_contains_operation(
            &equipped.expression,
            Operation::EquippedObject
        ));
    }

    #[test]
    fn rejects_open_effect_unblockable_shapes() {
        for line in [
            "A:AB$ Effect | ValidTgts$ Creature | StaticAbilities$ KWPump",
            "A:AB$ Effect | ValidTgts$ Creature | StaticAbilities$ Unblockable | Duration$ Permanent",
            "A:AB$ Effect | ValidTgts$ Creature | StaticAbilities$ Unblockable | Duration$ UntilYourNextTurn",
            "A:AB$ Effect | StaticAbilities$ Unblockable",
            "A:AB$ Effect | ValidTgts$ Creature | StaticAbilities$ Unblockable | RememberObjects$ Targeted & TargetedController",
        ] {
            let error = map_line(line)
                .err()
                .unwrap_or_else(|| panic!("open Unblockable Effect must quarantine: {line}"));
            assert!(matches!(
                error.code.as_str(),
                "MISSING_PARAMETER" | "UNSUPPORTED_PARAMETER" | "UNSUPPORTED_VALUE"
            ));
        }
    }

    #[test]
    fn maps_closed_effect_must_attack_bindings() {
        let targeted = map_line(
            "A:SP$ Effect | ValidTgts$ Creature | StaticAbilities$ MustAttack | RememberObjects$ Targeted | ExileOnMoved$ Battlefield | SpellDescription$ Target creature attacks this turn if able.",
        )
        .unwrap_or_else(|error| panic!("targeted MustAttack should map: {}", error.message));
        assert!(matches!(
            targeted.expression,
            Expression::Call {
                operation: Operation::MustAttack,
                ..
            }
        ));

        let remembered = map_line(
            "A:DB$ Effect | RememberObjects$ Remembered | StaticAbilities$ MustAttack | Duration$ UntilEndOfCombat | SpellDescription$ That creature attacks this combat if able.",
        )
        .unwrap_or_else(|error| panic!("remembered MustAttack should map: {}", error.message));
        assert!(expression_contains_operation(
            &remembered.expression,
            Operation::Remembered
        ));
        assert!(matches!(
            remembered.expression,
            Expression::Call {
                operation: Operation::MustAttack,
                ref arguments,
            } if arguments.get(1)
                == Some(&Expression::Text("until_end_of_combat".to_string()))
        ));
    }

    #[test]
    fn rejects_open_effect_must_attack_shapes() {
        for line in [
            "A:SP$ Effect | StaticAbilities$ MustAttack",
            "A:SP$ Effect | ValidTgts$ Creature | StaticAbilities$ MustAttack,MustBlock | RememberObjects$ Targeted",
            "A:SP$ Effect | ValidTgts$ Creature | StaticAbilities$ MustAttack | Duration$ Permanent",
            "A:SP$ Effect | StaticAbilities$ MustAttack | RememberObjects$ ThisTargetedCard | ValidTgts$ Creature",
        ] {
            let error = map_line(line)
                .err()
                .unwrap_or_else(|| panic!("open MustAttack Effect must quarantine: {line}"));
            assert!(matches!(
                error.code.as_str(),
                "MISSING_PARAMETER" | "UNSUPPORTED_PARAMETER" | "UNSUPPORTED_VALUE"
            ));
        }
    }

    #[test]
    fn maps_additional_closed_primitive_apis() {
        for (line, operation) in [
            (
                "A:SP$ Sacrifice | Defined$ Opponent | SacValid$ Creature | SpellDescription$ Sacrifice.",
                Operation::SacrificeEffect,
            ),
            (
                "A:SP$ GainControl | ValidTgts$ Creature | NewController$ You | SpellDescription$ Control.",
                Operation::ChangeControl,
            ),
            (
                "A:SP$ PreventDamage | ValidTgts$ Any | Amount$ 2 | SpellDescription$ Prevent.",
                Operation::PreventDamage,
            ),
            (
                "A:SP$ PutCounterAll | ValidCards$ Creature.YouCtrl | CounterType$ P1P1 | CounterNum$ 1 | SpellDescription$ Counters.",
                Operation::AddCounter,
            ),
            (
                "A:SP$ CopySpellAbility | TargetType$ Spell | ValidTgts$ Instant,Sorcery | MayChooseTarget$ True | SpellDescription$ Copy.",
                Operation::Copy,
            ),
            (
                "A:SP$ CopyPermanent | ValidTgts$ Creature.YouCtrl | TgtPrompt$ Select target creature you control | SpellDescription$ Copy.",
                Operation::Copy,
            ),
            (
                "A:DB$ CopyPermanent | Defined$ Remembered",
                Operation::Copy,
            ),
            (
                "A:DB$ CopyPermanent | Populate$ True",
                Operation::Populate,
            ),
            (
                "A:SP$ AddTurn | Defined$ You | NumTurns$ 1 | SpellDescription$ Turn.",
                Operation::ExtraTurn,
            ),
            (
                "A:SP$ UntapAll | ValidCards$ Creature.YouCtrl | SpellDescription$ Untap.",
                Operation::Untap,
            ),
            (
                "A:SP$ TapOrUntap | ValidTgts$ Permanent | SpellDescription$ Choose.",
                Operation::ChooseOne,
            ),
            (
                "A:SP$ RemoveCounter | Defined$ Self | CounterType$ CHARGE | CounterNum$ 1 | SpellDescription$ Remove.",
                Operation::RemoveCounters,
            ),
            (
                "A:SP$ Proliferate | Amount$ 1 | SpellDescription$ Proliferate.",
                Operation::Proliferate,
            ),
            (
                "S:Mode$ CantAttack | ValidCard$ Card.Self | Description$ Cannot attack.",
                Operation::Continuous,
            ),
            (
                "S:Mode$ CantBlock | ValidCard$ Card.Self | Description$ Cannot block.",
                Operation::Continuous,
            ),
            (
                "S:Mode$ CantBeCast | ValidCard$ Card.nonCreature | Caster$ Opponent | Description$ Cannot cast.",
                Operation::Continuous,
            ),
            (
                "A:SP$ Shuffle | ValidTgts$ Player | SpellDescription$ Shuffle.",
                Operation::Shuffle,
            ),
            (
                "A:SP$ SetLife | ValidTgts$ Player | LifeAmount$ 10 | SpellDescription$ Life.",
                Operation::SetLife,
            ),
            (
                "A:SP$ Venture | Defined$ You | SpellDescription$ Venture.",
                Operation::Venture,
            ),
            (
                "A:SP$ BecomeMonarch | Defined$ You | SpellDescription$ Monarch.",
                Operation::BecomeMonarch,
            ),
            (
                "A:SP$ TakeInitiative | SpellDescription$ Initiative.",
                Operation::TakeInitiative,
            ),
            (
                "A:SP$ Investigate | Num$ 2 | SpellDescription$ Investigate.",
                Operation::CreateToken,
            ),
            (
                "A:SP$ Attach | ValidTgts$ Creature.YouCtrl | SpellDescription$ Attach.",
                Operation::Attach,
            ),
            (
                "A:AB$ Debuff | Defined$ Self | Keywords$ Defender | SpellDescription$ Lose defender.",
                Operation::RemoveKeyword,
            ),
            (
                "A:SP$ TapAll | ValidTgts$ Opponent | ValidCards$ Creature | SpellDescription$ Tap all.",
                Operation::Tap,
            ),
            (
                "S:Mode$ CantAttack,CantBlock | ValidCard$ Creature.EnchantedBy | Description$ Cannot attack or block.",
                Operation::Continuous,
            ),
            (
                "A:SP$ RevealHand | ValidTgts$ Opponent | Look$ True | SpellDescription$ Look.",
                Operation::LookAt,
            ),
            (
                "A:SP$ AnimateAll | ValidCards$ Creature.YouCtrl | Power$ 3 | Toughness$ 3 | Keywords$ Trample | SpellDescription$ Animate all.",
                Operation::UntilEndOfTurn,
            ),
            (
                "A:AB$ SetState | Defined$ Self | Mode$ Transform | SpellDescription$ Transform.",
                Operation::Transform,
            ),
        ] {
            assert_operation(line, operation, 0);
        }

        let temporary = map_line(
            "A:SP$ GainControl | ValidTgts$ Creature | LoseControl$ EOT | Untap$ True | AddKWs$ Haste | SpellDescription$ Borrow.",
        )
        .unwrap_or_else(|error| panic!("temporary control should map: {}", error.message));
        assert!(expression_contains_operation(
            &temporary.expression,
            Operation::UntilEndOfTurn
        ));
        assert!(expression_contains_operation(
            &temporary.expression,
            Operation::Untap
        ));
    }

    #[test]
    fn maps_closed_sacrifice_all_effects() {
        for line in [
            "A:DB$ SacrificeAll | Defined$ EffectSource",
            "A:SP$ SacrificeAll | ValidCards$ Creature.YouCtrl",
            "A:DB$ SacrificeAll | Defined$ DelayTriggerRememberedLKI | Controller$ You | RememberSacrificed$ True",
        ] {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("SacrificeAll should map: {}", error.message));
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::SacrificeEffect
            ));
        }
    }

    #[test]
    fn rejects_open_copy_permanent_shapes() {
        for line in [
            "A:SP$ CopyPermanent | ValidTgts$ Creature | AddTypes$ Nightmare",
            "A:SP$ CopyPermanent | ValidTgts$ Creature | NumCopies$ X",
            "A:SP$ CopyPermanent | ValidTgts$ Creature | NumCopies$ 11",
            "A:SP$ CopyPermanent | Defined$ Self | TokenTapped$ False",
            "A:SP$ CopyPermanent | Defined$ Self | ValidTgts$ Creature",
            "A:SP$ CopyPermanent | Populate$ False",
            "A:SP$ CopyPermanent | Populate$ True | ValidTgts$ Creature",
            "A:SP$ CopyPermanent | Populate$ True | NumCopies$ 2",
            "A:SP$ CopyPermanent",
        ] {
            assert!(
                map_line(line).is_err(),
                "open CopyPermanent form must quarantine: {line}"
            );
        }
    }

    #[test]
    fn maps_literal_copy_counts_and_tapped_results() {
        let counted = map_line(
            "A:SP$ CopyPermanent | ValidTgts$ Creature | NumCopies$ 3 | SpellDescription$ Create three copies.",
        )
        .unwrap_or_else(|error| panic!("literal copy count should map: {}", error.message));
        assert!(expression_contains_operation(
            &counted.expression,
            Operation::Sequence
        ));

        let tapped = map_line(
            "A:DB$ CopyPermanent | Defined$ Self | TokenTapped$ True | SpellDescription$ Create a tapped copy.",
        )
        .unwrap_or_else(|error| panic!("tapped copy should map: {}", error.message));
        assert!(expression_contains_operation(
            &tapped.expression,
            Operation::Tap
        ));
    }

    #[test]
    fn maps_closed_card_choices_and_rejects_open_shapes() {
        let remembered = map_line(
            "A:DB$ ChooseCard | Defined$ You | Amount$ 1 | Choices$ Card.IsRemembered | ChoiceZone$ Exile | Mandatory$ True | RememberChosen$ True | ChoiceTitle$ Choose a card.",
        )
        .unwrap_or_else(|error| panic!("closed remembered choice should map: {}", error.message));
        assert!(expression_contains_operation(
            &remembered.expression,
            Operation::ChooseObjects
        ));
        assert!(expression_contains_operation(
            &remembered.expression,
            Operation::Remember
        ));

        let random = map_line(
            "A:DB$ ChooseCard | Choices$ Artifact,Enchantment | AtRandom$ True | SpellDescription$ Choose at random.",
        )
        .unwrap_or_else(|error| panic!("closed random choice should map: {}", error.message));
        assert!(expression_contains_operation(
            &random.expression,
            Operation::ChooseObjects
        ));

        for line in [
            "A:DB$ ChooseCard | Choices$ Card | ChoiceZone$ Sideboard",
            "A:DB$ ChooseCard | Choices$ Card | Amount$ X",
            "A:DB$ ChooseCard | Choices$ Card | Amount$ 2 | Mandatory$ True | MinAmount$ 0",
            "A:DB$ ChooseCard | Choices$ Card | AtRandom$ False",
            "A:DB$ ChooseCard | Choices$ Card | DefinedCards$ Remembered",
        ] {
            assert!(
                map_line(line).is_err(),
                "open choice must quarantine: {line}"
            );
        }
    }

    #[test]
    fn maps_closed_player_choices() {
        for line in [
            "A:DB$ ChoosePlayer | Defined$ You | Choices$ Player.Opponent | ChoiceTitle$ Choose an opponent",
            "A:DB$ ChoosePlayer | Choices$ Player | Random$ True",
            "A:SP$ ChoosePlayer | ValidTgts$ Opponent | Choices$ You | Secretly$ True | RememberChosen$ True",
        ] {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("ChoosePlayer should map: {}", error.message));
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::ChoosePlayer
            ));
        }
    }

    #[test]
    fn maps_closed_generic_effect_choices() {
        let mapped = map_script_root(concat!(
            "Name:Generic Choice\n",
            "A:SP$ GenericChoice | Defined$ You | Choices$ DBLife,DBDraw | ShowChoice$ Description | SetChosenMode$ True | SpellDescription$ Choose.\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 2\n",
            "SVar:DBDraw:DB$ Draw | Defined$ You | NumCards$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("GenericChoice should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::PlayerChooseEffect
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::GainLife
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Draw
        ));
    }

    #[test]
    fn maps_closed_branch_effects() {
        let mapped = map_script_root("A:DB$ Branch | BranchConditionSVar$ X | BranchConditionSVarCompare$ GE2 | TrueSubAbility$ Yes | FalseSubAbility$ No\nSVar:X:Count$Valid Creature.YouCtrl\nSVar:Yes:DB$ Draw | Defined$ You | NumCards$ 1\nSVar:No:DB$ GainLife | Defined$ You | LifeAmount$ 1\n")
            .unwrap_or_else(|error| panic!("Branch should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::BranchEffect
        ));
    }

    #[test]
    fn maps_closed_play_permissions_and_rejects_open_shapes() {
        let free = map_line(
            "A:DB$ Play | Valid$ Card.nonLand+YouOwn | ValidZone$ Hand | ValidSA$ Spell.cmcLE5 | WithoutManaCost$ True | Amount$ 1 | Controller$ You | RememberPlayed$ True",
        )
        .unwrap_or_else(|error| panic!("closed free play should map: {}", error.message));
        assert!(expression_contains_operation(
            &free.expression,
            Operation::Play
        ));
        assert!(expression_contains_operation(
            &free.expression,
            Operation::Remember
        ));

        let defined = map_line(
            "A:DB$ Play | Defined$ Remembered | ValidSA$ Spell | Amount$ All | Controller$ You",
        )
        .unwrap_or_else(|error| panic!("defined play should map: {}", error.message));
        assert!(expression_contains_operation(
            &defined.expression,
            Operation::Remembered
        ));

        for line in [
            "A:DB$ Play | Valid$ Card | ValidZone$ Battlefield",
            "A:DB$ Play | Valid$ Card | Amount$ X",
            "A:DB$ Play | Valid$ Card | ValidSA$ Spell.cmcLEX",
            "A:DB$ Play | Defined$ Remembered | Valid$ Card",
            "A:DB$ Play | Defined$ Remembered | CopyCard$ True",
        ] {
            assert!(map_line(line).is_err(), "open Play must quarantine: {line}");
        }
    }

    #[test]
    fn maps_closed_player_repeat_chains() {
        let repeated = map_script_root(concat!(
            "Name:Repeat Players\n",
            "A:SP$ RepeatEach | RepeatPlayers$ Player | RepeatSubAbility$ DBGain | ChangeZoneTable$ True | SubAbility$ DBDraw | SpellDescription$ Each player gains life, then draw.\n",
            "SVar:DBGain:DB$ GainLife | Defined$ Player | LifeAmount$ 1\n",
            "SVar:DBDraw:DB$ Draw | Defined$ You | NumCards$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("closed player repeat should map: {}", error.message));
        assert!(expression_contains_operation(
            &repeated.expression,
            Operation::ForEach
        ));
        assert!(expression_contains_operation(
            &repeated.expression,
            Operation::Draw
        ));
        assert!(expression_contains_operation(
            &repeated.expression,
            Operation::BatchEvents
        ));

        let cards = map_script_root(concat!(
            "Name:Repeat Cards\n",
            "A:SP$ RepeatEach | RepeatCards$ Creature | Zone$ Battlefield | ChooseOrder$ True | UseImprinted$ True | DamageMap$ True | RepeatSubAbility$ DBDamage\n",
            "SVar:DBDamage:DB$ DealDamage | Defined$ Imprinted | NumDmg$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("closed card repeat should map: {}", error.message));
        assert!(expression_contains_operation(
            &cards.expression,
            Operation::ForEachImprinted
        ));
        assert!(expression_contains_operation(
            &cards.expression,
            Operation::OrderByPlayer
        ));
        assert!(expression_contains_operation(
            &cards.expression,
            Operation::BatchEvents
        ));

        for repeat_players in ["ActivePlayer", "TargetedController"] {
            let line = format!(
                "A:SP$ RepeatEach | RepeatPlayers$ {repeat_players} | RepeatSubAbility$ Missing"
            );
            assert!(map_line(&line).is_err());
        }
    }

    #[test]
    fn maps_closed_cannot_be_countered_replacements() {
        for line in [
            "R:Event$ Counter | ValidCard$ Card.Self | ValidSA$ Spell | Layer$ CantHappen | Description$ This spell can't be countered.",
            "R:Event$ Counter | ValidSA$ Spell.Creature+YouCtrl | Layer$ CantHappen | ActiveZones$ Battlefield | Description$ Creature spells you control can't be countered.",
        ] {
            let mapped = map_script_root(line).unwrap_or_else(|error| {
                panic!("closed counter replacement should map: {}", error.message)
            });
            assert!(matches!(
                mapped.event,
                Some(Expression::Call {
                    operation: Operation::EventCounterAttempt,
                    ..
                })
            ));
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::CannotBeCountered
            ));
        }

        for line in [
            "R:Event$ Counter | ValidSA$ Spell | Layer$ Replace | ActiveZones$ Battlefield",
            "R:Event$ Counter | ValidCard$ Card.Self | ValidSA$ Spell | Layer$ CantHappen | ActiveZones$ Battlefield",
            "R:Event$ Counter | ValidSA$ Spell | ValidCause$ SpellAbility.YouCtrl | ReplaceWith$ DBRemove",
        ] {
            assert!(
                map_script_root(line).is_err(),
                "open replacement must quarantine: {line}"
            );
        }
    }

    #[test]
    fn maps_supported_structured_costs() {
        for (line, operation, costs) in [
            (
                "A:AB$ GainLife | Cost$ PayLife<2> | LifeAmount$ 3 | SpellDescription$ Life.",
                Operation::GainLife,
                1,
            ),
            (
                "A:AB$ Token | Cost$ AddCounter<1/LOYALTY> | TokenScript$ w_1_1_soldier | SpellDescription$ Token.",
                Operation::CreateToken,
                1,
            ),
            (
                "A:AB$ Draw | Cost$ AddCounter<0/LOYALTY> | NumCards$ 1 | SpellDescription$ Draw.",
                Operation::Draw,
                1,
            ),
            (
                "A:AB$ Destroy | Cost$ T Sac<1/CARDNAME> | ValidTgts$ Creature | SpellDescription$ Destroy.",
                Operation::Destroy,
                2,
            ),
            (
                "A:AB$ Scry | Cost$ SubCounter<1/OIL> | ScryNum$ 1 | SpellDescription$ Scry.",
                Operation::Scry,
                1,
            ),
            (
                "A:AB$ Draw | Cost$ SubCounter<3/CHARGE> | NumCards$ 1 | SpellDescription$ Draw.",
                Operation::Draw,
                3,
            ),
            (
                "A:AB$ Mill | Cost$ B Discard<1/Card> | NumCards$ 2 | SpellDescription$ Mill.",
                Operation::Mill,
                2,
            ),
            (
                "A:AB$ Draw | Cost$ 2 T Sac<1/Land> | NumCards$ 1 | SpellDescription$ Draw.",
                Operation::Draw,
                3,
            ),
            (
                "A:AB$ Draw | Cost$ B Discard<1/Land> | NumCards$ 1 | SpellDescription$ Draw.",
                Operation::Draw,
                2,
            ),
            (
                "A:AB$ GainLife | Cost$ ExileFromGrave<1/Creature> | LifeAmount$ 2 | SpellDescription$ Life.",
                Operation::GainLife,
                1,
            ),
            (
                "A:AB$ ChangeZone | Cost$ Exile<1/CARDNAME> | Origin$ Battlefield | Destination$ Exile | Defined$ Self | SpellDescription$ Exile.",
                Operation::Exile,
                1,
            ),
        ] {
            assert_operation(line, operation, costs);
        }
    }

    #[test]
    fn lifts_shared_activation_timing() {
        for (line, expected) in [
            (
                "A:AB$ SetState | Defined$ Self | Mode$ Transform | SorcerySpeed$ True | SpellDescription$ Transform.",
                Operation::TimingSorcery,
            ),
            (
                "A:AB$ Draw | Defined$ You | PlayerTurn$ True | SpellDescription$ Draw.",
                Operation::TimingYourTurn,
            ),
            (
                "A:AB$ Draw | Defined$ You | ActivationLimit$ 1 | SpellDescription$ Draw.",
                Operation::TimingOnceEachTurn,
            ),
        ] {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("timing fixture should map: {}", error.message));
            assert!(matches!(
                mapped.timing,
                Some(Expression::Call { operation, .. }) if operation == expected
            ));
        }
    }

    #[test]
    fn lifts_closed_legacy_activation_conditions_and_phase_windows() {
        for activation in [
            "Threshold",
            "Delirium",
            "Metalcraft",
            "Hellbent",
            "Solved",
            "Blessing",
        ] {
            let mapped = map_line(&format!(
                "A:AB$ Draw | Defined$ You | Activation$ {activation} | SpellDescription$ Draw."
            ))
            .unwrap_or_else(|error| panic!("closed activation should map: {}", error.message));
            assert!(matches!(
                mapped.timing.as_ref(),
                Some(timing)
                    if expression_contains_operation(timing, Operation::TimingCondition)
            ));
        }

        let mapped = map_line(
            "A:AB$ Pump | Defined$ Self | NumAtt$ +1 | ActivationPhases$ Upkeep->BeginCombat | ActivationFirstCombat$ True",
        )
        .unwrap_or_else(|error| panic!("phase window should map: {}", error.message));
        let Some(timing) = mapped.timing.as_ref() else {
            panic!("phase timing must be present");
        };
        assert!(expression_contains_operation(timing, Operation::TimingAll));
        assert!(expression_contains_operation(timing, Operation::During));

        for line in [
            "A:AB$ Draw | Defined$ You | Activation$ Unknown",
            "A:AB$ Draw | Defined$ You | ActivationPhases$ Upkeep->Unknown",
            "A:AB$ Draw | Defined$ You | ActivationFirstCombat$ False",
        ] {
            assert!(
                map_line(line).is_err(),
                "open timing must quarantine: {line}"
            );
        }
    }

    #[test]
    fn maps_closed_created_object_lifetimes_and_memory() {
        let token = map_line(
            "A:AB$ Token | TokenScript$ r_3_1_elemental | RememberTokens$ True | AtEOT$ Exile",
        )
        .unwrap_or_else(|error| panic!("token metadata should map: {}", error.message));
        for operation in [
            Operation::CreateToken,
            Operation::Remember,
            Operation::EffectResult,
            Operation::RegisterDelayedTrigger,
            Operation::Exile,
        ] {
            assert!(expression_contains_operation(&token.expression, operation));
        }

        let copy = map_line(
            "A:DB$ CopyPermanent | Defined$ Self | RememberTokens$ True | AtEOT$ Sacrifice",
        )
        .unwrap_or_else(|error| panic!("copy metadata should map: {}", error.message));
        assert!(expression_contains_operation(
            &copy.expression,
            Operation::Copy
        ));
        assert!(expression_contains_operation(
            &copy.expression,
            Operation::SacrificeEffect
        ));
    }

    #[test]
    fn maps_closed_type_removal_and_linked_exile_lifetimes() {
        let continuous = map_line(
            "S:Mode$ Continuous | Affected$ Creature.EnchantedBy | AddType$ Creature & Frog | RemoveCardTypes$ True | RemoveLandTypes$ True | Description$ Frog.",
        )
        .unwrap_or_else(|error| panic!("type removal should map: {}", error.message));
        assert!(expression_contains_operation(
            &continuous.expression,
            Operation::RemoveType
        ));

        let exile = map_line(
            "A:DB$ ChangeZone | Origin$ Battlefield | Destination$ Exile | ValidTgts$ Permanent.nonLand | Duration$ UntilHostLeavesPlay",
        )
        .unwrap_or_else(|error| panic!("linked exile should map: {}", error.message));
        for operation in [
            Operation::Exile,
            Operation::EventLeaves,
            Operation::EffectResult,
            Operation::MoveZoneFrom,
        ] {
            assert!(expression_contains_operation(&exile.expression, operation));
        }
    }

    #[test]
    fn maps_targeted_reducers_and_combat_state_unions() {
        let reducer = map_line(
            "S:Mode$ ReduceCost | ValidCard$ Card.Self | ValidTarget$ Creature.tapped | Type$ Spell | Amount$ 1 | EffectZone$ All | Description$ Reduce.",
        )
        .unwrap_or_else(|error| panic!("target reducer should map: {}", error.message));
        assert!(expression_contains_operation(
            &reducer.expression,
            Operation::Targets
        ));

        let damage = map_line(
            "A:SP$ DealDamage | ValidTgts$ Creature.attacking,Creature.blocking | NumDmg$ 1",
        )
        .unwrap_or_else(|error| panic!("combat union should map: {}", error.message));
        assert!(expression_contains_operation(
            &damage.expression,
            Operation::DesignationIs
        ));
    }

    #[test]
    fn maps_closed_threshold_conditions_without_resolution_rechecks() {
        let static_ability = map_line(
            "S:Mode$ Continuous | Affected$ Creature.EnchantedBy | AddKeyword$ Shroud | Condition$ Threshold | Description$ Threshold.",
        )
        .unwrap_or_else(|error| panic!("threshold condition should map: {}", error.message));
        assert!(expression_contains_operation(
            &static_ability.expression,
            Operation::WhileCondition
        ));

        let trigger = map_script_root(
            "T:Mode$ Attacks | ValidCard$ Card.Self | IsPresent$ Creature.attacking+Other | PresentCompare$ GE2 | NoResolvingCheck$ True | Execute$ Pump\nSVar:Pump:DB$ Pump | Defined$ Self | NumAtt$ +2 | NumDef$ +2\n",
        )
        .unwrap_or_else(|error| panic!("trigger-only check should map: {}", error.message));
        assert!(expression_contains_operation(
            trigger
                .event
                .as_ref()
                .unwrap_or_else(|| panic!("trigger event")),
            Operation::EventWhen
        ));
        assert!(!expression_contains_operation(
            &trigger.expression,
            Operation::WhileCondition
        ));
    }

    #[test]
    fn maps_closed_entry_counters_and_life_gained_values() {
        let moved = map_line(
            "A:SP$ ChangeZone | Origin$ Graveyard | Destination$ Battlefield | ValidTgts$ Creature | WithCountersType$ M1M1 | WithCountersAmount$ 2",
        )
        .unwrap_or_else(|error| panic!("entry counters should map: {}", error.message));
        assert!(expression_contains_operation(
            &moved.expression,
            Operation::AddCounter
        ));
        assert!(expression_contains_operation(
            &moved.expression,
            Operation::EffectResult
        ));

        let mana = map_script_root(
            "A:AB$ Mana | Cost$ T | Produced$ Any | Amount$ X\nSVar:X:Count$LifeYouGainedThisTurn\n",
        )
        .unwrap_or_else(|error| panic!("life-gained value should map: {}", error.message));
        assert!(expression_contains_operation(
            &mana.expression,
            Operation::HistoryCount
        ));
    }

    #[test]
    fn maps_plural_keyword_counters() {
        let mapped = map_line(
            "A:DB$ PutCounter | Defined$ Targeted | CounterTypes$ Flying,First Strike,Lifelink",
        )
        .unwrap_or_else(|error| panic!("plural counters should map: {}", error.message));
        assert!(matches!(
            mapped.expression,
            Expression::Call {
                operation: Operation::Sequence,
                ref arguments,
            } if arguments.len() == 3
        ));

        for (line, operation) in [
            ("A:DB$ PutCounter | Bolster$ 2", Operation::Bolster),
            (
                "A:DB$ PutCounter | Support$ 2 | ValidTgts$ Creature.Other+YouCtrl | TargetMin$ 0 | TargetMax$ 2",
                Operation::Support,
            ),
            ("A:AB$ PutCounter | Cost$ 2 G | Adapt$ 1", Operation::Adapt),
            (
                "A:AB$ PutCounter | Cost$ 3 G G | Monstrosity$ 4",
                Operation::Monstrosity,
            ),
            (
                "A:DB$ PutCounter | Defined$ Self | Placer$ TriggeredSource | TriggeredCounterMap$ True | CounterMapValues$ 1",
                Operation::CopyTriggeredCounters,
            ),
        ] {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("named counter mechanic should map: {}", error.message));
            assert!(expression_contains_operation(&mapped.expression, operation));
        }
    }

    #[test]
    fn maps_closed_untap_prevention_replacements() {
        let mapped = map_script_root(
            "R:Event$ Untap | ActiveZones$ Battlefield | ValidCard$ Creature.EnchantedBy | ValidStepTurnToController$ You | Layer$ CantHappen | Description$ Does not untap.\n",
        )
        .unwrap_or_else(|error| panic!("untap prevention should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Continuous
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::CannotUntap
        ));

        for line in [
            "R:Event$ Untap | ValidCard$ Card.Self | Layer$ Replace | Description$ Bad.",
            "R:Event$ Untap | ValidCard$ Card.Self | Layer$ CantHappen | ValidStepTurnToController$ Opponent | Description$ Bad.",
        ] {
            assert!(map_script_root(line).is_err());
        }
    }

    #[test]
    fn maps_dynamic_target_range_bounds() {
        let mapped = map_script_root(
            "A:SP$ Destroy | ValidTgts$ Creature | TargetMin$ X | TargetMax$ X\nSVar:X:Count$CardPower\n",
        )
        .unwrap_or_else(|error| panic!("dynamic target range should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::TargetRange
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Power
        ));
    }

    #[test]
    fn maps_dynamic_mana_value_selectors() {
        let svar = map_script_root(concat!(
            "A:SP$ ChangeZone | Origin$ Graveyard | Destination$ Battlefield | ValidTgts$ Creature.YouOwn+cmcLEX\n",
            "SVar:X:Count$CardPower\n",
        ))
        .unwrap_or_else(|error| panic!("SVar mana-value limit should map: {}", error.message));
        assert!(expression_contains_operation(
            &svar.expression,
            Operation::ManaValue
        ));
        assert!(expression_contains_operation(
            &svar.expression,
            Operation::Power
        ));

        let paid = map_script_root(
            "A:SP$ ChangeZone | Cost$ X B | Origin$ Library | Destination$ Hand | ChangeType$ Creature.cmcLEX | ChangeNum$ 1\n",
        )
        .unwrap_or_else(|error| panic!("paid-X mana-value limit should map: {}", error.message));
        assert!(expression_contains_operation(
            &paid.expression,
            Operation::PaidX
        ));
    }

    #[test]
    fn lowers_closed_svar_arithmetic() {
        let mapped = map_script_root(concat!(
            "A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ Z\n",
            "SVar:X:Count$CardPower\n",
            "SVar:Y:SVar$X/Times.3\n",
            "SVar:Z:SVar$Y/Plus.2\n",
        ))
        .unwrap_or_else(|error| panic!("closed SVar arithmetic should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::ScaleValue
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::AddValue
        ));

        let half = map_script_root(concat!(
            "A:SP$ GainLife | Defined$ You | LifeAmount$ Y\n",
            "SVar:X:Count$CardPower\n",
            "SVar:Y:SVar$X/HalfDown\n",
        ))
        .unwrap_or_else(|error| panic!("rounded SVar division should map: {}", error.message));
        assert!(expression_contains_operation(
            &half.expression,
            Operation::DivideValue
        ));

        let cycle = map_script_root(concat!(
            "A:SP$ GainLife | Defined$ You | LifeAmount$ X\n",
            "SVar:X:SVar$Y\n",
            "SVar:Y:SVar$X\n",
        ))
        .err()
        .unwrap_or_else(|| panic!("cyclic SVar arithmetic must quarantine"));
        assert_eq!(cycle.code, "CYCLIC_SVAR");
    }

    #[test]
    fn maps_closed_defined_event_selectors_without_conflating_roles() {
        let cases = [
            (
                "A:DB$ Pump | Defined$ ParentTarget | NumAtt$ 1 | NumDef$ 1",
                Operation::ParentTarget,
            ),
            (
                "A:DB$ LoseLife | Defined$ TriggeredPlayer | LifeAmount$ 1",
                Operation::TriggeredPlayer,
            ),
            (
                "A:DB$ DealDamage | Defined$ TriggeredTarget | NumDmg$ 1",
                Operation::TriggeredTarget,
            ),
            (
                "A:DB$ LoseLife | Defined$ TriggeredActivator | LifeAmount$ 1",
                Operation::TriggeredActivator,
            ),
            (
                "A:DB$ LoseLife | Defined$ TriggeredDefendingPlayer | LifeAmount$ 1",
                Operation::TriggeredDefendingPlayer,
            ),
            (
                "A:DB$ LoseLife | Defined$ TargetedController | LifeAmount$ 1",
                Operation::ControllerOf,
            ),
        ];
        for (line, expected) in cases {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("defined selector should map: {}", error.message));
            assert!(expression_contains_operation(&mapped.expression, expected));
        }
    }

    #[test]
    fn maps_closed_modal_owner_and_stack_ability_copy_selectors() {
        let charm = map_script_root(
            "A:SP$ Charm | Choices$ DBLife,DBDraw | Defined$ You\nSVar:DBLife:DB$ GainLife | LifeAmount$ 2\nSVar:DBDraw:DB$ Draw | NumCards$ 1\n",
        )
        .unwrap_or_else(|error| panic!("owner-selected charm should map: {}", error.message));
        assert!(expression_contains_operation(
            &charm.expression,
            Operation::ChooseOne
        ));
        let ranged_charm = map_script_root(
            "A:SP$ Charm | Choices$ DBLife,DBDraw,DBMill | Defined$ You | MinCharmNum$ 1 | CharmNum$ 2\nSVar:DBLife:DB$ GainLife | LifeAmount$ 2\nSVar:DBDraw:DB$ Draw | NumCards$ 1\nSVar:DBMill:DB$ Mill | NumCards$ 1\n",
        )
        .unwrap_or_else(|error| panic!("ranged charm should map: {}", error.message));
        assert!(expression_contains_operation(
            &ranged_charm.expression,
            Operation::ChooseBetween
        ));

        let optional_charm = map_script_root(
            "A:SP$ Charm | Choices$ DBLife,DBDraw | MinCharmNum$ 0 | CharmNum$ 1\nSVar:DBLife:DB$ GainLife | LifeAmount$ 2\nSVar:DBDraw:DB$ Draw | NumCards$ 1\n",
        )
        .unwrap_or_else(|error| panic!("optional charm should map: {}", error.message));
        assert!(expression_contains_operation(
            &optional_charm.expression,
            Operation::ChooseUpTo
        ));

        let triggered = map_line(
            "A:DB$ CopySpellAbility | Defined$ TriggeredSpellAbility | MayChooseTarget$ True",
        )
        .unwrap_or_else(|error| panic!("triggered copy should map: {}", error.message));
        assert!(expression_contains_operation(
            &triggered.expression,
            Operation::TriggeredStackAbility
        ));

        let parent = map_line("A:DB$ CopySpellAbility | Defined$ Parent")
            .unwrap_or_else(|error| panic!("parent copy should map: {}", error.message));
        assert!(expression_contains_operation(
            &parent.expression,
            Operation::ParentStackAbility
        ));

        for line in [
            "A:SP$ Charm | Choices$ A,B | Defined$ Opponent",
            "A:DB$ CopySpellAbility | Defined$ Self",
            "A:DB$ CopySpellAbility | Defined$ TriggeredSpellAbility | ValidTgts$ Instant",
        ] {
            assert!(map_line(line).is_err());
        }
    }

    #[test]
    fn maps_closed_immediate_and_phase_delayed_effects() {
        let immediate = map_script_root(
            "A:AB$ ImmediateTrigger | Cost$ 1 | Execute$ DBLife\nSVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 2\n",
        )
        .unwrap_or_else(|error| panic!("immediate trigger should map: {}", error.message));
        assert_eq!(immediate.costs.len(), 1);
        assert!(expression_contains_operation(
            &immediate.expression,
            Operation::GainLife
        ));

        let remembered_immediate = map_script_root(concat!(
            "A:DB$ ImmediateTrigger | Execute$ DBLife | RememberObjects$ TriggeredCard | SubAbility$ DBDraw\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 2\n",
            "SVar:DBDraw:DB$ Draw | Defined$ You | NumCards$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("remembering immediate trigger should map: {}", error.message));
        assert!(expression_contains_operation(
            &remembered_immediate.expression,
            Operation::EventImmediate
        ));
        assert!(expression_contains_operation(
            &remembered_immediate.expression,
            Operation::RegisterDelayedTriggerRemembering
        ));
        assert!(expression_contains_operation(
            &remembered_immediate.expression,
            Operation::Draw
        ));

        for phase in [
            "Upkeep",
            "End of Turn",
            "End Of Turn",
            "EndCombat",
            "Main1",
            "Main2",
            "Draw",
            "Cleanup",
        ] {
            let delayed = map_script_root(&format!(
                "A:DB$ DelayedTrigger | Mode$ Phase | Phase$ {phase} | ValidPlayer$ You | NextTurn$ True | Execute$ DBDraw\nSVar:DBDraw:DB$ Draw | Defined$ You\n"
            ))
            .unwrap_or_else(|error| panic!("delayed trigger should map: {}", error.message));
            assert!(expression_contains_operation(
                &delayed.expression,
                Operation::RegisterDelayedTrigger
            ));
        }

        let remembered_delayed = map_script_root(concat!(
            "A:DB$ DelayedTrigger | Mode$ Phase | Phase$ EndCombat | Execute$ DBDestroy | RememberObjects$ TriggeredBlockerLKICopy | SubAbility$ DBCleanup\n",
            "SVar:DBDestroy:DB$ Destroy | Defined$ DelayTriggerRememberedLKI\n",
            "SVar:DBCleanup:DB$ Cleanup | ClearRemembered$ True\n",
        ))
        .unwrap_or_else(|error| panic!("remembering delayed trigger should map: {}", error.message));
        assert!(expression_contains_operation(
            &remembered_delayed.expression,
            Operation::RegisterDelayedTriggerRemembering
        ));
        assert!(expression_contains_operation(
            &remembered_delayed.expression,
            Operation::Forget
        ));

        for script in [
            "A:DB$ DelayedTrigger | Mode$ SpellCast | Phase$ Upkeep | Execute$ DBDraw\nSVar:DBDraw:DB$ Draw | Defined$ You\n",
            "A:DB$ DelayedTrigger | Mode$ Phase | Phase$ Upkeep | NextTurn$ True | ThisTurn$ True | Execute$ DBDraw\nSVar:DBDraw:DB$ Draw | Defined$ You\n",
        ] {
            assert!(map_script_root(script).is_err());
        }
    }

    #[test]
    fn maps_closed_fog_and_defined_fight_effects() {
        let fog = map_line("A:SP$ Fog | SpellDescription$ Prevent all combat damage.")
            .unwrap_or_else(|error| panic!("fog should map: {}", error.message));
        assert!(expression_contains_operation(
            &fog.expression,
            Operation::PreventAllCombatDamage
        ));

        let fight =
            map_line("A:DB$ Fight | Defined$ ParentTarget | ValidTgts$ Creature.YouDontCtrl")
                .unwrap_or_else(|error| panic!("defined fight should map: {}", error.message));
        assert!(expression_contains_operation(
            &fight.expression,
            Operation::Fight
        ));
        assert!(map_line("A:SP$ Fight | ValidTgts$ Creature").is_err());
    }

    #[test]
    fn maps_closed_must_attack_continuous_effects() {
        let mapped =
            map_line("S:Mode$ MustAttack | ValidCreature$ Creature.YouCtrl | Description$ Attack.")
                .unwrap_or_else(|error| panic!("must-attack effect should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Continuous
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::MustAttack
        ));
        assert!(map_line(
            "S:Mode$ MustAttack | ValidCreature$ Creature | MustAttack$ EnchantedController"
        )
        .is_err());
    }

    #[test]
    fn maps_closed_explore_and_connive_effects() {
        for (line, operation) in [
            ("A:DB$ Explore", Operation::Explore),
            (
                "A:AB$ Connive | Cost$ 3 | ValidTgts$ Creature.YouCtrl",
                Operation::Connive,
            ),
        ] {
            let mapped = map_line(line).unwrap_or_else(|error| {
                panic!("explore-like effect should map: {}", error.message)
            });
            assert!(expression_contains_operation(&mapped.expression, operation));
        }

        let dynamic = map_script_root(
            "A:DB$ Connive | Defined$ Self | ConniveNum$ X\nSVar:X:Count$CardPower\n",
        )
        .unwrap_or_else(|error| panic!("dynamic connive should map: {}", error.message));
        assert!(expression_contains_operation(
            &dynamic.expression,
            Operation::Power
        ));
    }

    #[test]
    fn maps_closed_blocker_ranges_and_flash_permissions() {
        for (line, operation) in [
            (
                "S:Mode$ MinMaxBlocker | ValidCard$ Card.Self | Max$ 1",
                Operation::MaximumBlockers,
            ),
            (
                "S:Mode$ MinMaxBlocker | ValidCard$ Creature.Self | Min$ 3",
                Operation::MinimumBlockers,
            ),
            (
                "S:Mode$ CastWithFlash | ValidCard$ Sorcery | ValidSA$ Spell | Caster$ You",
                Operation::CastWithFlash,
            ),
        ] {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("static permission should map: {}", error.message));
            assert!(expression_contains_operation(&mapped.expression, operation));
        }
        assert!(map_line(
            "S:Mode$ CastWithFlash | ValidCard$ Card | ValidSA$ Activated.Equip | Caster$ You"
        )
        .is_err());
    }

    #[test]
    fn lowers_closed_presence_conditions_by_ability_kind() {
        let mana = map_line(
            "A:AB$ Mana | Cost$ T | Produced$ C | Amount$ 2 | IsPresent$ Land.YouCtrl | PresentCompare$ GE5 | SpellDescription$ Add mana.",
        )
        .unwrap_or_else(|error| panic!("conditional mana should map: {}", error.message));
        assert!(matches!(
            mana.timing,
            Some(Expression::Call {
                operation: Operation::TimingCondition,
                ..
            })
        ));

        let graveyard = map_line(
            "S:Mode$ Continuous | Affected$ Card.Self | AddKeyword$ Vigilance | IsPresent$ Lesson.YouCtrl | PresentZone$ Graveyard | Description$ Vigilance.",
        )
        .unwrap_or_else(|error| panic!("zone-bound presence should map: {}", error.message));
        assert!(expression_contains_operation(
            &graveyard.expression,
            Operation::ZoneIs
        ));
        assert!(expression_contains_operation(
            &graveyard.expression,
            Operation::OwnedBy
        ));
        assert!(!expression_contains_operation(
            &graveyard.expression,
            Operation::ControlledBy
        ));

        for zone in ["Command", "Stack", "Graveyard,Hand"] {
            let error = map_line(&format!(
                "S:Mode$ Continuous | Affected$ Card.Self | AddKeyword$ Vigilance | IsPresent$ Card.YouCtrl | PresentZone$ {zone} | Description$ Vigilance."
            ))
            .err()
            .unwrap_or_else(|| panic!("unsupported PresentZone must quarantine"));
            assert_eq!(error.code, "UNSUPPORTED_VALUE");
        }

        for line in [
            "S:Mode$ AlternativeCost | ValidSA$ Spell | ValidCard$ Card.Self | ValidPlayer$ You | Cost$ 0 | EffectZone$ All | IsPresent$ Card.IsCommander+YouCtrl | Description$ Free with commander.",
            "S:Mode$ Continuous | Affected$ Card.Self | AddKeyword$ Double Strike | IsPresent$ Equipment.Attached | PresentCompare$ GE2 | Description$ Double strike.",
            "S:Mode$ Continuous | Affected$ Creature.YouCtrl | AddPower$ 5 | AddToughness$ 5 | IsPresent$ Card.Self+counters_GE7_QUEST | Description$ Buff.",
        ] {
            let mapped = map_line(line).unwrap_or_else(|error| {
                panic!("conditional static should map: {}", error.message)
            });
            assert!(matches!(
                mapped.expression,
                Expression::Call {
                    operation: Operation::WhileCondition,
                    ..
                }
            ));
        }

        let script = parse_legacy_script(
            "conditional-trigger.txt",
            concat!(
                "Name:Conditional Trigger\n",
                "T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | TriggerZones$ Battlefield | IsPresent$ Artifact.YouCtrl | Execute$ TrigToken | TriggerDescription$ Token.\n",
                "SVar:TrigToken:DB$ Token | TokenScript$ c_1_1_thopter | TokenOwner$ You\n",
            ),
        )
        .unwrap_or_else(|error| panic!("conditional trigger should parse: {error}"));
        let context = MappingContext::from_script(&script);
        let (prefix, expression) = script
            .lines
            .iter()
            .find_map(|line| match &line.kind {
                LegacyLineKind::Ability { prefix, expression } => Some((*prefix, expression)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("conditional trigger has no root ability"));
        let mapped = map_legacy_ability_in_context(prefix, expression, &context)
            .unwrap_or_else(|error| panic!("conditional trigger should map: {}", error.message));
        assert!(matches!(
            mapped.event,
            Some(Expression::Call {
                operation: Operation::EventWhen,
                ..
            })
        ));
        assert!(matches!(
            mapped.expression,
            Expression::Call {
                operation: Operation::WhileCondition,
                ..
            }
        ));

        let unsupported = map_line(
            "A:AB$ Draw | IsPresent$ Card.Self | PresentCompare$ EQX | SpellDescription$ Draw.",
        )
        .err()
        .unwrap_or_else(|| panic!("dynamic presence comparison must quarantine"));
        assert_eq!(unsupported.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn lowers_standalone_condition_present_and_closed_defined_bindings() {
        let mapped = map_line(
            "A:AB$ Pump | Defined$ Self | NumAtt$ +1 | ConditionPresent$ Creature.YouCtrl | ConditionCompare$ GE2 | SpellDescription$ Pump.",
        )
        .unwrap_or_else(|error| panic!("standalone ConditionPresent should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.timing.unwrap_or_else(|| {
                panic!("standalone ConditionPresent should become an activation condition")
            }),
            Operation::AtLeast
        ));

        let graveyard = map_line(
            "A:AB$ Draw | NumCards$ 1 | ConditionZone$ Graveyard | ConditionPresent$ Card.YouOwn | ConditionCompare$ GE2 | SpellDescription$ Draw.",
        )
        .unwrap_or_else(|error| panic!("private-zone ConditionPresent should map: {}", error.message));
        assert!(expression_contains_operation(
            &graveyard.timing.unwrap_or_else(|| {
                panic!("graveyard ConditionPresent should become an activation condition")
            }),
            Operation::TimingCondition
        ));

        let remembered = map_line(
            "A:AB$ Draw | NumCards$ 1 | ConditionDefined$ Remembered | ConditionPresent$ Card | ConditionCompare$ GE1 | SpellDescription$ Draw.",
        )
        .unwrap_or_else(|error| panic!("closed remembered ConditionPresent should map: {}", error.message));
        let remembered_timing = remembered.timing.unwrap_or_else(|| {
            panic!("remembered condition should become an activation condition")
        });
        assert!(expression_contains_operation(
            &remembered_timing,
            Operation::Remembered
        ));

        let self_condition = map_line(
            "A:AB$ Draw | NumCards$ 1 | ConditionDefined$ Self | ConditionPresent$ Creature | ConditionCompare$ GE1 | SpellDescription$ Draw.",
        )
        .unwrap_or_else(|error| panic!("closed self ConditionPresent should map: {}", error.message));
        assert!(expression_contains_operation(
            &self_condition.timing.unwrap_or_else(|| {
                panic!("self condition should become an activation condition")
            }),
            Operation::Equals
        ));

        let error = map_line(
            "A:AB$ Draw | NumCards$ 1 | ConditionDefined$ Remembered | ConditionZone$ Graveyard | ConditionPresent$ Card | ConditionCompare$ GE1 | SpellDescription$ Draw.",
        )
        .err()
        .unwrap_or_else(|| panic!("defined private-zone condition must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn lowers_named_and_svar_conditions_by_ability_kind() {
        let static_ability = map_line(
            "S:Mode$ Continuous | Affected$ Card.Self | AddKeyword$ Flying | Condition$ PlayerTurn | Description$ Conditional.",
        )
        .unwrap_or_else(|error| panic!("named static condition should map: {}", error.message));
        assert!(expression_contains_operation(
            &static_ability.expression,
            Operation::WhileCondition
        ));
        assert!(expression_contains_operation(
            &static_ability.expression,
            Operation::During
        ));

        let described = map_line(
            "A:SP$ Draw | Defined$ You | NumCards$ 1 | Condition$ Blessing | ConditionDescription$ If you have the city's blessing, | SpellDescription$ draw a card.",
        )
        .unwrap_or_else(|error| panic!("condition description metadata should map: {}", error.message));
        assert!(expression_contains_operation(
            &described.expression,
            Operation::WhileCondition
        ));

        let event_dependent = map_script_root(concat!(
            "Name:Evolve Trigger\n",
            "T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Creature.YouCtrl+Other | Condition$ Evolve | Execute$ TrigCounter | TriggerDescription$ Evolve.\n",
            "SVar:TrigCounter:DB$ PutCounter | Defined$ Self | CounterType$ P1P1 | CounterNum$ 1\n",
        ))
        .err()
        .unwrap_or_else(|| panic!("event-dependent condition must quarantine"));
        assert_eq!(event_dependent.code, "UNSUPPORTED_VALUE");

        let trigger = map_script_root(concat!(
            "Name:Compared Trigger\n",
            "T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | TriggerZones$ Battlefield | CheckSVar$ X | SVarCompare$ GE2 | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
            "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
            "SVar:X:Count$Valid Creature.YouCtrl\n",
        ))
        .unwrap_or_else(|error| panic!("SVar trigger condition should map: {}", error.message));
        let event = trigger
            .event
            .as_ref()
            .unwrap_or_else(|| panic!("conditional trigger should retain an event"));
        assert!(expression_contains_operation(event, Operation::EventWhen));
        assert!(expression_contains_operation(
            &trigger.expression,
            Operation::WhileCondition
        ));
        assert!(expression_contains_operation(event, Operation::AtLeast));

        let activation = map_script_root(concat!(
            "Name:Compared Activation\n",
            "A:AB$ Draw | Defined$ You | CheckSVar$ X | SVarCompare$ NE0 | SpellDescription$ Draw.\n",
            "SVar:X:Count$Valid Artifact.YouCtrl\n",
        ))
        .unwrap_or_else(|error| panic!("SVar activation condition should map: {}", error.message));
        let timing = activation
            .timing
            .as_ref()
            .unwrap_or_else(|| panic!("conditional activation should have typed timing"));
        assert!(expression_contains_operation(
            timing,
            Operation::TimingCondition
        ));
        assert!(expression_contains_operation(timing, Operation::Not));

        let condition_check = map_script_root(concat!(
            "Name:Condition SVar Activation\n",
            "A:AB$ Draw | Defined$ You | ConditionCheckSVar$ X | ConditionSVarCompare$ GE2 | SpellDescription$ Draw.\n",
            "SVar:X:Count$Valid Artifact.YouCtrl\n",
        ))
        .unwrap_or_else(|error| {
            panic!(
                "ConditionCheckSVar activation condition should map: {}",
                error.message
            )
        });
        let timing = condition_check
            .timing
            .as_ref()
            .unwrap_or_else(|| panic!("ConditionCheckSVar should create typed timing"));
        assert!(expression_contains_operation(
            timing,
            Operation::TimingCondition
        ));
        assert!(expression_contains_operation(timing, Operation::AtLeast));

        let cast_count = map_script_root(concat!(
            "Name:Second Spell Trigger\n",
            "T:Mode$ SpellCast | ValidCard$ Card.YouCtrl | ValidActivatingPlayer$ You | ActivatorThisTurnCast$ EQ2 | Execute$ TrigCounter | TriggerZones$ Battlefield | TriggerDescription$ Second spell.\n",
            "SVar:TrigCounter:DB$ PutCounter | Defined$ Self | CounterType$ P1P1 | CounterNum$ 1\n",
        ))
        .unwrap_or_else(|error| {
            panic!(
                "ActivatorThisTurnCast trigger condition should map: {}",
                error.message
            )
        });
        let event = cast_count
            .event
            .as_ref()
            .unwrap_or_else(|| panic!("cast-count trigger should retain an event"));
        assert!(expression_contains_operation(event, Operation::EventWhen));
        assert!(expression_contains_operation(
            event,
            Operation::HistoryCount
        ));
        assert!(expression_contains_operation(event, Operation::Equals));
        assert!(expression_contains_operation(
            event,
            Operation::ControlledBy
        ));

        let opponent_turn = map_script_root(concat!(
            "Name:Opponent Turn Trigger\n",
            "T:Mode$ SpellCast | ValidCard$ Card | ValidActivatingPlayer$ You | ActivatorThisTurnCast$ EQ1 | OpponentTurn$ True | Execute$ TrigToken | TriggerZones$ Battlefield | TriggerDescription$ First spell on opponent turn.\n",
            "SVar:TrigToken:DB$ Token | TokenScript$ b_1_1_faerie_rogue_flying | TokenOwner$ You\n",
        ))
        .unwrap_or_else(|error| {
            panic!(
                "OpponentTurn trigger condition should map: {}",
                error.message
            )
        });
        let event = opponent_turn
            .event
            .as_ref()
            .unwrap_or_else(|| panic!("opponent-turn trigger should retain an event"));
        assert!(expression_contains_operation(event, Operation::And));
        assert!(expression_contains_operation(event, Operation::During));
        assert!(expression_contains_operation(event, Operation::Not));

        let bad_opponent_turn = map_script_root(concat!(
            "Name:Bad Opponent Turn Trigger\n",
            "T:Mode$ SpellCast | ValidCard$ Card | ValidActivatingPlayer$ You | OpponentTurn$ False | Execute$ TrigDraw | TriggerZones$ Battlefield | TriggerDescription$ Bad.\n",
            "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
        ))
        .err()
        .unwrap_or_else(|| panic!("non-true OpponentTurn must quarantine"));
        assert_eq!(bad_opponent_turn.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn lowers_blocker_and_target_player_parameters() {
        let restriction = map_line(
            "S:Mode$ CantBlockBy | ValidAttacker$ Creature.Self | ValidBlocker$ Creature.powerLE2 | Description$ Evasive.",
        )
        .unwrap_or_else(|error| panic!("blocker restriction should map: {}", error.message));
        assert!(expression_contains_operation(
            &restriction.expression,
            Operation::CannotBeBlockedBy
        ));
        assert!(expression_contains_operation(
            &restriction.expression,
            Operation::LessThan
        ));

        let aggregate_blocked = map_script_root(concat!(
            "Name:Blocked Trigger\n",
            "T:Mode$ AttackerBlocked | ValidCard$ Creature | ValidBlocker$ Card.Self | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
            "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
        ))
        .err()
        .unwrap_or_else(|| panic!("aggregate blocker filter must quarantine"));
        assert_eq!(aggregate_blocked.code, "UNSUPPORTED_PARAMETER");

        let per_blocker = map_script_root(concat!(
            "Name:Per Blocker Trigger\n",
            "T:Mode$ AttackerBlockedByCreature | ValidCard$ Creature | ValidBlocker$ Card.Self | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
            "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
        ))
        .unwrap_or_else(|error| panic!("per-blocker trigger should map: {}", error.message));
        let per_blocker_event = per_blocker
            .event
            .as_ref()
            .unwrap_or_else(|| panic!("per-blocker trigger should have an event"));
        assert!(matches!(
            per_blocker_event,
            Expression::Call {
                operation: Operation::EventBlocks,
                arguments,
            } if matches!(
                arguments.get(1),
                Some(Expression::Call {
                    operation: Operation::Source,
                    ..
                })
            )
        ));

        for line in [
            "A:SP$ Draw | NumCards$ 3 | ValidTgts$ Player | SpellDescription$ Draw.",
            "A:SP$ Token | TokenScript$ g_3_3_beast | TokenOwner$ Targeted | ValidTgts$ Opponent | SpellDescription$ Token.",
            "A:SP$ DestroyAll | ValidCards$ Creature | ValidTgts$ Opponent | SpellDescription$ Destroy.",
            "A:SP$ PumpAll | ValidCards$ Creature | ValidTgts$ Player | NumAtt$ -2 | NumDef$ -2 | SpellDescription$ Pump.",
        ] {
            let mapped = map_line(line).unwrap_or_else(|error| {
                panic!("target-player fixture should map: {}", error.message)
            });
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::Target
            ));
        }
    }

    #[test]
    fn lowers_unless_zone_hidden_and_targeted_bindings_fail_closed() {
        let zoned = map_line(
            "A:AB$ Draw | Defined$ You | ActivationZone$ Graveyard | SorcerySpeed$ True | IsPresent$ Creature.YouCtrl | PresentCompare$ GE1 | SpellDescription$ Draw.",
        )
        .unwrap_or_else(|error| panic!("combined source-zone timing should map: {}", error.message));
        let timing = zoned
            .timing
            .as_ref()
            .unwrap_or_else(|| panic!("source-zone activation should retain timing"));
        assert!(expression_contains_operation(timing, Operation::TimingAll));
        assert!(expression_contains_operation(
            timing,
            Operation::TimingCondition
        ));
        assert!(expression_contains_operation(timing, Operation::ZoneIs));
        assert!(expression_contains_operation(timing, Operation::AtLeast));
        assert!(expression_contains_operation(
            timing,
            Operation::TimingSorcery
        ));

        let targeted = map_line(
            "A:SP$ Pump | Defined$ Targeted | NumAtt$ 1 | NumDef$ 1 | SpellDescription$ Pump.",
        )
        .unwrap_or_else(|error| panic!("targeted object binding should map: {}", error.message));
        assert!(expression_contains_operation(
            &targeted.expression,
            Operation::Target
        ));

        let hidden_hand = map_line(
            "A:SP$ ChangeZone | Hidden$ True | Origin$ Hand | Destination$ Exile | ValidTgts$ Card | SpellDescription$ Exile.",
        )
        .unwrap_or_else(|error| {
            panic!("intrinsically hidden-zone move should map: {}", error.message)
        });
        assert!(expression_contains_operation(
            &hidden_hand.expression,
            Operation::HiddenInformation
        ));
        let replacement_hidden = map_line(
            "A:DB$ ChangeZone | Hidden$ True | Origin$ All | Destination$ Exile | Defined$ ReplacedCard",
        )
        .unwrap_or_else(|error| {
            panic!("public-zone Hidden must retain metadata: {}", error.message)
        });
        assert!(expression_contains_operation(
            &replacement_hidden.expression,
            Operation::HiddenInformation
        ));
        assert!(expression_contains_operation(
            &replacement_hidden.expression,
            Operation::Exile
        ));

        let unless = map_script_root(concat!(
            "Name:Unless Chain\n",
            "A:SP$ Counter | TargetType$ Spell | ValidTgts$ Card | UnlessCost$ 2 | UnlessPayer$ TargetedController | SubAbility$ DBLife | SpellDescription$ Counter.\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("typed unless-paid chain should map: {}", error.message));
        assert!(matches!(
            &unless.expression,
            Expression::Call {
                operation: Operation::Sequence,
                arguments,
            } if matches!(
                arguments.first(),
                Some(Expression::Call {
                    operation: Operation::UnlessPaid,
                    ..
                })
            ) && matches!(
                arguments.get(1),
                Some(Expression::Call {
                    operation: Operation::GainLife,
                    ..
                })
            )
        ));
        assert!(expression_contains_operation(
            &unless.expression,
            Operation::ManaCost
        ));

        let payer_relative = map_line(
            "A:SP$ Destroy | ValidTgts$ Creature | UnlessCost$ Sac<1/Creature> | UnlessPayer$ TargetedController | SpellDescription$ Destroy.",
        )
        .unwrap_or_else(|error| panic!("payer-relative sacrifice should map: {}", error.message));
        assert!(expression_contains_operation(
            &payer_relative.expression,
            Operation::Sacrifice
        ));
        assert!(expression_contains_operation(
            &payer_relative.expression,
            Operation::ControllerOf
        ));

        let conditional_chain = map_script_root(concat!(
            "Name:Conditional Chain\n",
            "A:SP$ Draw | Defined$ You | IsPresent$ Creature.YouCtrl | SubAbility$ DBLife | SpellDescription$ Draw.\n",
            "SVar:DBLife:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
        ))
        .unwrap_or_else(|error| panic!("conditional chain should map: {}", error.message));
        assert!(matches!(
            &conditional_chain.expression,
            Expression::Call {
                operation: Operation::Sequence,
                arguments,
            } if matches!(
                arguments.first(),
                Some(Expression::Call {
                    operation: Operation::WhileCondition,
                    ..
                })
            ) && matches!(
                arguments.get(1),
                Some(Expression::Call {
                    operation: Operation::GainLife,
                    ..
                })
            )
        ));

        for line in [
            "A:SP$ Draw | Defined$ You | ActivationZone$ Sideboard | SpellDescription$ Draw.",
            "A:SP$ Draw | Defined$ You | UnlessPayer$ You | SpellDescription$ Draw.",
            "A:SP$ Draw | Defined$ You | UnlessCost$ Y | UnlessPayer$ You | SpellDescription$ Draw.",
            "A:SP$ Draw | Defined$ You | UnlessCost$ PayEnergy<2> | UnlessPayer$ You | SpellDescription$ Draw.",
        ] {
            assert!(map_line(line).is_err(), "open timing/unless form must quarantine");
        }
    }

    #[test]
    fn lowers_closed_dynamic_svar_values() {
        for (script_text, expected_value) in [
            (
                concat!(
                    "Name:Dynamic Tokens\n",
                    "A:AB$ Token | Cost$ T | TokenAmount$ X | TokenScript$ r_1_1_goblin | TokenOwner$ You | SpellDescription$ Tokens.\n",
                    "SVar:X:Count$Valid Goblin.YouCtrl\n",
                ),
                Operation::Count,
            ),
            (
                concat!(
                    "Name:Greatest Power\n",
                    "A:SP$ GainLife | Defined$ You | LifeAmount$ X | SpellDescription$ Gain life.\n",
                    "SVar:X:Count$Valid Creature.YouCtrl$GreatestCardPower\n",
                ),
                Operation::Aggregate,
            ),
            (
                concat!(
                    "Name:Life Total\n",
                    "A:SP$ GainLife | Defined$ You | LifeAmount$ X | SpellDescription$ Gain life.\n",
                    "SVar:X:Count$YourLifeTotal\n",
                ),
                Operation::LifeTotal,
            ),
            (
                concat!(
                    "Name:Dynamic Animation\n",
                    "A:SP$ Animate | Defined$ Self | Types$ Creature | Power$ X | Toughness$ X | Duration$ Permanent | SpellDescription$ Animate.\n",
                    "SVar:X:Count$Valid Creature.YouCtrl\n",
                ),
                Operation::Count,
            ),
            (
                concat!(
                    "Name:Morbid Count\n",
                    "A:SP$ GainLife | Defined$ You | LifeAmount$ X | SpellDescription$ Gain life.\n",
                    "SVar:X:Count$Morbid.1.0\n",
                ),
                Operation::HistoryCount,
            ),
            (
                concat!(
                    "Name:Discard History\n",
                    "A:SP$ GainLife | Defined$ You | LifeAmount$ X | SpellDescription$ Gain life.\n",
                    "SVar:X:PlayerCountPropertyYou$CardsDiscardedThisTurn\n",
                ),
                Operation::HistoryCount,
            ),
            (
                concat!(
                    "Name:Target Graveyard Count\n",
                    "A:SP$ Mill | ValidTgts$ Player | NumCards$ X | SpellDescription$ Mill.\n",
                    "SVar:X:TargetedPlayer$CardsInGraveyard\n",
                ),
                Operation::Count,
            ),
            (
                concat!(
                    "Name:Domain Count\n",
                    "A:SP$ GainLife | Defined$ You | LifeAmount$ X | SpellDescription$ Gain life.\n",
                    "SVar:X:Count$Domain\n",
                ),
                Operation::DistinctCount,
            ),
            (
                concat!(
                    "Name:Creatures Died\n",
                    "A:SP$ GainLife | Defined$ You | LifeAmount$ X | SpellDescription$ Gain life.\n",
                    "SVar:X:Count$ThisTurnEntered_Graveyard_from_Battlefield_Creature.YouCtrl\n",
                ),
                Operation::HistoryCount,
            ),
            (
                concat!(
                    "Name:Paid X Tokens\n",
                    "A:SP$ Token | TokenAmount$ X | TokenScript$ w_1_1_warrior | TokenOwner$ You | SpellDescription$ Tokens.\n",
                    "SVar:X:Count$xPaid\n",
                ),
                Operation::PaidX,
            ),
            (
                concat!(
                    "Name:Target Power\n",
                    "A:SP$ ChangeZone | ValidTgts$ Creature | Origin$ Battlefield | Destination$ Exile | SubAbility$ DBGainLife | SpellDescription$ Exile.\n",
                    "SVar:DBGainLife:DB$ GainLife | Defined$ TargetedController | LifeAmount$ X\n",
                    "SVar:X:Targeted$CardPower\n",
                ),
                Operation::Power,
            ),
            (
                concat!(
                    "Name:Triggered Card Power\n",
                    "T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Card.Self | TriggerZones$ Battlefield | Execute$ TrigDamage | TriggerDescription$ Damage.\n",
                    "SVar:TrigDamage:DB$ DealDamage | Defined$ You | NumDmg$ X\n",
                    "SVar:X:TriggeredCard$CardPower\n",
                ),
                Operation::Power,
            ),
            (
                concat!(
                    "Name:Triggered Card Counter\n",
                    "A:AB$ DealDamage | ValidTgts$ Any | NumDmg$ X | SpellDescription$ Damage.\n",
                    "SVar:X:TriggeredCard$CardCounters.P1P1\n",
                ),
                Operation::CounterCount,
            ),
            (
                concat!(
                    "Name:Dynamic Mana\n",
                    "A:AB$ Mana | Cost$ T | Produced$ G | Amount$ X | SpellDescription$ Mana.\n",
                    "SVar:X:Count$Valid Elf.YouCtrl\n",
                ),
                Operation::Count,
            ),
            (
                concat!(
                    "Name:Counter Mana\n",
                    "A:AB$ Mana | Cost$ T | Produced$ G | Amount$ X | SpellDescription$ Mana.\n",
                    "SVar:X:Count$CardCounters.P1P1\n",
                ),
                Operation::CounterCount,
            ),
            (
                concat!(
                    "Name:Devotion Mana\n",
                    "A:AB$ Mana | Cost$ T | Produced$ G | Amount$ X | SpellDescription$ Mana.\n",
                    "SVar:X:Count$Devotion.Green\n",
                ),
                Operation::Devotion,
            ),
            (
                concat!(
                    "Name:Distinct Colors\n",
                    "S:Mode$ Continuous | Affected$ Card.Self | AddPower$ X | AddToughness$ X | Description$ Buff.\n",
                    "SVar:X:Count$Valid Permanent.YouCtrl$Colors\n",
                ),
                Operation::DistinctCount,
            ),
            (
                concat!(
                    "Name:Turn History\n",
                    "T:Mode$ SpellCast | ValidCard$ Card | ValidActivatingPlayer$ You | Execute$ TrigLife | TriggerZones$ Battlefield | TriggerDescription$ Life.\n",
                    "SVar:TrigLife:DB$ GainLife | Defined$ You | LifeAmount$ X\n",
                    "SVar:X:Count$ThisTurnCast_Card.YouCtrl\n",
                ),
                Operation::HistoryCount,
            ),
            (
                concat!(
                    "Name:Hand Count\n",
                    "A:SP$ GainLife | Defined$ You | LifeAmount$ X | SpellDescription$ Gain life.\n",
                    "SVar:X:Count$ValidHand Card.YouOwn\n",
                ),
                Operation::Count,
            ),
            (
                concat!(
                    "Name:Raid Count\n",
                    "A:SP$ GainLife | Defined$ You | LifeAmount$ X | SpellDescription$ Gain life.\n",
                    "SVar:X:Count$AttackersDeclared\n",
                ),
                Operation::HistoryCount,
            ),
            (
                concat!(
                    "Name:Negative Dynamic Power\n",
                    "A:SP$ Pump | ValidTgts$ Creature | NumAtt$ -X | NumDef$ -X | SpellDescription$ Shrink.\n",
                    "SVar:X:Count$CardPower\n",
                ),
                Operation::Negate,
            ),
            (
                concat!(
                    "Name:Triggered Damage Amount\n",
                    "T:Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Player | TriggerZones$ Battlefield | Execute$ TrigLife | TriggerDescription$ Life.\n",
                    "SVar:TrigLife:DB$ GainLife | Defined$ You | LifeAmount$ X\n",
                    "SVar:X:TriggerCount$DamageAmount\n",
                ),
                Operation::TriggeredAmount,
            ),
            (
                concat!(
                    "Name:Opponent Count\n",
                    "A:SP$ Token | TokenAmount$ X | TokenScript$ r_1_1_goblin | TokenOwner$ You | SpellDescription$ Tokens.\n",
                    "SVar:X:PlayerCountOpponents$Amount\n",
                ),
                Operation::OpponentCount,
            ),
            (
                concat!(
                    "Name:Player Count\n",
                    "A:SP$ Token | TokenAmount$ X | TokenScript$ r_1_1_goblin | TokenOwner$ You | SpellDescription$ Tokens.\n",
                    "SVar:X:PlayerCountPlayers$Amount\n",
                ),
                Operation::PlayerCount,
            ),
            (
                concat!(
                    "Name:Lowest Player Life\n",
                    "A:SP$ GainLife | Defined$ You | LifeAmount$ X | SpellDescription$ Gain life.\n",
                    "SVar:X:PlayerCountPlayers$LowestLifeTotal\n",
                ),
                Operation::PlayerAggregate,
            ),
            (
                concat!(
                    "Name:Sacrificed Power\n",
                    "A:AB$ DealDamage | Cost$ Sac<1/Creature> | ValidTgts$ Any | NumDmg$ X | SpellDescription$ Damage.\n",
                    "SVar:X:Sacrificed$CardPower\n",
                ),
                Operation::Remembered,
            ),
            (
                concat!(
                    "Name:Sacrificed Toughness\n",
                    "A:AB$ GainLife | Cost$ Sac<1/Creature> | Defined$ You | LifeAmount$ X | SpellDescription$ Life.\n",
                    "SVar:X:Sacrificed$CardToughness\n",
                ),
                Operation::Remembered,
            ),
            (
                concat!(
                    "Name:Sacrificed Mana Value\n",
                    "A:AB$ Mill | Cost$ Sac<1/Creature> | NumCards$ X | ValidTgts$ Player | SpellDescription$ Mill.\n",
                    "SVar:X:Sacrificed$CardManaCost\n",
                ),
                Operation::Remembered,
            ),
            (
                concat!(
                    "Name:Remembered Amount\n",
                    "A:SP$ Draw | Defined$ You | NumCards$ X | SpellDescription$ Draw.\n",
                    "SVar:X:Remembered$Amount\n",
                ),
                Operation::Count,
            ),
            (
                concat!(
                    "Name:Remembered Mana Value\n",
                    "A:SP$ GainLife | Defined$ You | LifeAmount$ X | SpellDescription$ Gain.\n",
                    "SVar:X:Remembered$CardManaCost\n",
                ),
                Operation::Aggregate,
            ),
            (
                concat!(
                    "Name:Parent Target Power\n",
                    "A:SP$ Pump | ValidTgts$ Creature.YouCtrl | NumAtt$ +1 | NumDef$ +1 | SubAbility$ DBDamage | SpellDescription$ Pump and deal damage.\n",
                    "SVar:DBDamage:DB$ DealDamage | ValidTgts$ Creature.YouDontCtrl | NumDmg$ X\n",
                    "SVar:X:ParentTargeted$CardPower\n",
                ),
                Operation::ParentTarget,
            ),
        ] {
            let script = parse_legacy_script("dynamic-value.txt", script_text)
                .unwrap_or_else(|error| panic!("dynamic fixture should parse: {error}"));
            let context = MappingContext::from_script(&script);
            let (prefix, expression) = script
                .lines
                .iter()
                .find_map(|line| match &line.kind {
                    LegacyLineKind::Ability { prefix, expression } => Some((*prefix, expression)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("dynamic fixture has no root ability"));
            let mapped = map_legacy_ability_in_context(prefix, expression, &context)
                .unwrap_or_else(|error| panic!("dynamic value should map: {}", error.message));
            assert!(
                expression_contains_operation(&mapped.expression, expected_value),
                "{} is missing {}",
                script_text.lines().next().unwrap_or("dynamic fixture"),
                expected_value.as_str()
            );
            if script_text.starts_with("Name:Triggered Card") {
                assert!(
                    expression_contains_operation(&mapped.expression, Operation::Triggered),
                    "{} must retain the triggered-card selector",
                    script_text.lines().next().unwrap_or("triggered-value fixture")
                );
            }
        }

        for open_value in [
            "LifeAmount/Times.2",
            "ScryBottom",
            "DamageAmount/LimitMax.11",
        ] {
            let script = parse_legacy_script(
                "open-trigger-count.txt",
                &format!(
                    "A:SP$ GainLife | Defined$ You | LifeAmount$ X\nSVar:X:TriggerCount${open_value}\n"
                ),
            )
            .unwrap_or_else(|error| panic!("open trigger-count fixture should parse: {error}"));
            let context = MappingContext::from_script(&script);
            let LegacyLineKind::Ability { prefix, expression } = &script.lines[0].kind else {
                panic!("open trigger-count fixture has no ability");
            };
            let error = map_legacy_ability_in_context(*prefix, expression, &context)
                .err()
                .unwrap_or_else(|| panic!("open trigger count must quarantine"));
            assert_eq!(error.code, "UNSUPPORTED_VALUE_SVAR");
        }

        let open_opponents = parse_legacy_script(
            "open-opponent-count.txt",
            "A:SP$ Token | TokenAmount$ X | TokenScript$ r_1_1_goblin | TokenOwner$ You\nSVar:X:PlayerCountOpponents$HighestCardsInHand\n",
        )
        .unwrap_or_else(|error| panic!("open opponent-count fixture should parse: {error}"));
        let context = MappingContext::from_script(&open_opponents);
        let LegacyLineKind::Ability { prefix, expression } = &open_opponents.lines[0].kind else {
            panic!("open opponent-count fixture has no ability");
        };
        let error = map_legacy_ability_in_context(*prefix, expression, &context)
            .err()
            .unwrap_or_else(|| panic!("open opponent count must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE_SVAR");

        let script = parse_legacy_script(
            "open-sacrifice-value.txt",
            concat!(
                "A:AB$ Mill | Cost$ Sac<1/Creature> | NumCards$ X | ValidTgts$ Player | SpellDescription$ Mill.\n",
                "SVar:X:Sacrificed$Valid Card.IsSuspected\n",
            ),
        )
        .unwrap_or_else(|error| panic!("open sacrificed fixture should parse: {error}"));
        let context = MappingContext::from_script(&script);
        let LegacyLineKind::Ability { prefix, expression } = &script.lines[0].kind else {
            panic!("open sacrificed fixture has no ability");
        };
        let error = map_legacy_ability_in_context(*prefix, expression, &context)
            .err()
            .unwrap_or_else(|| panic!("open sacrificed characteristic must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE_SVAR");
    }

    #[test]
    fn resolves_etb_and_upkeep_svar_effect_graphs() {
        for (script_text, expected_event, expected_effect) in [
            (
                concat!(
                    "Name:Graph ETB\n",
                    "T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You | NumCards$ 2\n",
                ),
                Operation::EventEnters,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Dies\n",
                    "T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Creature.YouCtrl | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventZoneChange,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Leaves\n",
                    "T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Any | ValidCard$ Card.Self | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventLeaves,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Leaves Exile\n",
                    "T:Mode$ ChangesZone | Origin$ Exile | Destination$ Any | ValidCard$ Card.Self | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventZoneChange,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Tapped\n",
                    "T:Mode$ Taps | ValidCard$ Card.Self | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventTapped,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Life Gained\n",
                    "T:Mode$ LifeGained | ValidPlayer$ You | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventLifeGained,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Cycled\n",
                    "T:Mode$ Cycled | ValidCard$ Card.Self | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventCycled,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Sacrificed\n",
                    "T:Mode$ Sacrificed | ValidCard$ Artifact.YouCtrl | ValidPlayer$ You | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventSacrificed,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Batch Zone Change\n",
                    "T:Mode$ ChangesZoneAll | ValidCards$ Creature.YouCtrl | Origin$ Battlefield | Destination$ Graveyard | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventZoneChangeAll,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Upkeep\n",
                    "T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigLife | TriggerDescription$ Gain life.\n",
                    "SVar:TrigLife:DB$ GainLife | Defined$ You | LifeAmount$ 1 | SubAbility$ TrigDraw\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventUpkeep,
                Operation::Sequence,
            ),
            (
                concat!(
                    "Name:Graph End Step\n",
                    "T:Mode$ Phase | Phase$ End of Turn | ValidPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventPhase,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Beginning of Combat\n",
                    "T:Mode$ Phase | Phase$ BeginCombat | ValidPlayer$ Opponent | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventPhase,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Attacks\n",
                    "T:Mode$ Attacks | ValidCard$ Card.Self | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventAttacks,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Spell Cast\n",
                    "T:Mode$ SpellCast | ValidCard$ Instant,Sorcery | ValidActivatingPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigCounter | TriggerDescription$ Counter.\n",
                    "SVar:TrigCounter:DB$ PutCounter | Defined$ Self | CounterType$ P1P1 | CounterNum$ 1\n",
                ),
                Operation::EventCast,
                Operation::AddCounter,
            ),
            (
                concat!(
                    "Name:Graph Spell Cast Or Copy\n",
                    "T:Mode$ SpellCastOrCopy | ValidCard$ Instant,Sorcery | ValidActivatingPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventCast,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Damage\n",
                    "T:Mode$ DamageDone | ValidSource$ Card.Self | ValidTarget$ Player.Opponent | CombatDamage$ True | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventDamage,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Damage Once\n",
                    "T:Mode$ DamageDoneOnce | ValidSource$ Creature.YouCtrl | ValidTarget$ Player | CombatDamage$ True | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventDamage,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Damage Dealt Once\n",
                    "T:Mode$ DamageDealtOnce | ValidSource$ Card.Self | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventDamage,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Draw\n",
                    "T:Mode$ Drawn | ValidCard$ Card.OppOwn | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventDraw,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Attack Declaration\n",
                    "T:Mode$ AttackersDeclared | AttackingPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventAttacks,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Turn Face Up\n",
                    "T:Mode$ TurnFaceUp | ValidCard$ Card.Self | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventTurnedFaceUp,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Blocks\n",
                    "T:Mode$ Blocks | ValidCard$ Card.Self | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventBlocks,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Attacker Blocked\n",
                    "T:Mode$ AttackerBlocked | ValidCard$ Card.Self | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventBlocks,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Attacker Blocked By Creature\n",
                    "T:Mode$ AttackerBlockedByCreature | ValidCard$ Card.Self | ValidBlocker$ Creature | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventBlocks,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Attacker Unblocked\n",
                    "T:Mode$ AttackerUnblocked | ValidCard$ Card.Self | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventAttacks,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Targeted\n",
                    "T:Mode$ BecomesTarget | ValidTarget$ Card.Self | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventTargeted,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Discarded\n",
                    "T:Mode$ Discarded | ValidCard$ Card.OppOwn | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventDiscard,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Counter Added\n",
                    "T:Mode$ CounterAddedOnce | ValidCard$ Card.Self | CounterType$ P1P1 | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventCounterAdded,
                Operation::Draw,
            ),
            (
                concat!(
                    "Name:Graph Optional Draw\n",
                    "T:Mode$ Drawn | ValidCard$ Card.YouOwn | OptionalDecider$ You | Secondary$ True | TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
                    "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
                ),
                Operation::EventDraw,
                Operation::ChooseUpTo,
            ),
        ] {
            let script = parse_legacy_script("graph.txt", script_text)
                .unwrap_or_else(|error| panic!("graph fixture should parse: {error}"));
            let context = MappingContext::from_script(&script);
            let ability = script
                .lines
                .iter()
                .find_map(|line| match &line.kind {
                    LegacyLineKind::Ability { prefix, expression } => Some((*prefix, expression)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("graph fixture has no root ability"));
            let mapped = map_legacy_ability_in_context(ability.0, ability.1, &context)
                .unwrap_or_else(|error| panic!("graph should map: {}", error.message));
            assert!(matches!(
                mapped.event,
                Some(Expression::Call {
                    operation,
                    ..
                }) if operation == expected_event
            ));
            assert!(matches!(
                mapped.expression,
                Expression::Call { operation, .. } if operation == expected_effect
            ));
        }
    }

    #[test]
    fn preserves_closed_phase_event_names_and_rejects_open_values() {
        for (phase, player, expected_player, expected_phase) in [
            ("End of Turn", "You", Operation::You, "end_step"),
            (
                "BeginCombat",
                "Opponent",
                Operation::Opponent,
                "begin_combat",
            ),
            ("Main1", "You", Operation::You, "precombat_main"),
            ("Main2", "You", Operation::You, "postcombat_main"),
            ("Main", "You", Operation::You, "main_phase"),
            ("Draw", "You", Operation::You, "draw_step"),
            ("Cleanup", "You", Operation::You, "cleanup_step"),
            ("EndCombat", "You", Operation::You, "end_combat"),
            (
                "Declare Attackers",
                "You",
                Operation::You,
                "declare_attackers",
            ),
            ("Untap", "You", Operation::You, "untap_step"),
            ("End Of Turn", "You", Operation::You, "end_step"),
        ] {
            let mapped = map_script_root(&format!(
                "T:Mode$ Phase | Phase$ {phase} | ValidPlayer$ {player} | TriggerZones$ Battlefield | Execute$ TrigDraw\nSVar:TrigDraw:DB$ Draw | Defined$ You\n"
            ))
            .unwrap_or_else(|error| panic!("closed phase should map: {}", error.message));
            assert!(matches!(
                mapped.event,
                Some(Expression::Call {
                    operation: Operation::EventPhase,
                    arguments,
                }) if matches!(arguments.as_slice(), [
                    Expression::Call { operation, arguments: selector_args },
                    Expression::Text(value),
                ] if *operation == expected_player && selector_args.is_empty() && value == expected_phase)
            ));
        }

        let limited = map_script_root(concat!(
            "T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | TriggerZones$ Battlefield | ActivationLimit$ 1 | Execute$ TrigDraw\n",
            "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
        ))
        .unwrap_or_else(|error| panic!("limited trigger should map: {}", error.message));
        assert!(expression_contains_operation(
            limited.event.as_ref().unwrap_or(&limited.expression),
            Operation::EventLimit
        ));
        assert!(map_script_root(concat!(
            "T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ You | TriggerZones$ Battlefield | ActivationLimit$ 4 | Execute$ TrigDraw\n",
            "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
        ))
        .is_err());

        for phase in ["EndOfTurn", "Combat", "Main1,Main2"] {
            let error = map_script_root(&format!(
                "T:Mode$ Phase | Phase$ {phase} | ValidPlayer$ You | TriggerZones$ Battlefield | Execute$ TrigDraw\nSVar:TrigDraw:DB$ Draw | Defined$ You\n"
            ))
            .err()
            .unwrap_or_else(|| panic!("open phase value must quarantine"));
            assert_eq!(error.code, "UNSUPPORTED_VALUE");
        }
    }

    #[test]
    fn rejects_missing_duplicate_and_cyclic_svars() {
        for (script_text, expected_code) in [
            (
                "A:SP$ Draw | SubAbility$ Missing | SpellDescription$ Missing.\n",
                "MISSING_SVAR",
            ),
            (
                concat!(
                    "A:SP$ Draw | SubAbility$ Again | SpellDescription$ Duplicate.\n",
                    "SVar:Again:DB$ Draw\n",
                    "SVar:Again:DB$ GainLife | LifeAmount$ 1\n",
                ),
                "DUPLICATE_SVAR",
            ),
            (
                concat!(
                    "A:SP$ Draw | SubAbility$ Loop | SpellDescription$ Cycle.\n",
                    "SVar:Loop:DB$ Draw | SubAbility$ Loop\n",
                ),
                "CYCLIC_SVAR",
            ),
        ] {
            let script = parse_legacy_script("bad-graph.txt", script_text)
                .unwrap_or_else(|error| panic!("bad graph fixture should parse: {error}"));
            let context = MappingContext::from_script(&script);
            let (prefix, expression) = script
                .lines
                .iter()
                .find_map(|line| match &line.kind {
                    LegacyLineKind::Ability { prefix, expression } => Some((*prefix, expression)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("bad graph fixture has no root ability"));
            let error = map_legacy_ability_in_context(prefix, expression, &context)
                .err()
                .unwrap_or_else(|| panic!("bad graph must quarantine"));
            assert_eq!(error.code, expected_code);
        }
    }

    #[test]
    fn quarantines_complex_or_approximate_cases() {
        let error = map_line(
            "A:SP$ Draw | NumCards$ 2 | SubAbility$ DBDiscard | SpellDescription$ Draw, then discard.",
        )
        .err()
        .unwrap_or_else(|| panic!("complex draw must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_PARAMETER");

        let dynamic = map_line(
            "A:SP$ DealDamage | ValidTgts$ Creature | NumDmg$ X | SpellDescription$ Dynamic damage.",
        )
        .err()
        .unwrap_or_else(|| panic!("dynamic damage must quarantine"));
        assert_eq!(dynamic.code, "UNSUPPORTED_VALUE");

        let chained = map_line(
            "A:SP$ Pump | ValidTgts$ Creature | NumAtt$ +2 | SubAbility$ DBTap | SpellDescription$ Complex pump.",
        )
        .err()
        .unwrap_or_else(|| panic!("chained pump must quarantine"));
        assert_eq!(chained.code, "UNSUPPORTED_PARAMETER");

        let qualified_target = map_line(
            "A:SP$ Destroy | ValidTgts$ Creature.counters_GE1_P1P1 | SpellDescription$ Qualified target.",
        )
        .err()
        .unwrap_or_else(|| panic!("qualified target must quarantine"));
        assert_eq!(qualified_target.code, "UNSUPPORTED_VALUE");

        let dynamic_continuous = map_line(
            "S:Mode$ Continuous | Affected$ Card.Self | AddPower$ X | Description$ Dynamic power.",
        )
        .err()
        .unwrap_or_else(|| panic!("dynamic continuous value must quarantine"));
        assert_eq!(dynamic_continuous.code, "UNSUPPORTED_VALUE");

        let unknown_condition = map_line(
            "S:Mode$ Continuous | Affected$ Card.Self | AddKeyword$ Flying | Condition$ UnclosedCondition | Description$ Conditional.",
        )
        .err()
        .unwrap_or_else(|| panic!("unknown condition must quarantine"));
        assert_eq!(unknown_condition.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn blocker_collection_peels_all_unknown_parameters_per_node() {
        let script = parse_legacy_script(
            "multi-parameter.txt",
            concat!(
                "A:SP$ Draw | Foo$ one | Bar$ two | NumCards$ 1\n",
                "SVar:Unused:DB$ Draw | Ignored$ unused\n",
            ),
        )
        .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let blockers = collect_script_mapping_blockers(&script);
        let messages = blockers
            .iter()
            .map(|blocker| blocker.message.as_str())
            .collect::<Vec<_>>();

        assert_eq!(blockers.len(), 2);
        assert!(messages.iter().any(|message| message.contains("`Bar`")));
        assert!(messages.iter().any(|message| message.contains("`Foo`")));
        assert!(messages.iter().all(|message| !message.contains("Ignored")));
    }

    #[test]
    fn audits_mapping_coverage_and_reason_coded_quarantine() {
        let root = std::env::temp_dir().join(format!(
            "forge-porttools-mapper-audit-{}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap_or_else(|error| {
                panic!("could not clear mapper fixture: {error}");
            });
        }
        let cards = root.join("cards");
        fs::create_dir_all(&cards).unwrap_or_else(|error| {
            panic!("could not create mapper fixture: {error}");
        });
        fs::write(
            cards.join("mapped.txt"),
            concat!(
                "Name:Mapped\n",
                "A:AB$ Mana | Cost$ T | Produced$ G | SpellDescription$ Add mana.\n",
                "A:SP$ Draw | NumCards$ 2 | SpellDescription$ Draw cards.\n",
                "A:SP$ DealDamage | ValidTgts$ Any | NumDmg$ 3 | SpellDescription$ Damage.\n",
            ),
        )
        .unwrap_or_else(|error| panic!("could not write mapped fixture: {error}"));
        fs::write(
            cards.join("quarantined.txt"),
            concat!(
                "Name:Quarantined\n",
                "A:SP$ Draw | Optional$ True | SpellDescription$ Complex.\n",
                "T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | Execute$ Trig\n",
            ),
        )
        .unwrap_or_else(|error| panic!("could not write quarantine fixture: {error}"));
        run_git(&root, &["init", "--quiet"]);
        run_git(
            &root,
            &["config", "user.email", "forge-test@example.invalid"],
        );
        run_git(&root, &["config", "user.name", "Forge Test"]);
        run_git(&root, &["add", "cards"]);
        run_git(&root, &["commit", "--quiet", "-m", "fixture"]);

        let metrics = root.join("api-coverage.json");
        let quarantine = root.join("api-quarantine.json");
        let report = audit_legacy_mappings(&cards, &metrics, &quarantine)
            .unwrap_or_else(|error| panic!("mapping audit should complete: {error}"));
        assert_eq!(report.legacy_uses, 5);
        assert_eq!(report.mapped_uses, 4);
        assert_eq!(report.verified_uses, 4);
        assert_eq!(report.quarantined_uses, 1);
        assert_eq!(
            report.quarantine_reason_counts.get("MISSING_SVAR"),
            Some(&1)
        );
        assert!(!report
            .quarantine_reason_counts
            .contains_key("UNSUPPORTED_PARAMETER"));
        assert!(metrics.is_file());
        assert!(quarantine.is_file());

        fs::remove_dir_all(&root).unwrap_or_else(|error| {
            panic!("could not remove mapper fixture: {error}");
        });
    }

    #[test]
    fn maps_closed_pump_at_eot_cleanup_values() {
        for (value, cleanup) in [
            ("Sacrifice", Operation::SacrificeEffect),
            ("Destroy", Operation::Destroy),
            ("Exile", Operation::Exile),
            ("Hand", Operation::ReturnToHand),
        ] {
            let mapped = map_line(&format!(
                "A:AB$ Pump | Defined$ Self | NumAtt$ +1 | AtEOT$ {value}"
            ))
            .unwrap_or_else(|error| panic!("closed AtEOT should map: {}", error.message));
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::RegisterDelayedTrigger
            ));
            assert!(expression_contains_operation(&mapped.expression, cleanup));
        }

        let cleanup_only = map_line("A:AB$ Pump | Defined$ Self | AtEOT$ Sacrifice")
            .unwrap_or_else(|error| panic!("cleanup-only AtEOT should map: {}", error.message));
        assert!(matches!(
            cleanup_only.expression,
            Expression::Call {
                operation: Operation::RegisterDelayedTrigger,
                ..
            }
        ));
    }

    #[test]
    fn rejects_open_pump_at_eot_cleanup_values() {
        for value in ["SacrificeCombat", "YourExile", "Library"] {
            let error = map_line(&format!("A:AB$ Pump | Defined$ Self | AtEOT$ {value}"))
                .err()
                .unwrap_or_else(|| panic!("open AtEOT value must quarantine: {value}"));
            assert_eq!(error.code, "UNSUPPORTED_VALUE");
        }
    }

    #[test]
    fn maps_permanent_pump_pt_and_keyword_effects() {
        for line in [
            "A:AB$ Pump | Defined$ Self | NumAtt$ +2 | Duration$ UntilYourNextTurn",
            "A:SP$ PumpAll | ValidCards$ Creature.YouCtrl | KW$ Indestructible | Duration$ UntilYourNextTurn",
            "A:DB$ Debuff | ValidTgts$ Creature | Keywords$ Flying | Duration$ UntilYourNextTurn",
        ] {
            map_line(line).unwrap_or_else(|error| {
                panic!("next-turn duration should map: {}", error.message)
            });
        }
        let pt = map_line("A:AB$ Pump | Defined$ Self | NumAtt$ +2 | Duration$ Permanent")
            .unwrap_or_else(|error| panic!("permanent PT Pump should map: {}", error.message));
        assert!(matches!(
            pt.expression,
            Expression::Call {
                operation: Operation::ModifyPt,
                arguments,
            } if arguments.len() == 3
        ));

        let keyword = map_line(
            "A:SP$ PumpAll | ValidCards$ Creature.YouCtrl | KW$ Indestructible | Duration$ Permanent",
        )
        .unwrap_or_else(|error| panic!("permanent keyword PumpAll should map: {}", error.message));
        assert!(matches!(
            keyword.expression,
            Expression::Call {
                operation: Operation::GrantKeyword,
                arguments,
            } if arguments.len() == 2
        ));
    }

    #[test]
    fn maps_perpetual_cross_zone_pumps_and_memory() {
        let mapped = map_line(
            "A:DB$ Pump | Defined$ Remembered | PumpZone$ Hand,Graveyard | NumAtt$ +1 | NumDef$ +1 | Duration$ Perpetual | RememberObjects$ Self",
        )
        .unwrap_or_else(|error| panic!("cross-zone perpetual pump should map: {}", error.message));
        for operation in [
            Operation::ApplyInZones,
            Operation::Perpetual,
            Operation::ModifyPt,
            Operation::Remember,
        ] {
            assert!(expression_contains_operation(&mapped.expression, operation));
        }

        let all = map_line(
            "A:DB$ PumpAll | ValidCards$ Creature.YouCtrl | PumpZone$ Battlefield,Hand,Library,Graveyard | NumAtt$ +1 | NumDef$ +1 | Duration$ Perpetual",
        )
        .unwrap_or_else(|error| panic!("cross-zone perpetual PumpAll should map: {}", error.message));
        assert!(expression_contains_operation(
            &all.expression,
            Operation::ApplyInZones
        ));
        assert!(expression_contains_operation(
            &all.expression,
            Operation::Perpetual
        ));
    }

    #[test]
    fn maps_closed_next_untap_and_rejects_mixed_permanent_cleanup() {
        let mapped = map_line(
            "A:AB$ Pump | Defined$ Self | KW$ HIDDEN This card doesn't untap during your next untap step. | Duration$ Permanent",
        )
        .unwrap_or_else(|error| panic!("closed untap restriction should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::CannotUntap
        ));

        let error = map_line(
            "A:AB$ Pump | Defined$ Self | NumAtt$ +1 | AtEOT$ Sacrifice | Duration$ Permanent",
        )
        .err()
        .unwrap_or_else(|| panic!("mixed permanent cleanup must remain quarantined"));
        assert_eq!(error.code, "UNSUPPORTED_PARAMETER");
    }

    #[test]
    fn maps_closed_hidden_combat_restriction_keywords() {
        for (keyword, operation) in [
            ("HIDDEN CARDNAME can't attack.", Operation::CannotAttack),
            ("HIDDEN CARDNAME can't block.", Operation::CannotBlock),
        ] {
            let mapped = map_line(&format!("A:AB$ Pump | Defined$ Self | KW$ {keyword}"))
                .unwrap_or_else(|error| {
                    panic!("closed combat restriction should map: {}", error.message)
                });
            assert!(matches!(
                mapped.expression,
                Expression::Call {
                    operation: actual,
                    ..
                } if actual == operation
            ));
        }

        let mapped =
            map_line("A:AB$ Pump | Defined$ Self | KW$ HIDDEN CARDNAME can't attack or block.")
                .unwrap_or_else(|error| {
                    panic!("combined combat restriction should map: {}", error.message)
                });
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::CannotAttack
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::CannotBlock
        ));
    }

    #[test]
    fn rejects_open_hidden_combat_restriction_keywords() {
        let error = map_line(
            "A:AB$ Pump | Defined$ Self | KW$ HIDDEN All creatures able to block CARDNAME do so.",
        )
        .err()
        .unwrap_or_else(|| panic!("open combat restriction must quarantine"));
        assert_eq!(error.code, "UNSUPPORTED_VALUE");
    }

    #[test]
    fn maps_closed_drawn_event_number() {
        let mapped = map_script_root(concat!(
            "T:Mode$ Drawn | ValidCard$ Card.YouCtrl | Number$ 2 | ",
            "TriggerZones$ Battlefield | Execute$ TrigDraw | TriggerDescription$ Draw.\n",
            "SVar:TrigDraw:DB$ Draw | Defined$ You\n",
        ))
        .unwrap_or_else(|error| panic!("drawn-number trigger should map: {}", error.message));
        assert!(matches!(
            mapped.event,
            Some(Expression::Call {
                operation: Operation::EventDraw,
                arguments,
            }) if matches!(arguments.as_slice(), [_, Expression::Integer(2)])
        ));
    }

    #[test]
    fn rejects_non_positive_drawn_event_number() {
        for number in ["0", "-1", "X"] {
            let error = map_script_root(&format!(
                "T:Mode$ Drawn | ValidCard$ Card.YouCtrl | Number$ {number} | TriggerZones$ Battlefield | Execute$ TrigDraw\nSVar:TrigDraw:DB$ Draw | Defined$ You\n"
            ))
            .err()
            .unwrap_or_else(|| panic!("invalid drawn number must fail closed: {number}"));
            assert_eq!(error.code, "UNSUPPORTED_VALUE");
        }
    }

    #[test]
    fn maps_source_bound_sacrifice_without_sac_valid() {
        let mapped = map_line("A:AB$ Sacrifice | Cost$ 1")
            .unwrap_or_else(|error| panic!("source sacrifice should map: {}", error.message));
        assert_eq!(mapped.costs.len(), 1);
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::SacrificeEffect
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Permanents
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Source
        ));
    }

    #[test]
    fn rejects_ambiguous_sacrifice_without_sac_valid() {
        let error = map_line("A:AB$ Sacrifice | Defined$ You")
            .err()
            .unwrap_or_else(|| panic!("player-bound sacrifice must require SacValid"));
        assert_eq!(error.code, "MISSING_PARAMETER");
    }

    #[test]
    fn maps_closed_cleanup_domains() {
        let mapped = map_script_root(concat!(
            "Name:Cleanup Remembered\n",
            "A:SP$ Draw | Defined$ You | NumCards$ 1 | SubAbility$ DBCleanup\n",
            "SVar:DBCleanup:DB$ Cleanup | ClearRemembered$ True\n",
        ))
        .unwrap_or_else(|error| panic!("clear remembered cleanup should map: {}", error.message));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Draw
        ));
        assert!(expression_contains_operation(
            &mapped.expression,
            Operation::Forget
        ));

        for line in [
            "A:DB$ Cleanup | ClearChosenCard$ True",
            "A:DB$ Cleanup | ClearRemembered$ True | ClearImprinted$ True",
            "A:DB$ Cleanup | ClearChosenPlayer$ True | ClearChosenColor$ True | ClearChosenType$ True",
            "A:DB$ Cleanup | ClearNamedCard$ True | ClearCoinFlips$ True | ClearTriggered$ True",
        ] {
            let mapped = map_line(line)
                .unwrap_or_else(|error| panic!("closed cleanup should map: {}", error.message));
            assert!(expression_contains_operation(
                &mapped.expression,
                Operation::Forget
            ));
        }

        for line in ["A:DB$ Cleanup", "A:DB$ Cleanup | ClearRemembered$ False"] {
            assert!(
                map_line(line).is_err(),
                "open cleanup form must remain quarantined: {line}"
            );
        }
    }

    fn assert_operation(line: &str, operation: Operation, expected_costs: usize) {
        let mapped = map_line(line).unwrap_or_else(|error| {
            panic!("simple mapping should pass: {}", error.message);
        });
        assert_eq!(mapped.costs.len(), expected_costs);
        assert!(matches!(
            mapped.expression,
            Expression::Call {
                operation: actual,
                ..
            } if actual == operation
        ));
    }

    fn map_line(line: &str) -> Result<super::MappedLegacyAbility, super::MappingDiagnostic> {
        let script = parse_legacy_script("fixture.txt", line).unwrap_or_else(|error| {
            panic!("mapping fixture should parse: {error}");
        });
        let Some(first) = script.lines.first() else {
            panic!("mapping fixture is empty");
        };
        let LegacyLineKind::Ability { prefix, expression } = &first.kind else {
            panic!("mapping fixture is not an ability");
        };
        map_legacy_ability(*prefix, expression)
    }

    fn map_script_root(
        script_text: &str,
    ) -> Result<super::MappedLegacyAbility, super::MappingDiagnostic> {
        let script = parse_legacy_script("fixture.txt", script_text).unwrap_or_else(|error| {
            panic!("mapping fixture should parse: {error}");
        });
        let context = MappingContext::from_script(&script);
        let (prefix, expression) = script
            .lines
            .iter()
            .find_map(|line| match &line.kind {
                LegacyLineKind::Ability { prefix, expression } => Some((*prefix, expression)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("mapping fixture has no root ability"));
        map_legacy_ability_in_context(prefix, expression, &context)
    }

    fn expression_contains_operation(expression: &Expression, expected: Operation) -> bool {
        match expression {
            Expression::Call {
                operation,
                arguments,
            } => {
                *operation == expected
                    || arguments
                        .iter()
                        .any(|argument| expression_contains_operation(argument, expected))
            }
            _ => false,
        }
    }

    fn expression_operation_count(expression: &Expression, expected: Operation) -> usize {
        match expression {
            Expression::Call {
                operation,
                arguments,
            } => {
                usize::from(*operation == expected)
                    + arguments
                        .iter()
                        .map(|argument| expression_operation_count(argument, expected))
                        .sum::<usize>()
            }
            Expression::List(values) => values
                .iter()
                .map(|value| expression_operation_count(value, expected))
                .sum(),
            _ => 0,
        }
    }

    fn has_ordered_type_replacement(expression: &Expression) -> bool {
        match expression {
            Expression::Call {
                operation: Operation::Sequence,
                arguments,
            } => {
                matches!(
                    arguments.first(),
                    Some(Expression::Call {
                        operation: Operation::RemoveType,
                        arguments,
                    }) if arguments.get(1)
                        == Some(&Expression::Text("creature_subtypes".to_string()))
                ) && matches!(
                    arguments.get(1),
                    Some(Expression::Call {
                        operation: Operation::AddType,
                        ..
                    })
                )
            }
            Expression::Call { arguments, .. } => {
                arguments.iter().any(has_ordered_type_replacement)
            }
            _ => false,
        }
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("could not run git fixture command: {error}"));
        if !output.status.success() {
            panic!(
                "git fixture command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
