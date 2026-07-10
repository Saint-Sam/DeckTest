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
        api: "Draw",
        mapper: map_draw,
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
        api: "Destroy",
        mapper: map_destroy,
    },
    MapperSpec {
        prefix: LegacyAbilityPrefix::Activated,
        api: "PutCounter",
        mapper: map_put_counter,
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
        api: "Animate",
        mapper: map_animate,
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
];

struct MappingContext<'a> {
    svars: BTreeMap<String, &'a LegacyExpression>,
    duplicate_svars: BTreeSet<String>,
}

impl<'a> MappingContext<'a> {
    fn from_script(script: &'a crate::legacy::LegacyScript) -> Self {
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
        }
    }
}

/// Maps one parsed top-level legacy ability without approximation.
pub fn map_legacy_ability(
    prefix: LegacyAbilityPrefix,
    expression: &LegacyExpression,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
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
    let mut parameters = parameters(expression)?;
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
    mapped.timing = timing;
    Ok(mapped)
}

fn map_legacy_ability_in_context(
    prefix: LegacyAbilityPrefix,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    map_with_context(prefix, expression, context, &mut Vec::new())
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

fn map_with_context(
    prefix: LegacyAbilityPrefix,
    expression: &LegacyExpression,
    context: &MappingContext<'_>,
    stack: &mut Vec<String>,
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
        return map_charm_ability(prefix, selector_key, expression, context, stack);
    }
    if prefix == LegacyAbilityPrefix::Replacement && api == "Moved" {
        return map_moved_replacement(prefix, selector_key, expression, context, stack);
    }
    if prefix == LegacyAbilityPrefix::Triggered {
        return map_triggered_ability(prefix, api, selector_key, expression, context, stack);
    }

    let parameter_map = parameters(expression)?;
    let sub_ability = parameter_map.get("SubAbility").cloned();
    let mut base_expression = expression.clone();
    if sub_ability.is_some() {
        base_expression
            .fields
            .retain(|field| field.key.as_deref() != Some("SubAbility"));
    }
    let mut mapped = map_legacy_ability(prefix, &base_expression)?;
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
    Ok(mapped)
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
            "CharmNum",
            "MinCharmNum",
            "AdditionalDescription",
            "PrecostDesc",
        ],
    )?;
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
    let minimum = optional_positive_integer(&parameters, "MinCharmNum")?.unwrap_or(maximum);
    let expression = if minimum == 1 && maximum == 1 {
        call(Operation::ChooseOne, effects)
    } else if minimum == maximum {
        let mut arguments = vec![Expression::Integer(maximum)];
        arguments.extend(effects);
        call(Operation::ChooseExactly, arguments)
    } else {
        return Err(diagnostic(
            "UNSUPPORTED_VALUE",
            &format!("Charm range {minimum}..={maximum} has no exact lowering"),
        ));
    };
    mapped_direct(prefix, "Charm", &parameters, expression)
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
        _ => {
            return Err(diagnostic(
                "UNMAPPED_API",
                &format!("no linked trigger mapper is registered for T:{api}"),
            ));
        }
    };
    let linked = resolve_svar(execute, context, stack)?;
    if linked.event.is_some() || !linked.costs.is_empty() {
        return Err(diagnostic(
            "UNSUPPORTED_LINK",
            &format!("Execute `{execute}` is not a cost-free effect chain"),
        ));
    }
    let expression = if optional.is_some() {
        call(
            Operation::ChooseUpTo,
            vec![Expression::Integer(1), linked.expression],
        )
    } else {
        linked.expression
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
    if !closed_zone(origin) || !closed_zone(destination) || destination == "Any" {
        return Err(diagnostic(
            "UNSUPPORTED_EVENT",
            &format!("ChangesZone transition `{origin}` -> `{destination}` is not a closed zone"),
        ));
    }
    let affected = zone_event_selector(required(parameters, "ValidCard")?, origin)?;
    Ok(if origin == "Any" && destination == "Battlefield" {
        call(Operation::EventEnters, vec![affected])
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
    if phase != "Upkeep" {
        return Err(unsupported_value("Phase", phase));
    }
    let player = match parameters.get("ValidPlayer").map(String::as_str) {
        None | Some("Any") | Some("Player") => call(Operation::Any, vec![]),
        Some("You") => call(Operation::You, vec![]),
        Some("Opponent") | Some("Player.Opponent") => call(Operation::Opponent, vec![]),
        Some(value) => return Err(unsupported_value("ValidPlayer", value)),
    };
    Ok(call(Operation::EventUpkeep, vec![player]))
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
    let mut arguments = vec![spells];
    if let Some(value) = parameters.get("ValidActivatingPlayer") {
        arguments.push(match value.as_str() {
            "Any" | "Player" => call(Operation::Any, vec![]),
            "You" => call(Operation::You, vec![]),
            "Opponent" | "Player.Opponent" => call(Operation::Opponent, vec![]),
            _ => return Err(unsupported_value("ValidActivatingPlayer", value)),
        });
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
    Ok(call(Operation::EventDraw, vec![drawer]))
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
        &["Cost", "Produced", "Amount", "SpellDescription"],
    )?;
    let produced = required(parameters, "Produced")?;
    let amount = optional_positive_integer(parameters, "Amount")?.unwrap_or(1);
    let mana = normalize_mana(produced, amount)?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::AddMana,
            vec![Expression::Text(mana), call(Operation::You, vec![])],
        ),
    })
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
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    if parameters
        .get("Defined")
        .is_some_and(|value| value != "You")
    {
        return Err(unsupported_value(
            "Defined",
            required(parameters, "Defined")?,
        ));
    }
    let amount = optional_positive_integer(parameters, "NumCards")?.unwrap_or(1);
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::Draw,
            vec![Expression::Integer(amount), call(Operation::You, vec![])],
        ),
    })
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
            "ValidTgts",
            "NumDmg",
            "SpellDescription",
            "TgtPrompt",
        ],
    )?;
    let targets = required(parameters, "ValidTgts")?;
    let amount = positive_integer(required(parameters, "NumDmg")?, "NumDmg")?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::DealDamage,
            vec![valid_target_selector(targets)?, Expression::Integer(amount)],
        ),
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
            "TgtPrompt",
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
    require_end_of_turn_duration(parameters)?;
    let affected = object_selector(parameters, DefaultSelector::Source)?;
    let mut effects = Vec::new();
    if parameters.contains_key("NumAtt") || parameters.contains_key("NumDef") {
        let power = optional_signed_integer(parameters, "NumAtt")?.unwrap_or(0);
        let toughness = optional_signed_integer(parameters, "NumDef")?.unwrap_or(0);
        effects.push(call(
            Operation::ModifyPt,
            vec![
                affected.clone(),
                Expression::Integer(power),
                Expression::Integer(toughness),
                Expression::Text("until_end_of_turn".to_string()),
            ],
        ));
    }
    append_keyword_grants(&mut effects, &affected, parameters.get("KW"))?;
    let expression = combine_effects(effects, "simple Pump requires a PT or keyword modifier")?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression,
    })
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
    require_end_of_turn_duration(parameters)?;
    let affected = affected_selector(required(parameters, "ValidCards")?)?;
    let mut effects = Vec::new();
    if parameters.contains_key("NumAtt") || parameters.contains_key("NumDef") {
        let power = optional_signed_integer(parameters, "NumAtt")?.unwrap_or(0);
        let toughness = optional_signed_integer(parameters, "NumDef")?.unwrap_or(0);
        effects.push(call(
            Operation::ModifyPt,
            vec![
                affected.clone(),
                Expression::Integer(power),
                Expression::Integer(toughness),
                Expression::Text("until_end_of_turn".to_string()),
            ],
        ));
    }
    append_keyword_grants(&mut effects, &affected, parameters.get("KW"))?;
    let expression = combine_effects(effects, "simple PumpAll requires a PT or keyword modifier")?;
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
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(Operation::Mill, vec![Expression::Integer(amount), affected]),
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
    map_object_effect(prefix, api, selector, parameters, Operation::Untap)
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
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(operation, arguments),
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
            "TgtPrompt",
            "CounterType",
            "CounterNum",
            "SpellDescription",
            "StackDescription",
            "IsCurse",
            "AILogic",
        ],
    )?;
    let counter_type = required(parameters, "CounterType")?;
    if counter_type.trim().is_empty() {
        return Err(unsupported_value("CounterType", counter_type));
    }
    let amount = optional_positive_integer(parameters, "CounterNum")?.unwrap_or(1);
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        event: None,
        timing: None,
        expression: call(
            Operation::AddCounter,
            vec![
                object_selector(parameters, DefaultSelector::Source)?,
                Expression::Text(counter_type.to_ascii_lowercase()),
                Expression::Integer(amount),
            ],
        ),
    })
}

