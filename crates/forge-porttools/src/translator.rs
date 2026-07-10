//! Deterministic full-card translation from legacy scripts into canonical Forge source.

use crate::{
    legacy::{collect_scripts, git_revision, parse_legacy_script, LegacyLineKind},
    mapper::map_script_abilities,
};
use forge_cardc::{emit_card, is_known_keyword, parse_card_named};
use forge_carddef::{
    AbilityDefinition, AbilityKind, CardCatalog, CardClassification, CardLayout, OracleId,
};
use rayon::{prelude::*, ThreadPoolBuilder};
use serde::Serialize;
use std::{collections::BTreeMap, fs, path::Path};

/// Inputs and outputs for one local parallel translation campaign.
#[derive(Clone, Copy, Debug)]
pub struct TranslateOptions<'a> {
    /// Pinned legacy cards directory.
    pub root: &'a Path,
    /// Compact Forge card catalog used for stable Oracle identities.
    pub catalog: &'a Path,
    /// Generated `.frs` directory owned by this tool.
    pub output: &'a Path,
    /// Generated translation summary JSON.
    pub metrics: &'a Path,
    /// Generated file-level quarantine JSON.
    pub quarantine: &'a Path,
    /// Number of local worker threads.
    pub jobs: usize,
}

/// Deterministic summary of one translation campaign.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TranslationReport {
    /// Metrics schema version.
    pub schema_version: u32,
    /// Repository-relative source root.
    pub source_root: String,
    /// Exact vendored legacy revision.
    pub source_revision: String,
    /// Legacy scripts considered.
    pub total_scripts: usize,
    /// Scripts emitted and compiler-roundtripped.
    pub emitted_scripts: usize,
    /// Scripts rejected without partial output.
    pub quarantined_scripts: usize,
    /// File-level translation percentage.
    pub emitted_percent: f64,
    /// Local worker count.
    pub jobs: usize,
    /// Quarantine counts by stable code.
    pub quarantine_reason_counts: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct TranslationQuarantine {
    schema_version: u32,
    source_revision: String,
    total_quarantined: usize,
    reason_counts: BTreeMap<String, usize>,
    files: Vec<TranslationFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct TranslationFailure {
    path: String,
    line: usize,
    code: String,
    message: String,
}

enum TranslationOutcome {
    Emitted { relative: String, source: String },
    Quarantined(TranslationFailure),
}

type CatalogIdentities = BTreeMap<String, Option<(OracleId, CardLayout)>>;

/// Translates every legacy script in parallel and emits only complete validated cards.
pub fn translate_all(options: TranslateOptions<'_>) -> Result<TranslationReport, String> {
    if options.jobs == 0 {
        return Err("translation jobs must be positive".to_string());
    }
    let catalog_file = fs::File::open(options.catalog)
        .map_err(|error| format!("could not open {}: {error}", options.catalog.display()))?;
    let catalog: CardCatalog = serde_json::from_reader(catalog_file)
        .map_err(|error| format!("invalid {}: {error}", options.catalog.display()))?;
    let identities = catalog_identities(&catalog);

    let mut paths = Vec::new();
    collect_scripts(options.root, &mut paths)?;
    paths.sort();
    let pool = ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .build()
        .map_err(|error| format!("could not create translation worker pool: {error}"))?;
    let mut outcomes = pool.install(|| {
        paths
            .par_iter()
            .map(|path| translate_one(options.root, path, &identities))
            .collect::<Vec<_>>()
    });
    outcomes.sort_by(|left, right| outcome_path(left).cmp(outcome_path(right)));

    prepare_output(options.output)?;
    let mut failures = Vec::new();
    let mut emitted_scripts = 0;
    for outcome in outcomes {
        match outcome {
            TranslationOutcome::Emitted { relative, source } => {
                let destination = options.output.join(relative).with_extension("frs");
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("could not create {}: {error}", parent.display())
                    })?;
                }
                fs::write(&destination, source).map_err(|error| {
                    format!("could not write {}: {error}", destination.display())
                })?;
                emitted_scripts += 1;
            }
            TranslationOutcome::Quarantined(failure) => failures.push(failure),
        }
    }

    let mut reason_counts = BTreeMap::new();
    for failure in &failures {
        *reason_counts.entry(failure.code.clone()).or_insert(0) += 1;
    }
    let source_revision = git_revision(options.root)?;
    let report = TranslationReport {
        schema_version: 1,
        source_root: crate::repository_relative(options.root),
        source_revision: source_revision.clone(),
        total_scripts: paths.len(),
        emitted_scripts,
        quarantined_scripts: failures.len(),
        emitted_percent: emitted_scripts as f64 * 100.0 / paths.len() as f64,
        jobs: options.jobs,
        quarantine_reason_counts: reason_counts.clone(),
    };
    crate::write_json(options.metrics, &report)?;
    crate::write_json(
        options.quarantine,
        &TranslationQuarantine {
            schema_version: 1,
            source_revision,
            total_quarantined: failures.len(),
            reason_counts,
            files: failures,
        },
    )?;
    Ok(report)
}

