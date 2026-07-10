use crate::{
    error::{CardcError, CardcResult},
    validate::{validate_expression, validate_keyword},
};
use forge_carddef::{
    AbilityDefinition, AbilityKind, CardClassification, CardDefinition, CardFace, CardLayout,
    CardType, Color, Expression, KeywordId, ManaCost, ManaSymbol, Operation, OperationCategory,
    OracleId, Supertype, TypeLine,
};
use pest::{error::LineColLocation, iterators::Pair, Parser};
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "card_dsl.pest"]
struct CardParser;

/// Parses one in-memory `.frs` source.
pub fn parse_card(source: &str) -> CardcResult<CardDefinition> {
    parse_card_named("<memory>", source)
}

/// Parses one named `.frs` source and includes the name in diagnostics.
pub fn parse_card_named(path: &str, source: &str) -> CardcResult<CardDefinition> {
    let mut parsed = CardParser::parse(Rule::file, source).map_err(|error| {
        let (line, column) = match error.line_col {
            LineColLocation::Pos(position) => position,
            LineColLocation::Span(start, _end) => start,
        };
        CardcError::new(path, line, column, error.variant.message())
    })?;
    let file = parsed
        .next()
        .ok_or_else(|| CardcError::new(path, 1, 1, "parser produced no file"))?;
    let card = file
        .into_inner()
        .find(|pair| pair.as_rule() == Rule::card)
        .ok_or_else(|| CardcError::new(path, 1, 1, "source contains no card"))?;
    parse_card_pair(path, card)
}

fn parse_card_pair(path: &str, pair: Pair<'_, Rule>) -> CardcResult<CardDefinition> {
    let (card_line, card_column) = location(&pair);
    let mut items = pair.into_inner();
    let name_pair = next_required(
        path,
        &mut items,
        card_line,
        card_column,
        "missing card name",
    )?;
    let name = parse_string(path, &name_pair)?;
    let mut id = None;
    let mut layout = None;
    let mut status = None;
    let mut faces = Vec::new();

    for item in items {
        match item.as_rule() {
            Rule::id_field => {
                let value = single_inner(path, &item, "id")?;
                let text = parse_string(path, &value)?;
                let parsed_id =
                    OracleId::parse(text).ok_or_else(|| at(path, &value, "invalid id"))?;
                set_once(path, &item, "id", &mut id, parsed_id)?;
            }
            Rule::layout_field => {
                let value = single_inner(path, &item, "layout")?;
                let parsed_layout = CardLayout::parse(value.as_str()).ok_or_else(|| {
                    at(path, &value, format!("unknown layout `{}`", value.as_str()))
                })?;
                set_once(path, &item, "layout", &mut layout, parsed_layout)?;
            }
            Rule::status_field => {
                let value = single_inner(path, &item, "status")?;
                let classification = parse_definition_status(path, &value)?;
                set_once(path, &item, "status", &mut status, classification)?;
            }
            Rule::face => faces.push(parse_face(path, item)?),
            _ => return Err(at(path, &item, "unexpected card syntax")),
        }
    }

    let id = id.ok_or_else(|| CardcError::new(path, card_line, card_column, "missing `id:`"))?;
    let layout =
        layout.ok_or_else(|| CardcError::new(path, card_line, card_column, "missing `layout:`"))?;
    let status =
        status.ok_or_else(|| CardcError::new(path, card_line, card_column, "missing `status:`"))?;
    if name.is_empty() {
        return Err(CardcError::new(
            path,
            card_line,
            card_column,
            "card name is empty",
        ));
    }
    if faces.is_empty() {
        return Err(CardcError::new(
            path,
            card_line,
            card_column,
            "card has no faces",
        ));
    }
    validate_face_count(path, card_line, card_column, layout, faces.len())?;

    Ok(CardDefinition {
        id,
        name,
        layout,
        status,
        faces,
    })
}