fn map_continuous(
    prefix: LegacyAbilityPrefix,
    api: &str,
    selector: &str,
    parameters: &BTreeMap<String, String>,
) -> Result<MappedLegacyAbility, MappingDiagnostic> {
    require_selector(selector, "Mode")?;
    reject_unknown(
        parameters,
        &[
            "Affected",
            "AddPower",
            "AddToughness",
            "AddKeyword",
            "SetPower",
            "SetToughness",
            "AddType",
            "RemoveType",
            "SetColor",
            "RemoveAllAbilities",
            "GainControl",
            "SetMaxHandSize",
            "AffectedZone",
            "EffectZone",
            "Description",
        ],
    )?;
    require_battlefield_zone(parameters, "AffectedZone")?;
    require_battlefield_zone(parameters, "EffectZone")?;
    let affected = affected_selector(required(parameters, "Affected")?)?;
    let affected_player = required(parameters, "Affected")? == "You";
    let mut effects = Vec::new();
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
            effects.push(call(
                Operation::GrantKeyword,
                vec![
                    call(Operation::Any, vec![]),
                    Expression::Text(normalize_simple_keyword(keyword)?),
                ],
            ));
        }
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
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: Vec::new(),
        event: None,
        timing: None,
        expression: call(Operation::Continuous, vec![affected, effect]),
    })
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
            "ValidTgts",
            "TgtPrompt",
            "Origin",
            "Destination",
            "SpellDescription",
            "StackDescription",
            "IsCurse",
            "AILogic",
        ],
    )?;
    let origin = required(parameters, "Origin")?;
    let replacement_object = parameters
        .get("Defined")
        .is_some_and(|value| value == "ReplacedCard");
    let identity_bound = parameters
        .get("Defined")
        .is_some_and(|value| matches!(value.as_str(), "Self" | "TriggeredCard" | "ReplacedCard"))
        && !parameters.contains_key("ValidTgts");
    let closed_origin = matches!(origin, "Graveyard" | "Hand" | "Exile" | "Stack");
    if origin != "Battlefield"
        && !(origin == "All" && replacement_object)
        && !(closed_origin && identity_bound)
    {
        return Err(unsupported_value("Origin", origin));
    }
    let affected = object_selector(parameters, DefaultSelector::Source)?;
    let expression = match required(parameters, "Destination")? {
        "Graveyard" => call(
            Operation::MoveZone,
            vec![affected, Expression::Text("graveyard".to_string())],
        ),
        "Exile" => call(Operation::Exile, vec![affected]),
        "Hand" => call(Operation::ReturnToHand, vec![affected]),
        "Battlefield" => call(
            Operation::MoveZone,
            vec![affected, Expression::Text("battlefield".to_string())],
        ),
        value => return Err(unsupported_value("Destination", value)),
    };
    mapped_direct(prefix, api, parameters, expression)
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
            "TokenAmount",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let token = required(parameters, "TokenScript")?;
    if token.is_empty() || token.contains(',') {
        return Err(unsupported_value("TokenScript", token));
    }
    let amount = optional_positive_integer(parameters, "TokenAmount")?.unwrap_or(1);
    let owner = parameters
        .get("TokenOwner")
        .map(|value| defined_player_selector(value))
        .transpose()?
        .unwrap_or_else(|| call(Operation::You, vec![]));
    let expression = call(
        Operation::CreateToken,
        vec![
            Expression::Text(token.to_string()),
            Expression::Integer(amount),
            owner,
        ],
    );
    mapped_direct(prefix, api, parameters, expression)
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
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let expression = call(
        Operation::Destroy,
        vec![valid_cards_selector(required(parameters, "ValidCards")?)?],
    );
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
            "ValidDescription",
            "NumDmg",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let mut affected = Vec::new();
    if let Some(cards) = parameters.get("ValidCards") {
        affected.push(valid_cards_selector(cards)?);
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
    let expression = call(
        Operation::DealDamage,
        vec![target, Expression::Integer(amount)],
    );
    mapped_direct(prefix, api, parameters, expression)
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
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let amount = positive_integer(required(parameters, amount_key)?, amount_key)?;
    let expression = call(
        operation,
        vec![Expression::Integer(amount), call(Operation::You, vec![])],
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
            "Activator",
            "Amount",
            "EffectZone",
            "Description",
        ],
    )?;
    if required(parameters, "Type")? != "Spell" {
        return Err(unsupported_value("Type", required(parameters, "Type")?));
    }
    require_battlefield_zone(parameters, "EffectZone")?;
    let amount = positive_integer(required(parameters, "Amount")?, "Amount")?;
    let mut spells = parameters
        .get("ValidCard")
        .map(|value| spell_selector(value))
        .transpose()?
        .unwrap_or_else(|| call(Operation::Spells, vec![]));
    if let Some(activator) = parameters.get("Activator") {
        let player = match activator.as_str() {
            "You" => call(Operation::You, vec![]),
            "Opponent" | "Player.Opponent" => call(Operation::Opponent, vec![]),
            "Player" | "Any" => call(Operation::Any, vec![]),
            _ => return Err(unsupported_value("Activator", activator)),
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
                    Operation::CostReduction,
                    vec![call(Operation::Any, vec![]), Expression::Integer(amount)],
                ),
            ],
        ),
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
        &["ValidAttacker", "Description", "Secondary", "EffectZone"],
    )?;
    require_battlefield_zone(parameters, "EffectZone")?;
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
    let blockers = call(
        Operation::Permanents,
        vec![call(
            Operation::TypeIs,
            vec![Expression::Text("creature".to_string())],
        )],
    );
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
            "SpellDescription",
            "StackDescription",
            "ValidDescription",
            "AILogic",
            "IsCurse",
        ],
    )?;
    let origin = required(parameters, "Origin")?;
    if origin != "Battlefield" {
        return Err(unsupported_value("Origin", origin));
    }
    let affected = valid_cards_selector(required(parameters, "ChangeType")?)?;
    let expression = match required(parameters, "Destination")? {
        "Graveyard" => call(
            Operation::MoveZone,
            vec![affected, Expression::Text("graveyard".to_string())],
        ),
        "Exile" => call(Operation::Exile, vec![affected]),
        "Hand" => call(Operation::ReturnToHand, vec![affected]),
        value => return Err(unsupported_value("Destination", value)),
    };
    mapped_direct(prefix, api, parameters, expression)
}