fn translate_one(root: &Path, path: &Path, identities: &CatalogIdentities) -> TranslationOutcome {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    match translate_one_inner(path, &relative, identities) {
        Ok(source) => TranslationOutcome::Emitted { relative, source },
        Err((line, code, message)) => TranslationOutcome::Quarantined(TranslationFailure {
            path: relative,
            line,
            code,
            message,
        }),
    }
}

fn translate_one_inner(
    path: &Path,
    relative: &str,
    identities: &CatalogIdentities,
) -> Result<String, (usize, String, String)> {
    let bytes = fs::read(path).map_err(|error| {
        (
            1,
            "READ_ERROR".to_string(),
            format!("could not read source: {error}"),
        )
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        (
            1,
            "INVALID_UTF8".to_string(),
            format!("source is not UTF-8: {error}"),
        )
    })?;
    let script = parse_legacy_script(relative, text)
        .map_err(|error| (error.line, "PARSE_ERROR".to_string(), error.to_string()))?;
    if script.face_count != 1 {
        return Err((
            1,
            "UNSUPPORTED_LAYOUT".to_string(),
            "multi-face emission is not implemented".to_string(),
        ));
    }
    let properties = properties(&script)?;
    let name = required_property(&properties, "Name")?;
    let identity = identities.get(name).ok_or_else(|| {
        (
            1,
            "MISSING_CATALOG_IDENTITY".to_string(),
            format!("catalog has no exact identity for `{name}`"),
        )
    })?;
    let (id, layout) = identity.as_ref().ok_or_else(|| {
        (
            1,
            "AMBIGUOUS_CATALOG_IDENTITY".to_string(),
            format!("catalog name `{name}` resolves to multiple identities"),
        )
    })?;
    if *layout != CardLayout::Normal {
        return Err((
            1,
            "UNSUPPORTED_LAYOUT".to_string(),
            format!(
                "catalog layout `{}` requires multi-face emission",
                layout.as_str()
            ),
        ));
    }
    let mut card = parse_base_card(relative, id, name, &properties, &script)?;
    let mapped = map_script_abilities(&script).map_err(|failure| {
        (
            failure.line,
            failure.diagnostic.code,
            failure.diagnostic.message,
        )
    })?;
    for mapped in mapped {
        let line = mapped.line;
        let selector = mapped.selector;
        let ability = mapped.ability;
        let kind = match ability.prefix {
            crate::legacy::LegacyAbilityPrefix::Activated => match selector.as_str() {
                "SP" => AbilityKind::Spell,
                "AB" => AbilityKind::Activated,
                _ => {
                    return Err((
                        line,
                        "UNSUPPORTED_SELECTOR".to_string(),
                        format!("top-level activated selector `{selector}` cannot be emitted"),
                    ));
                }
            },
            crate::legacy::LegacyAbilityPrefix::Triggered => AbilityKind::Triggered,
            crate::legacy::LegacyAbilityPrefix::Replacement => AbilityKind::Replacement,
            crate::legacy::LegacyAbilityPrefix::Static => AbilityKind::Static,
        };
        card.faces[0].abilities.push(AbilityDefinition {
            kind,
            costs: ability.costs,
            event: ability.event,
            condition: None,
            timing: None,
            effect: ability.expression,
            mana_ability: ability.api == "Mana" && kind == AbilityKind::Activated,
        });
    }
    let emitted = emit_card(&card);
    let reparsed = parse_card_named(relative, &emitted)
        .map_err(|error| (error.line, "COMPILE_ERROR".to_string(), error.to_string()))?;
    if reparsed != card {
        return Err((
            1,
            "ROUNDTRIP_MISMATCH".to_string(),
            "canonical compiler roundtrip changed the translated card".to_string(),
        ));
    }
    Ok(emitted)
}