fn parse_face(path: &str, pair: Pair<'_, Rule>) -> CardcResult<CardFace> {
    let (face_line, face_column) = location(&pair);
    let mut items = pair.into_inner();
    let name_pair = next_required(
        path,
        &mut items,
        face_line,
        face_column,
        "missing face name",
    )?;
    let name = parse_string(path, &name_pair)?;
    let mut cost = None;
    let mut type_line = None;
    let mut oracle_text = None;
    let mut power = None;
    let mut toughness = None;
    let mut loyalty = None;
    let mut defense = None;
    let mut keywords = None;
    let mut abilities = Vec::new();

    for item in items {
        match item.as_rule() {
            Rule::cost_field => {
                let value = single_inner(path, &item, "cost")?;
                let text = parse_string(path, &value)?;
                let parsed_cost = parse_mana_cost(path, &value, &text)?;
                set_once(path, &item, "cost", &mut cost, parsed_cost)?;
            }
            Rule::types_field => {
                let value = single_inner(path, &item, "types")?;
                let text = parse_string(path, &value)?;
                let parsed_type = parse_type_line(path, &value, &text)?;
                set_once(path, &item, "types", &mut type_line, parsed_type)?;
            }
            Rule::oracle_field => {
                let value = single_inner(path, &item, "oracle")?;
                let text = parse_string(path, &value)?;
                set_once(path, &item, "oracle", &mut oracle_text, text)?;
            }
            Rule::power_field => parse_stat_field(path, item, "power", &mut power)?,
            Rule::toughness_field => parse_stat_field(path, item, "toughness", &mut toughness)?,
            Rule::loyalty_field => parse_stat_field(path, item, "loyalty", &mut loyalty)?,
            Rule::defense_field => parse_stat_field(path, item, "defense", &mut defense)?,
            Rule::keywords_field => {
                let list = single_inner(path, &item, "keywords")?;
                let parsed_keywords = parse_keywords(path, list)?;
                set_once(path, &item, "keywords", &mut keywords, parsed_keywords)?;
            }
            Rule::ability => abilities.push(parse_ability(path, item)?),
            _ => return Err(at(path, &item, "unexpected face syntax")),
        }
    }

    let mana_cost =
        cost.ok_or_else(|| CardcError::new(path, face_line, face_column, "missing `cost:`"))?;
    let type_line = type_line
        .ok_or_else(|| CardcError::new(path, face_line, face_column, "missing `types:`"))?;
    let oracle_text = oracle_text
        .ok_or_else(|| CardcError::new(path, face_line, face_column, "missing `oracle:`"))?;
    if name.is_empty() {
        return Err(CardcError::new(
            path,
            face_line,
            face_column,
            "face name is empty",
        ));
    }
    if type_line.card_types.contains(&CardType::Creature)
        && (power.is_none() || toughness.is_none())
    {
        return Err(CardcError::new(
            path,
            face_line,
            face_column,
            "creature face requires both `power:` and `toughness:`",
        ));
    }
    if type_line.card_types.contains(&CardType::Planeswalker) && loyalty.is_none() {
        return Err(CardcError::new(
            path,
            face_line,
            face_column,
            "planeswalker face requires `loyalty:`",
        ));
    }
    if type_line.card_types.contains(&CardType::Battle) && defense.is_none() {
        return Err(CardcError::new(
            path,
            face_line,
            face_column,
            "battle face requires `defense:`",
        ));
    }

    Ok(CardFace {
        name,
        mana_cost,
        type_line,
        oracle_text,
        power,
        toughness,
        loyalty,
        defense,
        keywords: keywords.unwrap_or_default(),
        abilities,
    })
}