fn map_animate(
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
            "Power",
            "Toughness",
            "Types",
            "Colors",
            "OverwriteColors",
            "Keywords",
            "RemoveAllAbilities",
            "Duration",
        ],
    )?;
    let affected = object_selector(parameters, DefaultSelector::Source)?;
    let mut effects = Vec::new();
    if parameters.contains_key("Power") || parameters.contains_key("Toughness") {
        let power = optional_number_or_value(parameters, "Power", Operation::Power)?;
        let toughness = optional_number_or_value(parameters, "Toughness", Operation::Toughness)?;
        effects.push(call(
            Operation::SetPt,
            vec![affected.clone(), power, toughness],
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
    let mut expression = combine_effects(effects, "simple Animate has no typed changes")?;
    match parameters.get("Duration").map(String::as_str) {
        None | Some("EndOfTurn") => {
            expression = call(Operation::UntilEndOfTurn, vec![expression]);
        }
        Some("Permanent") => {}
        Some(value) => return Err(unsupported_value("Duration", value)),
    }
    mapped_direct(prefix, api, parameters, expression)
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
    let permanents = affected_selector(required(parameters, "SacValid")?)?;
    let permanents =
        add_collection_predicate(permanents, call(Operation::ControlledBy, vec![player]))?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(Operation::SacrificeEffect, vec![permanents]),
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
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::ChangeControl,
            vec![affected, call(Operation::You, vec![])],
        ),
    )
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
            "CounterType",
            "CounterNum",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
    let amount = optional_positive_integer(parameters, "CounterNum")?.unwrap_or(1);
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::AddCounter,
            vec![
                affected_selector(required(parameters, "ValidCards")?)?,
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
    let target = call(
        Operation::Target,
        vec![spell_selector(required(parameters, "ValidTgts")?)?],
    );
    mapped_direct(prefix, api, parameters, call(Operation::Copy, vec![target]))
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
        &["Cost", "ValidCards", "SpellDescription", "StackDescription"],
    )?;
    mapped_direct(
        prefix,
        api,
        parameters,
        call(
            Operation::Untap,
            vec![affected_selector(required(parameters, "ValidCards")?)?],
        ),
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
        &["ValidCard", "EffectZone", "Description", "Secondary"],
    )?;
    require_battlefield_zone(parameters, "EffectZone")?;
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
    let restriction = combine_effects(
        restrictions
            .into_iter()
            .map(|operation| call(operation, vec![call(Operation::Any, vec![])]))
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
    require_battlefield_zone(parameters, "EffectZone")?;
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
        &["Cost", "Defined", "SpellDescription", "StackDescription"],
    )?;
    if parameters
        .get("Defined")
        .is_some_and(|player| player != "You")
    {
        return Err(unsupported_value(
            "Defined",
            required(parameters, "Defined")?,
        ));
    }
    let operation = match api {
        "Venture" => Operation::Venture,
        "BecomeMonarch" => Operation::BecomeMonarch,
        "TakeInitiative" => Operation::TakeInitiative,
        _ => return Err(diagnostic("UNMAPPED_API", "unknown owner marker effect")),
    };
    mapped_direct(prefix, api, parameters, call(operation, vec![]))
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
            "Num",
            "SpellDescription",
            "StackDescription",
        ],
    )?;
    if parameters
        .get("Defined")
        .is_some_and(|player| player != "You")
    {
        return Err(unsupported_value(
            "Defined",
            required(parameters, "Defined")?,
        ));
    }
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
                call(Operation::You, vec![]),
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
            "TgtPrompt",
            "SpellDescription",
            "StackDescription",
            "AILogic",
        ],
    )?;
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
    require_selector_one_of(selector, &["AB", "SP", "DB"])?;
    reject_unknown(
        parameters,
        &[
            "Cost",
            "ValidCards",
            "Power",
            "Toughness",
            "Types",
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
        ],
    )?;
    let affected = affected_selector(required(parameters, "ValidCards")?)?;
    let mut effects = Vec::new();
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
        effects.push(call(Operation::RemoveAllAbilities, vec![affected]));
    }
    let mut expression = combine_effects(effects, "simple AnimateAll has no typed changes")?;
    match parameters.get("Duration").map(String::as_str) {
        None | Some("EndOfTurn") | Some("UntilEndOfTurn") => {
            expression = call(Operation::UntilEndOfTurn, vec![expression]);
        }
        Some("Permanent") => {}
        Some(value) => return Err(unsupported_value("Duration", value)),
    }
    mapped_direct(prefix, api, parameters, expression)
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
    if once && (sorcery || your_turn) {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            "combined activation limit and phase timing has no closed timing conjunction",
        ));
    }
    Ok(if once {
        Some(call(Operation::TimingOnceEachTurn, vec![]))
    } else if sorcery {
        Some(call(Operation::TimingSorcery, vec![]))
    } else if your_turn {
        Some(call(Operation::TimingYourTurn, vec![]))
    } else {
        None
    })
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

