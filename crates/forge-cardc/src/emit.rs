use forge_carddef::{
    AbilityDefinition, CardClassification, CardDefinition, CardFace, CardType, Expression,
    ManaCost, ManaSymbol, Supertype, TypeLine,
};
use std::fmt::Write as _;

/// Emits a validated card in canonical `.frs` form.
#[must_use]
pub fn emit_card(card: &CardDefinition) -> String {
    let mut output = String::new();
    let _ = writeln!(&mut output, "card {} {{", quote(&card.name));
    let _ = writeln!(&mut output, "  id: {}", quote(card.id.as_str()));
    let _ = writeln!(&mut output, "  layout: {}", card.layout.as_str());
    let _ = writeln!(&mut output, "  status: {}", emit_status(&card.status));
    for face in &card.faces {
        emit_face(&mut output, face);
    }
    output.push_str("}\n");
    output
}

fn emit_face(output: &mut String, face: &CardFace) {
    let _ = writeln!(output, "  face {} {{", quote(&face.name));
    let _ = writeln!(
        output,
        "    cost: {}",
        quote(&emit_mana_cost(&face.mana_cost))
    );
    let _ = writeln!(
        output,
        "    types: {}",
        quote(&emit_type_line(&face.type_line))
    );
    let _ = writeln!(output, "    oracle: {}", quote(&face.oracle_text));
    emit_optional_string(output, "power", face.power.as_deref());
    emit_optional_string(output, "toughness", face.toughness.as_deref());
    emit_optional_string(output, "loyalty", face.loyalty.as_deref());
    emit_optional_string(output, "defense", face.defense.as_deref());
    let rendered_keywords = face
        .keywords
        .iter()
        .map(|keyword| keyword.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(output, "    keywords: [{rendered_keywords}]");
    for ability in &face.abilities {
        emit_ability(output, ability);
    }
    output.push_str("  }\n");
}

fn emit_ability(output: &mut String, ability: &AbilityDefinition) {
    let _ = writeln!(output, "    ability {} {{", ability.kind.as_str());
    if !ability.costs.is_empty() {
        let costs = ability
            .costs
            .iter()
            .map(emit_expression)
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(output, "      costs: [{costs}]");
    }
    emit_optional_expression(output, "event", ability.event.as_ref());
    emit_optional_expression(output, "condition", ability.condition.as_ref());
    emit_optional_expression(output, "timing", ability.timing.as_ref());
    let _ = writeln!(output, "      effect: {}", emit_expression(&ability.effect));
    if ability.mana_ability {
        output.push_str("      mana_ability: true\n");
    }
    output.push_str("    }\n");
}

fn emit_optional_expression(output: &mut String, label: &str, value: Option<&Expression>) {
    if let Some(value) = value {
        let _ = writeln!(output, "      {label}: {}", emit_expression(value));
    }
}

fn emit_optional_string(output: &mut String, label: &str, value: Option<&str>) {
    if let Some(value) = value {
        let _ = writeln!(output, "    {label}: {}", quote(value));
    }
}

fn emit_expression(expression: &Expression) -> String {
    match expression {
        Expression::Integer(value) => value.to_string(),
        Expression::Boolean(value) => value.to_string(),
        Expression::Text(value) => quote(value),
        Expression::Symbol(value) => value.clone(),
        Expression::Call {
            operation,
            arguments,
        } => {
            let rendered = arguments
                .iter()
                .map(emit_expression)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({rendered})", operation.as_str())
        }
        Expression::List(items) => {
            let rendered = items
                .iter()
                .map(emit_expression)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{rendered}]")
        }
    }
}

fn emit_status(status: &CardClassification) -> &'static str {
    match status {
        CardClassification::VerifiedPlayable => "verified_playable",
        CardClassification::UnverifiedPlayable
        | CardClassification::Quarantined(_)
        | CardClassification::OutOfV1(_)
        | CardClassification::CatalogOnly(_) => "unverified_playable",
    }
}

fn emit_mana_cost(cost: &ManaCost) -> String {
    cost.symbols.iter().map(emit_mana_symbol).collect()
}

fn emit_mana_symbol(symbol: &ManaSymbol) -> String {
    let body = match symbol {
        ManaSymbol::Color(color) => color.symbol().to_string(),
        ManaSymbol::Generic(value) => value.to_string(),
        ManaSymbol::Colorless => "C".to_string(),
        ManaSymbol::Snow => "S".to_string(),
        ManaSymbol::Variable(value) => value.to_string(),
        ManaSymbol::Hybrid(first, second) => {
            format!("{}/{}", first.symbol(), second.symbol())
        }
        ManaSymbol::MonoHybrid(color) => format!("2/{}", color.symbol()),
        ManaSymbol::Phyrexian(color) => format!("{}/P", color.symbol()),
        ManaSymbol::HybridPhyrexian(first, second) => {
            format!("{}/{}/P", first.symbol(), second.symbol())
        }
        ManaSymbol::Half(color) => format!("H{}", color.symbol()),
    };
    format!("{{{body}}}")
}

fn emit_type_line(type_line: &TypeLine) -> String {
    let mut left = Vec::new();
    left.extend(type_line.supertypes.iter().map(|value| match value {
        Supertype::Basic => "Basic",
        Supertype::Legendary => "Legendary",
        Supertype::Ongoing => "Ongoing",
        Supertype::Snow => "Snow",
        Supertype::World => "World",
    }));
    left.extend(type_line.card_types.iter().map(|value| match value {
        CardType::Artifact => "Artifact",
        CardType::Battle => "Battle",
        CardType::Creature => "Creature",
        CardType::Dungeon => "Dungeon",
        CardType::Enchantment => "Enchantment",
        CardType::Instant => "Instant",
        CardType::Kindred => "Kindred",
        CardType::Land => "Land",
        CardType::Phenomenon => "Phenomenon",
        CardType::Plane => "Plane",
        CardType::Planeswalker => "Planeswalker",
        CardType::Scheme => "Scheme",
        CardType::Sorcery => "Sorcery",
        CardType::Vanguard => "Vanguard",
    }));
    let mut rendered = left.join(" ");
    if !type_line.subtypes.is_empty() {
        rendered.push_str(" - ");
        rendered.push_str(&type_line.subtypes.join(" "));
    }
    rendered
}

fn quote(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if control.is_control() => {
                let _ = write!(output, "\\u{:04x}", u32::from(control));
            }
            other => output.push(other),
        }
    }
    output.push('"');
    output
}