fn parse_ability(path: &str, pair: Pair<'_, Rule>) -> CardcResult<AbilityDefinition> {
    let (line, column) = location(&pair);
    let mut items = pair.into_inner();
    let kind_pair = next_required(path, &mut items, line, column, "missing ability kind")?;
    let kind = AbilityKind::parse(kind_pair.as_str()).ok_or_else(|| {
        at(
            path,
            &kind_pair,
            format!("unknown ability kind `{}`", kind_pair.as_str()),
        )
    })?;
    let mut costs = None;
    let mut event = None;
    let mut condition = None;
    let mut timing = None;
    let mut effect = None;
    let mut mana_ability = None;

    for item in items {
        match item.as_rule() {
            Rule::costs_field => {
                let list = single_inner(path, &item, "costs")?;
                let expressions = parse_expression_list(path, list)?;
                for expression in &expressions {
                    validate_expression(
                        path,
                        line,
                        column,
                        expression,
                        Some(OperationCategory::Cost),
                    )?;
                }
                set_once(path, &item, "costs", &mut costs, expressions)?;
            }
            Rule::event_field => parse_typed_expression_field(
                path,
                item,
                "event",
                OperationCategory::Event,
                &mut event,
            )?,
            Rule::condition_field => parse_typed_expression_field(
                path,
                item,
                "condition",
                OperationCategory::Predicate,
                &mut condition,
            )?,
            Rule::timing_field => parse_typed_expression_field(
                path,
                item,
                "timing",
                OperationCategory::Timing,
                &mut timing,
            )?,
            Rule::effect_field => parse_typed_expression_field(
                path,
                item,
                "effect",
                OperationCategory::Effect,
                &mut effect,
            )?,
            Rule::mana_ability_field => {
                let value = single_inner(path, &item, "mana_ability")?;
                let parsed = value.as_str() == "true";
                set_once(path, &item, "mana_ability", &mut mana_ability, parsed)?;
            }
            _ => return Err(at(path, &item, "unexpected ability syntax")),
        }
    }

    let costs = costs.unwrap_or_default();
    let effect = effect
        .ok_or_else(|| CardcError::new(path, line, column, "ability is missing `effect:`"))?;
    if kind == AbilityKind::Activated && costs.is_empty() {
        return Err(CardcError::new(
            path,
            line,
            column,
            "activated ability requires at least one cost",
        ));
    }
    if matches!(kind, AbilityKind::Triggered | AbilityKind::Replacement) && event.is_none() {
        return Err(CardcError::new(
            path,
            line,
            column,
            "triggered/replacement ability requires `event:`",
        ));
    }
    let mana_ability = mana_ability.unwrap_or(false);
    if mana_ability && kind != AbilityKind::Activated {
        return Err(CardcError::new(
            path,
            line,
            column,
            "only an activated ability may be a mana ability",
        ));
    }
    Ok(AbilityDefinition {
        kind,
        costs,
        event,
        condition,
        timing,
        effect,
        mana_ability,
    })
}

fn parse_typed_expression_field(
    path: &str,
    item: Pair<'_, Rule>,
    label: &str,
    category: OperationCategory,
    slot: &mut Option<Expression>,
) -> CardcResult<()> {
    let value = single_inner(path, &item, label)?;
    let (line, column) = location(&value);
    let expression = parse_expression(path, value)?;
    validate_expression(path, line, column, &expression, Some(category))?;
    set_once(path, &item, label, slot, expression)
}

fn parse_expression(path: &str, pair: Pair<'_, Rule>) -> CardcResult<Expression> {
    match pair.as_rule() {
        Rule::call => {
            let (line, column) = location(&pair);
            let mut inner = pair.into_inner();
            let name = inner
                .next()
                .ok_or_else(|| CardcError::new(path, line, column, "operation call has no name"))?;
            let operation = Operation::parse(name.as_str()).ok_or_else(|| {
                at(
                    path,
                    &name,
                    format!("unknown operation `{}`", name.as_str()),
                )
            })?;
            let arguments = match inner.next() {
                Some(arguments) => parse_arguments(path, arguments)?,
                None => Vec::new(),
            };
            let expression = Expression::Call {
                operation,
                arguments,
            };
            validate_expression(path, line, column, &expression, None)?;
            Ok(expression)
        }
        Rule::list => Ok(Expression::List(parse_expression_list(path, pair)?)),
        Rule::string => Ok(Expression::Text(parse_string(path, &pair)?)),
        Rule::integer => pair
            .as_str()
            .parse::<i64>()
            .map(Expression::Integer)
            .map_err(|_error| at(path, &pair, "integer is outside i64 range")),
        Rule::boolean => Ok(Expression::Boolean(pair.as_str() == "true")),
        Rule::identifier => {
            let value = pair.as_str();
            if !valid_symbol(value) {
                return Err(at(path, &pair, format!("invalid symbol `{value}`")));
            }
            Ok(Expression::Symbol(value.to_string()))
        }
        _ => Err(at(path, &pair, "invalid expression")),
    }
}

fn parse_expression_list(path: &str, pair: Pair<'_, Rule>) -> CardcResult<Vec<Expression>> {
    let mut inner = pair.into_inner();
    match inner.next() {
        Some(arguments) => parse_arguments(path, arguments),
        None => Ok(Vec::new()),
    }
}