fn parse_base_card(
    relative: &str,
    id: &OracleId,
    name: &str,
    properties: &BTreeMap<String, String>,
    script: &crate::legacy::LegacyScript,
) -> Result<forge_carddef::CardDefinition, (usize, String, String)> {
    let mana = legacy_mana_cost(properties.get("ManaCost").map(String::as_str).unwrap_or(""))?;
    let types = legacy_type_line(required_property(properties, "Types")?)?;
    let oracle = properties
        .get("Oracle")
        .map(|value| value.replace("\\n", "\n"))
        .unwrap_or_default();
    let mut fields = String::new();
    if let Some(pt) = properties.get("PT") {
        let (power, toughness) = pt.split_once('/').ok_or_else(|| {
            (
                1,
                "INVALID_CHARACTERISTICS".to_string(),
                format!("PT `{pt}` has no slash"),
            )
        })?;
        fields.push_str(&format!("    power: {}\n", quote(power)));
        fields.push_str(&format!("    toughness: {}\n", quote(toughness)));
    }
    if let Some(loyalty) = properties.get("Loyalty") {
        fields.push_str(&format!("    loyalty: {}\n", quote(loyalty)));
    }
    if let Some(defense) = properties.get("Defense") {
        fields.push_str(&format!("    defense: {}\n", quote(defense)));
    }
    let keywords = simple_keywords(script)?;
    let source = format!(
        "card {} {{\n  id: {}\n  layout: normal\n  status: unverified_playable\n  face {} {{\n    cost: {}\n    types: {}\n    oracle: {}\n{}    keywords: [{}]\n  }}\n}}\n",
        quote(name),
        quote(id.as_str()),
        quote(name),
        quote(&mana),
        quote(&types),
        quote(&oracle),
        fields,
        keywords.join(", "),
    );
    let mut card = parse_card_named(relative, &source).map_err(|error| {
        (
            error.line,
            "METADATA_COMPILE_ERROR".to_string(),
            error.to_string(),
        )
    })?;
    card.status = CardClassification::UnverifiedPlayable;
    Ok(card)
}

fn properties(
    script: &crate::legacy::LegacyScript,
) -> Result<BTreeMap<String, String>, (usize, String, String)> {
    let mut properties = BTreeMap::new();
    for line in &script.lines {
        let LegacyLineKind::Property { key, value } = &line.kind else {
            continue;
        };
        if !matches!(
            key.as_str(),
            "Name" | "ManaCost" | "Types" | "PT" | "Loyalty" | "Defense" | "Oracle"
        ) {
            continue;
        }
        if properties.insert(key.clone(), value.clone()).is_some() {
            return Err((
                line.line,
                "DUPLICATE_PROPERTY".to_string(),
                format!("property `{key}` is declared more than once"),
            ));
        }
    }
    Ok(properties)
}

fn required_property<'a>(
    properties: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, (usize, String, String)> {
    properties.get(key).map(String::as_str).ok_or_else(|| {
        (
            1,
            "MISSING_PROPERTY".to_string(),
            format!("required property `{key}` is absent"),
        )
    })
}

fn legacy_mana_cost(value: &str) -> Result<String, (usize, String, String)> {
    if value.is_empty() || value.eq_ignore_ascii_case("no cost") {
        return Ok(String::new());
    }
    let mut output = String::new();
    for symbol in value.split_whitespace() {
        if symbol.contains(['{', '}']) || symbol.is_empty() {
            return Err((
                1,
                "UNSUPPORTED_MANA_COST".to_string(),
                format!("mana symbol `{symbol}` is not canonical legacy syntax"),
            ));
        }
        let normalized = normalize_legacy_mana_symbol(symbol)?;
        output.push('{');
        output.push_str(&normalized);
        output.push('}');
    }
    Ok(output)
}

fn normalize_legacy_mana_symbol(symbol: &str) -> Result<String, (usize, String, String)> {
    let characters = symbol.chars().collect::<Vec<_>>();
    let color = |value: char| matches!(value, 'W' | 'U' | 'B' | 'R' | 'G');
    let normalized = match characters.as_slice() {
        [first, second] if color(*first) && color(*second) => format!("{first}/{second}"),
        ['2', second] if color(*second) => format!("2/{second}"),
        [first, 'P'] if color(*first) => format!("{first}/P"),
        [first, second, 'P'] if color(*first) && color(*second) => {
            format!("{first}/{second}/P")
        }
        _ if symbol.chars().all(|value| {
            value.is_ascii_digit()
                || matches!(
                    value,
                    'W' | 'U' | 'B' | 'R' | 'G' | 'C' | 'S' | 'X' | 'Y' | 'Z'
                )
                || value == '/'
        }) =>
        {
            symbol.to_string()
        }
        _ => {
            return Err((
                1,
                "UNSUPPORTED_MANA_COST".to_string(),
                format!("mana symbol `{symbol}` is outside the closed grammar"),
            ));
        }
    };
    Ok(normalized)
}