fn parse_simple_cost(value: Option<&String>) -> Result<Vec<Expression>, MappingDiagnostic> {
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
                    call(Operation::ControlledBy, vec![call(Operation::You, vec![])]),
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
        return valid_target_selector(value);
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
        return Ok(call(
            Operation::Target,
            vec![draw_player_selector(value, "ValidTgts")?],
        ));
    }
    parameters
        .get("Defined")
        .map(|value| defined_player_selector(value))
        .unwrap_or_else(|| Ok(default_selector(default)))
}

fn default_selector(default: DefaultSelector) -> Expression {
    match default {
        DefaultSelector::Source => call(Operation::Source, vec![]),
        DefaultSelector::You => call(Operation::You, vec![]),
    }
}

fn defined_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    match value {
        "Self" => Ok(call(Operation::Source, vec![])),
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
        "TriggeredCardController" => Ok(call(
            Operation::ControllerOf,
            vec![call(Operation::Triggered, vec![])],
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
        "Targeted" => Ok(call(Operation::Target, vec![call(Operation::Any, vec![])])),
        "TargetedController" => Ok(call(
            Operation::ControllerOf,
            vec![call(Operation::Target, vec![call(Operation::Any, vec![])])],
        )),
        "TriggeredCardController" => Ok(call(
            Operation::ControllerOf,
            vec![call(Operation::Triggered, vec![])],
        )),
        _ => Err(unsupported_value("Defined", value)),
    }
}

fn valid_target_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
    if value == "Any" {
        return Ok(call(Operation::Target, vec![call(Operation::Any, vec![])]));
    }
    Ok(call(
        Operation::Target,
        vec![affected_selector(value).map_err(|_| unsupported_value("ValidTgts", value))?],
    ))
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
    let modifiers = pieces.collect::<Vec<_>>();
    if base.is_empty() || modifiers.iter().any(|part| part.contains('+')) {
        return Err(unsupported_value("ValidCard", value));
    }
    let mut predicates = Vec::new();
    if base == "Card" {
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

fn affected_selector(value: &str) -> Result<Expression, MappingDiagnostic> {
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
                "Legendary" => call(
                    Operation::SupertypeIs,
                    vec![Expression::Text("legendary".to_string())],
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

fn append_keyword_grants(
    effects: &mut Vec<Expression>,
    affected: &Expression,
    keywords: Option<&String>,
) -> Result<(), MappingDiagnostic> {
    let Some(keywords) = keywords else {
        return Ok(());
    };
    for keyword in keywords.split(" & ") {
        effects.push(call(
            Operation::GrantKeyword,
            vec![
                affected.clone(),
                Expression::Text(normalize_simple_keyword(keyword)?),
                Expression::Text("until_end_of_turn".to_string()),
            ],
        ));
    }
    Ok(())
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

fn require_end_of_turn_duration(
    parameters: &BTreeMap<String, String>,
) -> Result<(), MappingDiagnostic> {
    if parameters
        .get("Duration")
        .map_or(true, |duration| duration == "UntilEndOfTurn")
    {
        Ok(())
    } else {
        Err(unsupported_value(
            "Duration",
            required(parameters, "Duration")?,
        ))
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
    matches!(
        key,
        "AILogic"
            | "AITgts"
            | "CostDesc"
            | "IsCurse"
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
        audit_legacy_mappings, map_legacy_ability, map_legacy_ability_in_context, MappingContext,
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
    }

    #[test]
    fn maps_simple_continuous_effects() {
        for line in [
            "S:Mode$ Continuous | Affected$ Card.Self | AddPower$ 2 | AddToughness$ 1 | Description$ Self gets +2/+1.",
            "S:Mode$ Continuous | Affected$ Creature.YouCtrl+Other | AddPower$ 1 | AddToughness$ 1 | Description$ Other creatures get +1/+1.",
            "S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddKeyword$ First Strike | Description$ Equipped creature has first strike.",
            "S:Mode$ Continuous | Affected$ Spirit.YouCtrl | AddPower$ 1 | AddKeyword$ Flying & Vigilance | Description$ Spirits get +1/+0 and keywords.",
            "S:Mode$ ReduceCost | ValidCard$ Instant,Sorcery | Type$ Spell | Activator$ You | Amount$ 1 | Description$ Reduce costs.",
            "S:Mode$ CantBlockBy | ValidAttacker$ Creature.Self | Description$ This creature can't be blocked.",
            "S:Mode$ Continuous | Affected$ Card.Self | SetPower$ 4 | SetToughness$ 5 | AddType$ Creature | SetColor$ Blue | Description$ Becomes a creature.",
            "S:Mode$ Continuous | Affected$ Creature.YouCtrl | RemoveAllAbilities$ True | Description$ Remove abilities.",
            "S:Mode$ Continuous | Affected$ Creature.EnchantedBy | GainControl$ You | Description$ Gain control.",
            "S:Mode$ Continuous | Affected$ You | SetMaxHandSize$ Unlimited | Description$ No maximum hand size.",
        ] {
            assert_operation(line, Operation::Continuous, 0);
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
        ] {
            map_line(line).unwrap_or_else(|error| {
                panic!("closed selector should map: {}", error.message);
            });
        }
        let error = match map_line(
            "A:AB$ Pump | ValidTgts$ Creature.attacking | NumAtt$ 1 | NumDef$ 1 | SpellDescription$ Pump.",
        ) {
            Ok(_) => panic!("combat-state selector must remain quarantined"),
            Err(error) => error,
        };
        assert_eq!(error.code, "UNSUPPORTED_VALUE");
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
            assert!(matches!(
                mapped.expression,
                Expression::Call { operation, .. } if operation == expected_effect
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
                "A:SP$ Token | TokenScript$ g_1_1_saproling | TokenOwner$ You | TokenAmount$ 2 | SpellDescription$ Tokens.",
                Operation::CreateToken,
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
                "A:AB$ Animate | Defined$ Self | Power$ 3 | Toughness$ 3 | Types$ Creature,Elemental | Colors$ Blue | OverwriteColors$ True | Keywords$ Flying | SpellDescription$ Animate.",
                Operation::UntilEndOfTurn,
            ),
        ] {
            assert_operation(line, operation, 0);
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

        let conditioned_continuous = map_line(
            "S:Mode$ Continuous | Affected$ Card.Self | AddKeyword$ Flying | Condition$ PlayerTurn | Description$ Conditional.",
        )
        .err()
        .unwrap_or_else(|| panic!("conditioned continuous effect must quarantine"));
        assert_eq!(conditioned_continuous.code, "UNSUPPORTED_PARAMETER");
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
        assert_eq!(report.mapped_uses, 3);
        assert_eq!(report.verified_uses, 3);
        assert_eq!(report.quarantined_uses, 2);
        assert_eq!(
            report.quarantine_reason_counts.get("MISSING_SVAR"),
            Some(&1)
        );
        assert_eq!(
            report.quarantine_reason_counts.get("UNSUPPORTED_PARAMETER"),
            Some(&1)
        );
        assert!(metrics.is_file());
        assert!(quarantine.is_file());

        fs::remove_dir_all(&root).unwrap_or_else(|error| {
            panic!("could not remove mapper fixture: {error}");
        });
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
