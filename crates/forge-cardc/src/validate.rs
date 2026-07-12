use crate::error::{CardcError, CardcResult};
use forge_carddef::{Expression, OperationCategory};

pub(crate) const KNOWN_KEYWORDS: &[&str] = &[
    "affinity",
    "afterlife",
    "annihilator",
    "ascend",
    "assist",
    "aura_swap",
    "awaken",
    "backup",
    "banding",
    "battle_cry",
    "bestow",
    "blitz",
    "bloodthirst",
    "boast",
    "bushido",
    "buyback",
    "cascade",
    "casualty",
    "champion",
    "changeling",
    "cipher",
    "cleave",
    "companion",
    "compleated",
    "convoke",
    "councils_dilemma",
    "crew",
    "cumulative_upkeep",
    "cycling",
    "dash",
    "daybound",
    "deathtouch",
    "decayed",
    "defender",
    "delve",
    "demonstrate",
    "dethrone",
    "devoid",
    "devour",
    "disturb",
    "double_strike",
    "dredge",
    "echo",
    "embalm",
    "emerge",
    "enchant",
    "encore",
    "enlist",
    "entwine",
    "epic",
    "equip",
    "escalate",
    "escape",
    "eternalize",
    "evoke",
    "evolve",
    "exalted",
    "exploit",
    "extort",
    "fabricate",
    "fading",
    "fear",
    "first_strike",
    "flanking",
    "flash",
    "flashback",
    "flying",
    "for_mirrodin",
    "foretell",
    "forestwalk",
    "fortify",
    "frenzy",
    "fuse",
    "graft",
    "gravestorm",
    "haste",
    "haunt",
    "hexproof",
    "hidden_agenda",
    "hideaway",
    "horsemanship",
    "improvise",
    "indestructible",
    "infect",
    "ingest",
    "intimidate",
    "jump_start",
    "kicker",
    "landfall",
    "landwalk",
    "islandwalk",
    "mountainwalk",
    "plainswalk",
    "swampwalk",
    "level_up",
    "lifelink",
    "living_weapon",
    "madness",
    "melee",
    "menace",
    "mentor",
    "miracle",
    "modular",
    "meld",
    "morph",
    "mutate",
    "myriad",
    "nightbound",
    "ninjutsu",
    "offering",
    "outlast",
    "overload",
    "partner",
    "persist",
    "phasing",
    "poisonous",
    "protection",
    "prototype",
    "provoke",
    "prowess",
    "rampage",
    "ravenous",
    "reach",
    "read_ahead",
    "rebound",
    "reconfigure",
    "recover",
    "reinforce",
    "renown",
    "replicate",
    "retrace",
    "riot",
    "ripple",
    "saddle",
    "scavenge",
    "shadow",
    "shroud",
    "skulk",
    "solved",
    "soulbond",
    "soulshift",
    "space_sculptor",
    "spectacle",
    "splice",
    "split_second",
    "storm",
    "sunburst",
    "surge",
    "suspend",
    "toxic",
    "training",
    "trample",
    "transfigure",
    "transform",
    "transmute",
    "tribute",
    "typecycling",
    "undaunted",
    "undying",
    "unearth",
    "unleash",
    "vanishing",
    "venture_into_the_dungeon",
    "vigilance",
    "ward",
    "will_of_the_council",
    "wither",
];

pub(crate) fn validate_keyword(
    path: &str,
    line: usize,
    column: usize,
    keyword: &str,
) -> CardcResult<()> {
    if KNOWN_KEYWORDS.contains(&keyword) {
        Ok(())
    } else {
        Err(CardcError::new(
            path,
            line,
            column,
            format!("unknown keyword `{keyword}`"),
        ))
    }
}

pub(crate) fn validate_expression(
    path: &str,
    line: usize,
    column: usize,
    expression: &Expression,
    expected: Option<OperationCategory>,
) -> CardcResult<()> {
    let Expression::Call {
        operation,
        arguments,
    } = expression
    else {
        let message = match expression {
            Expression::Symbol(symbol) => format!(
                "bare symbol `{symbol}` is not allowed in an ability expression; use a closed operation or quoted literal"
            ),
            Expression::List(_) => {
                "expression lists are not valid operation arguments; use a closed variadic operation"
                    .to_string()
            }
            _ if expected.is_some() => "field requires a typed operation call".to_string(),
            _ => return Ok(()),
        };
        return Err(CardcError::new(path, line, column, message));
    };

    if let Some(expected) = expected {
        if operation.category() != expected {
            return Err(CardcError::new(
                path,
                line,
                column,
                format!(
                    "operation `{}` has category {:?}, expected {:?}",
                    operation.as_str(),
                    operation.category(),
                    expected
                ),
            ));
        }
    }
    if arguments.len() < operation.min_args()
        || operation
            .max_args()
            .is_some_and(|maximum| arguments.len() > maximum)
    {
        return Err(CardcError::new(
            path,
            line,
            column,
            format!(
                "operation `{}` received {} argument(s); expected {}..{}",
                operation.as_str(),
                arguments.len(),
                operation.min_args(),
                operation
                    .max_args()
                    .map_or_else(|| "many".to_string(), |value| value.to_string())
            ),
        ));
    }
    for (index, argument) in arguments.iter().enumerate() {
        let Some(kind) = operation.argument_kind(index) else {
            return Err(CardcError::new(
                path,
                line,
                column,
                format!(
                    "operation `{}` has no typed signature for argument {}",
                    operation.as_str(),
                    index + 1
                ),
            ));
        };
        if !kind.accepts(argument) {
            return Err(CardcError::new(
                path,
                line,
                column,
                format!(
                    "operation `{}` argument {} requires {}, received {}",
                    operation.as_str(),
                    index + 1,
                    kind.as_str(),
                    expression_kind(argument)
                ),
            ));
        }
        validate_expression(path, line, column, argument, None)?;
    }
    Ok(())
}

fn expression_kind(expression: &Expression) -> &'static str {
    match expression {
        Expression::Integer(_) => "integer",
        Expression::Boolean(_) => "boolean",
        Expression::Text(_) => "text",
        Expression::Symbol(_) => "bare symbol",
        Expression::List(_) => "list",
        Expression::Call { operation, .. } => match operation.category() {
            OperationCategory::Selector => "selector",
            OperationCategory::Predicate => "predicate",
            OperationCategory::Cost => "cost",
            OperationCategory::Event => "event",
            OperationCategory::Effect => "effect",
            OperationCategory::Timing => "timing",
            OperationCategory::Value => "value",
        },
    }
}