fn parse_arguments(path: &str, pair: Pair<'_, Rule>) -> CardcResult<Vec<Expression>> {
    pair.into_inner()
        .map(|argument| parse_expression(path, argument))
        .collect()
}

fn parse_keywords(path: &str, pair: Pair<'_, Rule>) -> CardcResult<Vec<KeywordId>> {
    let expressions = parse_expression_list(path, pair)?;
    let mut keywords = Vec::with_capacity(expressions.len());
    for expression in expressions {
        let value = match expression {
            Expression::Text(value) | Expression::Symbol(value) => value,
            _ => {
                return Err(CardcError::new(
                    path,
                    1,
                    1,
                    "keywords must be strings or symbols",
                ))
            }
        };
        validate_keyword(path, 1, 1, &value)?;
        let keyword = KeywordId::parse(value)
            .ok_or_else(|| CardcError::new(path, 1, 1, "invalid keyword id"))?;
        keywords.push(keyword);
    }
    keywords.sort();
    keywords.dedup();
    Ok(keywords)
}

fn parse_mana_cost(path: &str, pair: &Pair<'_, Rule>, text: &str) -> CardcResult<ManaCost> {
    if text.is_empty() {
        return Ok(ManaCost::default());
    }
    let mut rest = text;
    let mut symbols = Vec::new();
    while !rest.is_empty() {
        let Some(opened) = rest.strip_prefix('{') else {
            return Err(at(path, pair, "mana symbol must start with `{`"));
        };
        let Some(end) = opened.find('}') else {
            return Err(at(path, pair, "mana symbol is missing `}`"));
        };
        symbols.push(parse_mana_symbol(path, pair, &opened[..end])?);
        rest = &opened[end + 1..];
    }
    Ok(ManaCost { symbols })
}

fn parse_mana_symbol(path: &str, pair: &Pair<'_, Rule>, symbol: &str) -> CardcResult<ManaSymbol> {
    if let Some(color) = Color::parse(symbol) {
        return Ok(ManaSymbol::Color(color));
    }
    match symbol {
        "C" => return Ok(ManaSymbol::Colorless),
        "S" => return Ok(ManaSymbol::Snow),
        "X" | "Y" | "Z" => {
            let variable = symbol
                .chars()
                .next()
                .ok_or_else(|| at(path, pair, "empty variable"))?;
            return Ok(ManaSymbol::Variable(variable));
        }
        _ => {}
    }
    if let Ok(generic) = symbol.parse::<u16>() {
        return Ok(ManaSymbol::Generic(generic));
    }
    if let Some(color) = symbol.strip_prefix('H').and_then(Color::parse) {
        return Ok(ManaSymbol::Half(color));
    }
    let parts = symbol.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["2", color] => Color::parse(color).map(ManaSymbol::MonoHybrid),
        [color, "P"] => Color::parse(color).map(ManaSymbol::Phyrexian),
        [first, second] => Color::parse(first)
            .zip(Color::parse(second))
            .map(|(first, second)| ManaSymbol::Hybrid(first, second)),
        [first, second, "P"] => Color::parse(first)
            .zip(Color::parse(second))
            .map(|(first, second)| ManaSymbol::HybridPhyrexian(first, second)),
        _ => None,
    }
    .ok_or_else(|| at(path, pair, format!("unknown mana symbol `{{{symbol}}}`")))
}