fn legacy_type_line(value: &str) -> Result<String, (usize, String, String)> {
    const LEFT: &[&str] = &[
        "Basic",
        "Legendary",
        "Snow",
        "World",
        "Ongoing",
        "Artifact",
        "Battle",
        "Creature",
        "Dungeon",
        "Enchantment",
        "Instant",
        "Kindred",
        "Land",
        "Phenomenon",
        "Plane",
        "Planeswalker",
        "Scheme",
        "Sorcery",
        "Vanguard",
    ];
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    let boundary = tokens
        .iter()
        .position(|token| !LEFT.contains(token))
        .unwrap_or(tokens.len());
    if boundary == 0 {
        return Err((
            1,
            "UNSUPPORTED_TYPE_LINE".to_string(),
            format!("type line `{value}` has no closed card type"),
        ));
    }
    let left = tokens[..boundary].join(" ");
    let right = tokens[boundary..].join(" ");
    Ok(if right.is_empty() {
        left
    } else {
        format!("{left} \u{2014} {right}")
    })
}

fn simple_keywords(
    script: &crate::legacy::LegacyScript,
) -> Result<Vec<String>, (usize, String, String)> {
    let mut keywords = Vec::new();
    for line in &script.lines {
        let LegacyLineKind::Keyword {
            name, arguments, ..
        } = &line.kind
        else {
            continue;
        };
        if !arguments.is_empty() {
            return Err((
                line.line,
                "UNSUPPORTED_KEYWORD".to_string(),
                format!("parameterized keyword `{name}` requires a typed mapper"),
            ));
        }
        let keyword = name.trim().to_ascii_lowercase().replace(' ', "_");
        if !is_known_keyword(&keyword) {
            return Err((
                line.line,
                "UNSUPPORTED_KEYWORD".to_string(),
                format!("keyword `{name}` is outside the closed translation pack"),
            ));
        }
        keywords.push(keyword);
    }
    keywords.sort();
    keywords.dedup();
    Ok(keywords)
}

fn catalog_identities(catalog: &CardCatalog) -> CatalogIdentities {
    let mut identities = BTreeMap::new();
    for identity in &catalog.identities {
        match identities.entry(identity.name.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Some((identity.id.clone(), identity.layout)));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                entry.insert(None);
            }
        }
    }
    identities
}

fn prepare_output(output: &Path) -> Result<(), String> {
    let marker = output.join(".forge-generated");
    if output.exists() {
        if !marker.is_file() {
            return Err(format!(
                "refusing to replace unowned output directory {}",
                output.display()
            ));
        }
        fs::remove_dir_all(output)
            .map_err(|error| format!("could not clear {}: {error}", output.display()))?;
    }
    fs::create_dir_all(output)
        .map_err(|error| format!("could not create {}: {error}", output.display()))?;
    fs::write(
        output.join(".forge-generated"),
        "forge-porttools translate v1\n",
    )
    .map_err(|error| format!("could not mark {}: {error}", output.display()))
}

fn quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn outcome_path(outcome: &TranslationOutcome) -> &str {
    match outcome {
        TranslationOutcome::Emitted { relative, .. } => relative,
        TranslationOutcome::Quarantined(failure) => &failure.path,
    }
}

#[cfg(test)]
mod tests {
    use super::{legacy_mana_cost, legacy_type_line, translate_one_inner, CatalogIdentities};
    use forge_carddef::{CardLayout, OracleId};
    use std::{fs, path::PathBuf};

    #[test]
    fn normalizes_closed_characteristics() {
        assert_eq!(legacy_mana_cost("1 G"), Ok("{1}{G}".to_string()));
        assert_eq!(
            legacy_type_line("Legendary Creature Elf Druid"),
            Ok("Legendary Creature \u{2014} Elf Druid".to_string())
        );
    }

    #[test]
    fn emits_and_roundtrips_a_complete_single_face_card() {
        let root = std::env::temp_dir().join(format!(
            "forge-porttools-translation-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("could not create translator fixture: {error}"));
        let path: PathBuf = root.join("grizzly_bears.txt");
        fs::write(
            &path,
            concat!(
                "Name:Grizzly Bears\n",
                "ManaCost:1 G\n",
                "Types:Creature Bear\n",
                "PT:2/2\n",
                "Oracle:\n",
            ),
        )
        .unwrap_or_else(|error| panic!("could not write translator fixture: {error}"));
        let mut identities = CatalogIdentities::new();
        identities.insert(
            "Grizzly Bears".to_string(),
            Some((
                OracleId::parse("fixture-grizzly")
                    .unwrap_or_else(|| panic!("fixture id should parse")),
                CardLayout::Normal,
            )),
        );
        let emitted = translate_one_inner(&path, "g/grizzly_bears.txt", &identities)
            .unwrap_or_else(|error| panic!("fixture should translate: {error:?}"));
        assert!(emitted.contains("card \"Grizzly Bears\""));
        assert!(emitted.contains("types: \"Creature "));
        fs::remove_dir_all(&root)
            .unwrap_or_else(|error| panic!("could not remove translator fixture: {error}"));
    }
}