fn parse_type_line(path: &str, pair: &Pair<'_, Rule>, text: &str) -> CardcResult<TypeLine> {
    let (left, right) = text
        .split_once(" — ")
        .or_else(|| text.split_once(" - "))
        .unwrap_or((text, ""));
    let mut supertypes = Vec::new();
    let mut card_types = Vec::new();
    for word in left.split_whitespace() {
        match word {
            "Basic" => supertypes.push(Supertype::Basic),
            "Legendary" => supertypes.push(Supertype::Legendary),
            "Ongoing" => supertypes.push(Supertype::Ongoing),
            "Snow" => supertypes.push(Supertype::Snow),
            "World" => supertypes.push(Supertype::World),
            "Artifact" => card_types.push(CardType::Artifact),
            "Battle" => card_types.push(CardType::Battle),
            "Creature" => card_types.push(CardType::Creature),
            "Dungeon" => card_types.push(CardType::Dungeon),
            "Enchantment" => card_types.push(CardType::Enchantment),
            "Instant" => card_types.push(CardType::Instant),
            "Kindred" | "Tribal" => card_types.push(CardType::Kindred),
            "Land" => card_types.push(CardType::Land),
            "Phenomenon" => card_types.push(CardType::Phenomenon),
            "Plane" => card_types.push(CardType::Plane),
            "Planeswalker" => card_types.push(CardType::Planeswalker),
            "Scheme" => card_types.push(CardType::Scheme),
            "Sorcery" => card_types.push(CardType::Sorcery),
            "Vanguard" => card_types.push(CardType::Vanguard),
            _ => return Err(at(path, pair, format!("unknown card type `{word}`"))),
        }
    }
    if card_types.is_empty() {
        return Err(at(path, pair, "type line has no card type"));
    }
    let mut subtypes = Vec::new();
    for subtype in right.split_whitespace() {
        if matches!(subtype, "-" | "—") || subtype.chars().any(char::is_control) {
            return Err(at(path, pair, format!("invalid subtype token `{subtype}`")));
        }
        subtypes.push(subtype.to_string());
    }
    Ok(TypeLine {
        supertypes,
        card_types,
        subtypes,
    })
}

fn parse_stat_field(
    path: &str,
    item: Pair<'_, Rule>,
    label: &str,
    slot: &mut Option<String>,
) -> CardcResult<()> {
    let value = single_inner(path, &item, label)?;
    let text = parse_string(path, &value)?;
    if text.is_empty() {
        return Err(at(path, &value, format!("`{label}:` may not be empty")));
    }
    set_once(path, &item, label, slot, text)
}

fn parse_definition_status(path: &str, pair: &Pair<'_, Rule>) -> CardcResult<CardClassification> {
    match pair.as_str() {
        "verified_playable" => Ok(CardClassification::VerifiedPlayable),
        "unverified_playable" => Ok(CardClassification::UnverifiedPlayable),
        value => Err(at(
            path,
            pair,
            format!("mechanics definition cannot use status `{value}`"),
        )),
    }
}

fn validate_face_count(
    path: &str,
    line: usize,
    column: usize,
    layout: CardLayout,
    count: usize,
) -> CardcResult<()> {
    let needs_multiple = matches!(
        layout,
        CardLayout::Split
            | CardLayout::Flip
            | CardLayout::Transform
            | CardLayout::ModalDfc
            | CardLayout::Adventure
            | CardLayout::ReversibleCard
    );
    if needs_multiple && count < 2 {
        return Err(CardcError::new(
            path,
            line,
            column,
            format!("layout `{}` requires at least two faces", layout.as_str()),
        ));
    }
    Ok(())
}

fn parse_string(path: &str, pair: &Pair<'_, Rule>) -> CardcResult<String> {
    serde_json::from_str(pair.as_str()).map_err(|error| at(path, pair, error.to_string()))
}

fn set_once<T>(
    path: &str,
    pair: &Pair<'_, Rule>,
    label: &str,
    slot: &mut Option<T>,
    value: T,
) -> CardcResult<()> {
    if slot.is_some() {
        return Err(at(path, pair, format!("duplicate `{label}:` field")));
    }
    *slot = Some(value);
    Ok(())
}

fn single_inner<'a>(path: &str, pair: &Pair<'a, Rule>, label: &str) -> CardcResult<Pair<'a, Rule>> {
    let mut inner = pair.clone().into_inner();
    let value = inner
        .next()
        .ok_or_else(|| at(path, pair, format!("`{label}:` has no value")))?;
    if inner.next().is_some() {
        return Err(at(path, pair, format!("`{label}:` has extra values")));
    }
    Ok(value)
}

fn next_required<'a>(
    path: &str,
    items: &mut impl Iterator<Item = Pair<'a, Rule>>,
    line: usize,
    column: usize,
    message: &str,
) -> CardcResult<Pair<'a, Rule>> {
    items
        .next()
        .ok_or_else(|| CardcError::new(path, line, column, message))
}

fn at(path: &str, pair: &Pair<'_, Rule>, message: impl Into<String>) -> CardcError {
    let (line, column) = location(pair);
    CardcError::new(path, line, column, message)
}

fn location(pair: &Pair<'_, Rule>) -> (usize, usize) {
    pair.as_span().start_pos().line_col()
}

fn valid_symbol(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first == '_')
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}
