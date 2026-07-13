//! Deterministic full-card translation from legacy scripts into canonical Forge source.

use crate::{
    legacy::{
        collect_scripts, git_revision, parse_legacy_script, LegacyLine, LegacyLineKind,
        LegacyScript,
    },
    mapper::{
        affected_selector, card_selector_in_zone, map_named_svar_ability, map_script_abilities,
        parse_simple_cost, resolve_value_svar, valid_target_selector, MappingContext,
    },
};
use forge_cardc::{emit_card, is_known_keyword, parse_card_named};
use forge_carddef::{
    AbilityDefinition, AbilityKind, CardCatalog, CardClassification, CardLayout, Expression,
    Operation, OracleId,
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
    /// Owner-approved card coverage priority list.
    pub priority: &'a Path,
    /// Generated tier-aware priority coverage JSON.
    pub priority_metrics: &'a Path,
    /// Number of local worker threads.
    pub jobs: usize,
    /// Whether to materialize the generated `.frs` output tree.
    pub write_output: bool,
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
    /// Owner-priority card names requested.
    pub priority_requested: usize,
    /// Owner-priority names resolved to playable catalog identities.
    pub priority_catalog_resolved: usize,
    /// Owner-priority cards emitted by this campaign.
    pub priority_emitted: usize,
    /// Owner-priority file-level translation percentage.
    pub priority_emitted_percent: f64,
    /// Stable dual-64-bit fingerprint of every sorted emitted path and source.
    pub output_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct PriorityCoverageReport {
    schema_version: u32,
    source_path: String,
    total_requested: usize,
    catalog_resolved: usize,
    emitted: usize,
    emitted_percent: f64,
    tiers: Vec<PriorityTierReport>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct PriorityTierReport {
    tier: u8,
    label: String,
    requested: usize,
    catalog_resolved: usize,
    emitted: usize,
    emitted_percent: f64,
    cards: Vec<PriorityCardResult>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct PriorityCardResult {
    requested_name: String,
    catalog_name: Option<String>,
    status: String,
    path: Option<String>,
    code: Option<String>,
    message: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PriorityTier {
    tier: u8,
    label: String,
    names: Vec<String>,
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
    Emitted {
        relative: String,
        source: String,
        aliases: Vec<String>,
    },
    Quarantined {
        failure: TranslationFailure,
        aliases: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CatalogIdentity {
    id: OracleId,
    name: String,
    layout: CardLayout,
    face_names: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct CatalogIdentities {
    by_name: BTreeMap<String, Option<CatalogIdentity>>,
    by_faces: BTreeMap<Vec<String>, Option<CatalogIdentity>>,
}

#[derive(Clone, Debug, Default)]
struct TranslatedKeywords {
    ids: Vec<String>,
    abilities: Vec<AbilityDefinition>,
}

/// Translates every legacy script in parallel and emits only complete validated cards.
pub fn translate_all(options: TranslateOptions<'_>) -> Result<TranslationReport, String> {
    crate::validate_local_worker_count("translation", options.jobs)?;
    let catalog_file = fs::File::open(options.catalog)
        .map_err(|error| format!("could not open {}: {error}", options.catalog.display()))?;
    let catalog: CardCatalog = serde_json::from_reader(catalog_file)
        .map_err(|error| format!("invalid {}: {error}", options.catalog.display()))?;
    let identities = catalog_identities(&catalog);
    let priority_tiers = read_priority_tiers(options.priority)?;

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
    let output_fingerprint = translation_fingerprint(&outcomes);
    let priority_report =
        build_priority_report(options.priority, &priority_tiers, &catalog, &outcomes);

    if options.write_output {
        prepare_output(options.output)?;
    }
    let mut failures = Vec::new();
    let mut emitted_scripts = 0;
    for outcome in outcomes {
        match outcome {
            TranslationOutcome::Emitted {
                relative, source, ..
            } => {
                if options.write_output {
                    let destination = options.output.join(relative).with_extension("frs");
                    if let Some(parent) = destination.parent() {
                        fs::create_dir_all(parent).map_err(|error| {
                            format!("could not create {}: {error}", parent.display())
                        })?;
                    }
                    fs::write(&destination, source).map_err(|error| {
                        format!("could not write {}: {error}", destination.display())
                    })?;
                }
                emitted_scripts += 1;
            }
            TranslationOutcome::Quarantined { failure, .. } => failures.push(failure),
        }
    }

    let mut reason_counts = BTreeMap::new();
    for failure in &failures {
        *reason_counts.entry(failure.code.clone()).or_insert(0) += 1;
    }
    let source_revision = git_revision(options.root)?;
    let report = TranslationReport {
        schema_version: 2,
        source_root: crate::repository_relative(options.root),
        source_revision: source_revision.clone(),
        total_scripts: paths.len(),
        emitted_scripts,
        quarantined_scripts: failures.len(),
        emitted_percent: emitted_scripts as f64 * 100.0 / paths.len() as f64,
        jobs: options.jobs,
        quarantine_reason_counts: reason_counts.clone(),
        priority_requested: priority_report.total_requested,
        priority_catalog_resolved: priority_report.catalog_resolved,
        priority_emitted: priority_report.emitted,
        priority_emitted_percent: priority_report.emitted_percent,
        output_fingerprint,
    };
    crate::write_json(options.metrics, &report)?;
    crate::write_json(options.priority_metrics, &priority_report)?;
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

fn translation_fingerprint(outcomes: &[TranslationOutcome]) -> String {
    let mut first = 0xcbf2_9ce4_8422_2325_u64;
    let mut second = 0x9e37_79b1_85eb_ca87_u64;
    for outcome in outcomes {
        let TranslationOutcome::Emitted {
            relative, source, ..
        } = outcome
        else {
            continue;
        };
        fingerprint_bytes(
            &mut first,
            &mut second,
            &(relative.len() as u64).to_le_bytes(),
        );
        fingerprint_bytes(&mut first, &mut second, relative.as_bytes());
        fingerprint_bytes(
            &mut first,
            &mut second,
            &(source.len() as u64).to_le_bytes(),
        );
        fingerprint_bytes(&mut first, &mut second, source.as_bytes());
    }
    format!("{first:016x}{second:016x}")
}

fn fingerprint_bytes(first: &mut u64, second: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *first ^= u64::from(*byte);
        *first = first.wrapping_mul(0x0000_0100_0000_01b3);
        *second ^= u64::from(*byte);
        *second = second.rotate_left(13).wrapping_mul(0x9e37_79b1_85eb_ca87);
    }
}

fn translate_one(root: &Path, path: &Path, identities: &CatalogIdentities) -> TranslationOutcome {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let script = match read_legacy_script(path, &relative) {
        Ok(script) => script,
        Err((line, code, message)) => {
            return TranslationOutcome::Quarantined {
                failure: TranslationFailure {
                    path: relative,
                    line,
                    code,
                    message,
                },
                aliases: Vec::new(),
            };
        }
    };
    let mut aliases = candidate_names(&script);
    match translate_script(&relative, &script, identities) {
        Ok((source, catalog_name)) => {
            aliases.push(catalog_name);
            aliases.sort();
            aliases.dedup();
            TranslationOutcome::Emitted {
                relative,
                source,
                aliases,
            }
        }
        Err((line, code, message)) => TranslationOutcome::Quarantined {
            failure: TranslationFailure {
                path: relative,
                line,
                code,
                message,
            },
            aliases,
        },
    }
}

#[cfg(test)]
fn translate_one_inner(
    path: &Path,
    relative: &str,
    identities: &CatalogIdentities,
) -> Result<String, (usize, String, String)> {
    let script = read_legacy_script(path, relative)?;
    translate_script(relative, &script, identities).map(|(source, _)| source)
}

fn read_legacy_script(
    path: &Path,
    relative: &str,
) -> Result<LegacyScript, (usize, String, String)> {
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
    parse_legacy_script(relative, text)
        .map_err(|error| (error.line, "PARSE_ERROR".to_string(), error.to_string()))
}

fn translate_script(
    relative: &str,
    script: &LegacyScript,
    identities: &CatalogIdentities,
) -> Result<(String, String), (usize, String, String)> {
    let faces = face_lines(script);
    let face_properties = faces
        .iter()
        .map(|face| properties(face))
        .collect::<Result<Vec<_>, _>>()?;
    let face_names = face_properties
        .iter()
        .map(|properties| required_property(properties, "Name").map(str::to_string))
        .collect::<Result<Vec<_>, _>>()?;
    let identity = catalog_identity(identities, &face_names)?;
    if identity.face_names.len() != faces.len() {
        return Err((
            1,
            "LAYOUT_MISMATCH".to_string(),
            format!(
                "catalog identity has {} face(s), but the legacy script has {}",
                identity.face_names.len(),
                faces.len()
            ),
        ));
    }
    let mut card = parse_base_card(relative, identity, script, &face_properties, &faces)?;
    let mapped = map_script_abilities(script).map_err(|failure| {
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
        let face_index = face_index_for_line(&faces, line).ok_or_else(|| {
            (
                line,
                "ABILITY_FACE_MISMATCH".to_string(),
                "ability source line does not belong to a card face".to_string(),
            )
        })?;
        card.faces[face_index].abilities.push(AbilityDefinition {
            kind,
            costs: ability.costs,
            event: ability.event,
            condition: None,
            timing: ability.timing,
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
    Ok((emitted, identity.name.clone()))
}

fn parse_base_card(
    relative: &str,
    identity: &CatalogIdentity,
    script: &LegacyScript,
    face_properties: &[BTreeMap<String, String>],
    faces: &[Vec<&LegacyLine>],
) -> Result<forge_carddef::CardDefinition, (usize, String, String)> {
    let mut face_sources = String::new();
    for (properties, lines) in face_properties.iter().zip(faces) {
        let name = required_property(properties, "Name")?;
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
        let keywords = translate_keywords(script, lines)?;
        face_sources.push_str(&format!(
            "  face {} {{\n    cost: {}\n    types: {}\n    oracle: {}\n{}    keywords: [{}]\n  }}\n",
            quote(name),
            quote(&mana),
            quote(&types),
            quote(&oracle),
            fields,
            keywords.ids.join(", "),
        ));
    }
    let source = format!(
        "card {} {{\n  id: {}\n  layout: {}\n  status: unverified_playable\n{face_sources}}}\n",
        quote(&identity.name),
        quote(identity.id.as_str()),
        identity.layout.as_str(),
    );
    let mut card = parse_card_named(relative, &source).map_err(|error| {
        (
            error.line,
            "METADATA_COMPILE_ERROR".to_string(),
            error.to_string(),
        )
    })?;
    for (face, lines) in card.faces.iter_mut().zip(faces) {
        face.abilities
            .extend(translate_keywords(script, lines)?.abilities);
    }
    card.status = CardClassification::UnverifiedPlayable;
    Ok(card)
}

fn properties(lines: &[&LegacyLine]) -> Result<BTreeMap<String, String>, (usize, String, String)> {
    let mut properties = BTreeMap::new();
    for line in lines {
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

pub(crate) fn collect_script_keyword_blockers(
    script: &LegacyScript,
) -> Vec<(usize, String, String, usize)> {
    let mut blockers = Vec::new();
    for line in &script.lines {
        let LegacyLineKind::Keyword {
            name, arguments, ..
        } = &line.kind
        else {
            continue;
        };
        if name.eq_ignore_ascii_case("Chapter") && arguments.len() == 2 {
            let names = arguments[1]
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>();
            let valid_shape = arguments[0]
                .parse::<usize>()
                .ok()
                .is_some_and(|maximum| maximum > 0 && maximum == names.len());
            if valid_shape {
                let mut fanout = BTreeMap::<&str, usize>::new();
                for name in names {
                    *fanout.entry(name).or_insert(0) += 1;
                }
                let mut linked_blocker = false;
                for (name, references) in fanout {
                    match map_named_svar_ability(script, name) {
                        Ok(linked)
                            if linked.costs.is_empty()
                                && linked.event.is_none()
                                && linked.timing.is_none() => {}
                        Ok(_) => {
                            linked_blocker = true;
                            blockers.push((
                                line.line,
                                "UNSUPPORTED_LINK".to_string(),
                                format!(
                                    "Chapter SVar `{name}` must be a cost-free DB effect without nested event or timing"
                                ),
                                references,
                            ));
                        }
                        Err(diagnostic) => {
                            linked_blocker = true;
                            blockers.push((
                                line.line,
                                diagnostic.code,
                                diagnostic.message,
                                references,
                            ));
                        }
                    }
                }
                if linked_blocker {
                    continue;
                }
            }
        }
        if let Err(blocker) = translate_keywords(script, std::slice::from_ref(&line)) {
            blockers.push((blocker.0, blocker.1, blocker.2, 1));
        }
    }
    blockers
}

fn translate_keywords(
    script: &LegacyScript,
    lines: &[&LegacyLine],
) -> Result<TranslatedKeywords, (usize, String, String)> {
    let mut translated = TranslatedKeywords::default();
    for line in lines {
        let LegacyLineKind::Keyword {
            name, arguments, ..
        } = &line.kind
        else {
            continue;
        };
        let keyword = name.trim().to_ascii_lowercase().replace(' ', "_");
        let (keyword_id, ability) = match (keyword.as_str(), arguments.as_slice()) {
            ("chapter", [max, svar_names]) => {
                let max = max.parse::<i64>().map_err(|_| {
                    (
                        line.line,
                        "UNSUPPORTED_KEYWORD".to_string(),
                        format!(
                            "keyword `{name}` chapter maximum `{max}` is not a positive integer"
                        ),
                    )
                })?;
                let names = svar_names.split(',').map(str::trim).collect::<Vec<_>>();
                if max <= 0
                    || names.iter().any(|name| name.is_empty())
                    || max as usize != names.len()
                {
                    return Err((
                        line.line,
                        "UNSUPPORTED_KEYWORD".to_string(),
                        format!(
                            "keyword `{name}` requires a positive maximum equal to the number of referenced SVars"
                        ),
                    ));
                }
                let mut abilities = Vec::with_capacity(names.len());
                for (index, svar_name) in names.into_iter().enumerate() {
                    let linked = map_named_svar_ability(script, svar_name)
                        .map_err(|diagnostic| (line.line, diagnostic.code, diagnostic.message))?;
                    if !linked.costs.is_empty() || linked.event.is_some() || linked.timing.is_some()
                    {
                        return Err((
                            line.line,
                            "UNSUPPORTED_LINK".to_string(),
                            format!(
                                "Chapter SVar `{svar_name}` must be a cost-free effect without nested event or timing"
                            ),
                        ));
                    }
                    let chapter = (index + 1) as i64;
                    abilities.push(AbilityDefinition {
                        kind: AbilityKind::Triggered,
                        costs: Vec::new(),
                        event: Some(expression_call(
                            Operation::EventChapter,
                            vec![
                                expression_call(Operation::Source, vec![]),
                                Expression::Integer(chapter),
                                Expression::Integer(max),
                            ],
                        )),
                        condition: None,
                        timing: None,
                        effect: expression_call(
                            Operation::Chapter,
                            vec![
                                Expression::Integer(chapter),
                                Expression::Integer(max),
                                linked.expression,
                            ],
                        ),
                        mana_ability: false,
                    });
                }
                translated.abilities.extend(abilities);
                (None, None)
            }
            ("etbcounter", [counter, amount]) => (
                None,
                Some(translate_etb_counter(
                    script, line.line, name, counter, amount, None,
                )?),
            ),
            ("etbcounter", [counter, amount, condition])
                if condition.eq_ignore_ascii_case("no Condition") =>
            {
                (
                    None,
                    Some(translate_etb_counter(
                        script, line.line, name, counter, amount, None,
                    )?),
                )
            }
            ("etbcounter", [counter, amount, condition, _description])
                if condition.eq_ignore_ascii_case("no Condition") =>
            {
                (
                    None,
                    Some(translate_etb_counter(
                        script, line.line, name, counter, amount, None,
                    )?),
                )
            }
            ("etbcounter", [counter, amount, condition, _description])
                if condition.starts_with("CheckSVar$ ") =>
            {
                (
                    None,
                    Some(translate_etb_counter(
                        script,
                        line.line,
                        name,
                        counter,
                        amount,
                        condition.strip_prefix("CheckSVar$ "),
                    )?),
                )
            }
            ("etbreplacement", [scope, replacement, mandatory, zone, affected])
                if scope == "Other"
                    && replacement == "AddExtraCounter"
                    && mandatory == "Mandatory"
                    && zone == "Battlefield" =>
            {
                (
                    None,
                    Some(translate_etb_extra_counter(line.line, affected)?),
                )
            }
            ("etbreplacement", [scope, replacement, choice])
                if scope == "Copy" && matches!(choice.as_str(), "Optional" | "Mandatory") =>
            {
                let linked = map_named_svar_ability(script, replacement)
                    .map_err(|diagnostic| (line.line, diagnostic.code, diagnostic.message))?;
                if !linked.costs.is_empty() || linked.event.is_some() || linked.timing.is_some() {
                    return Err((
                        line.line,
                        "UNSUPPORTED_LINK".to_string(),
                        format!(
                            "ETBReplacement SVar `{replacement}` must be a cost-free DB effect without nested event or timing"
                        ),
                    ));
                }
                let effect = if choice == "Optional" {
                    expression_call(
                        Operation::ChooseUpTo,
                        vec![Expression::Integer(1), linked.expression],
                    )
                } else {
                    linked.expression
                };
                translated.abilities.push(AbilityDefinition {
                    kind: AbilityKind::Replacement,
                    costs: Vec::new(),
                    event: Some(expression_call(
                        Operation::EventEnters,
                        vec![expression_call(Operation::Source, vec![])],
                    )),
                    condition: None,
                    timing: None,
                    effect,
                    mana_ability: false,
                });
                (None, None)
            }
            ("etbreplacement", [scope, replacement]) if scope == "Other" => {
                let linked = map_named_svar_ability(script, replacement)
                    .map_err(|diagnostic| (line.line, diagnostic.code, diagnostic.message))?;
                if !linked.costs.is_empty() || linked.event.is_some() || linked.timing.is_some() {
                    return Err((
                        line.line,
                        "UNSUPPORTED_LINK".to_string(),
                        format!(
                            "ETBReplacement SVar `{replacement}` must be a cost-free DB effect without nested event or timing"
                        ),
                    ));
                }
                translated.abilities.push(AbilityDefinition {
                    kind: AbilityKind::Replacement,
                    costs: Vec::new(),
                    event: Some(expression_call(
                        Operation::EventEnters,
                        vec![expression_call(Operation::Source, vec![])],
                    )),
                    condition: None,
                    timing: None,
                    effect: linked.expression,
                    mana_ability: false,
                });
                (None, None)
            }
            ("landwalk", [land]) => {
                let keyword = match land.as_str() {
                    "Desert" => "desertwalk",
                    "Forest" => "forestwalk",
                    "Forest.Snow" => "snowforestwalk",
                    "Island" => "islandwalk",
                    "Land.Legendary" => "legendarylandwalk",
                    "Land.Snow" => "snowlandwalk",
                    "Land.nonBasic" => "nonbasiclandwalk",
                    "Mountain" => "mountainwalk",
                    "Plains" => "plainswalk",
                    "Plains.Snow" => "snowplainswalk",
                    "Swamp" => "swampwalk",
                    "Swamp.Snow" => "snowswampwalk",
                    value => {
                        return Err((
                            line.line,
                            "UNSUPPORTED_VALUE".to_string(),
                            format!("keyword `{name}` land type `{value}` has no closed lowering"),
                        ));
                    }
                };
                (Some(keyword.to_string()), None)
            }
            ("affinity", [validity]) => (
                Some(keyword.clone()),
                Some(translate_affinity_keyword(line.line, validity)?),
            ),
            ("affinity", [validity, _description]) => (
                Some(keyword.clone()),
                Some(translate_affinity_keyword(line.line, validity)?),
            ),
            ("unearth", [cost]) => {
                let costs = parse_simple_cost(Some(cost))
                    .map_err(|diagnostic| (line.line, diagnostic.code, diagnostic.message))?;
                (
                    Some(keyword.clone()),
                    Some(AbilityDefinition {
                        kind: AbilityKind::Activated,
                        costs,
                        event: None,
                        condition: None,
                        timing: Some(expression_call(Operation::TimingSorcery, vec![])),
                        effect: expression_call(
                            Operation::Unearth,
                            vec![expression_call(Operation::Source, vec![])],
                        ),
                        mana_ability: false,
                    }),
                )
            }
            ("morph", [cost]) => {
                let costs = parse_simple_cost(Some(cost))
                    .map_err(|diagnostic| (line.line, diagnostic.code, diagnostic.message))?;
                (
                    Some(keyword.clone()),
                    Some(AbilityDefinition {
                        kind: AbilityKind::Activated,
                        costs,
                        event: None,
                        condition: None,
                        timing: None,
                        effect: expression_call(
                            Operation::Morph,
                            vec![expression_call(Operation::Source, vec![])],
                        ),
                        mana_ability: false,
                    }),
                )
            }
            ("ward", [cost]) => (
                Some(keyword.clone()),
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::WardCost,
                )?),
            ),
            ("echo", [cost]) => (
                Some(keyword.clone()),
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::EchoCost,
                )?),
            ),
            ("cumulative_upkeep", [cost]) | ("cumulative_upkeep", [cost, _]) => (
                Some(keyword.clone()),
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::CumulativeUpkeepCost,
                )?),
            ),
            ("suspend", [time, cost]) => (
                Some(keyword.clone()),
                Some(translate_suspend_keyword(line.line, time, cost)?),
            ),
            ("kicker", costs @ [_, ..]) => (
                None,
                Some(translate_keyword_cost_options(
                    line.line,
                    costs,
                    Operation::KickerCost,
                )?),
            ),
            ("multikicker", costs @ [_, ..]) => (
                None,
                Some(translate_keyword_cost_options(
                    line.line,
                    costs,
                    Operation::MultikickerCost,
                )?),
            ),
            ("buyback", costs @ [_, ..]) => (
                None,
                Some(translate_keyword_cost_options(
                    line.line,
                    costs,
                    Operation::BuybackCost,
                )?),
            ),
            ("alternateadditionalcost", costs @ [_, ..]) => (
                None,
                Some(translate_keyword_cost_options(
                    line.line,
                    costs,
                    Operation::AlternateAdditionalCost,
                )?),
            ),
            (
                protection @ ("protection_from_black"
                | "protection_from_blue"
                | "protection_from_green"
                | "protection_from_red"
                | "protection_from_white"
                | "protection_from_each_color"
                | "protection_from_everything"),
                [],
            ) => (
                None,
                Some(AbilityDefinition {
                    kind: AbilityKind::Static,
                    costs: Vec::new(),
                    event: None,
                    condition: None,
                    timing: None,
                    effect: expression_call(
                        Operation::ProtectionFrom,
                        vec![
                            expression_call(Operation::Source, vec![]),
                            Expression::Text(
                                protection
                                    .strip_prefix("protection_from_")
                                    .unwrap_or(protection)
                                    .to_string(),
                            ),
                        ],
                    ),
                    mana_ability: false,
                }),
            ),
            ("protection", [protected_from]) | ("protection", [protected_from, _]) => (
                None,
                Some(AbilityDefinition {
                    kind: AbilityKind::Static,
                    costs: Vec::new(),
                    event: None,
                    condition: None,
                    timing: None,
                    effect: expression_call(
                        Operation::ProtectionFrom,
                        vec![
                            expression_call(Operation::Source, vec![]),
                            Expression::Text(protected_from.to_string()),
                        ],
                    ),
                    mana_ability: false,
                }),
            ),
            ("disguise", [cost]) => (
                None,
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::Disguise,
                )?),
            ),
            ("megamorph", [cost]) => (
                None,
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::Megamorph,
                )?),
            ),
            ("entwine", [cost]) => (
                None,
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::EntwineCost,
                )?),
            ),
            ("toxic", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Toxic,
                )?),
            ),
            ("bushido", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Bushido,
                )?),
            ),
            ("soulshift", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Soulshift,
                )?),
            ),
            ("ninjutsu", [cost]) => (
                None,
                Some(translate_activated_keyword_rule(
                    line.line,
                    cost,
                    Operation::Ninjutsu,
                    None,
                )?),
            ),
            ("saddle", [amount]) => (
                None,
                Some(translate_numeric_activated_keyword_rule(
                    line.line,
                    amount,
                    Operation::Saddle,
                    None,
                )?),
            ),
            ("level_up", [cost]) => (
                None,
                Some(translate_activated_keyword_rule(
                    line.line,
                    cost,
                    Operation::LevelUp,
                    Some(expression_call(Operation::TimingSorcery, vec![])),
                )?),
            ),
            ("encore", [cost]) => (
                None,
                Some(translate_activated_keyword_rule(
                    line.line,
                    cost,
                    Operation::Encore,
                    Some(expression_call(Operation::TimingSorcery, vec![])),
                )?),
            ),
            ("embalm", [cost]) => (
                None,
                Some(translate_activated_keyword_rule(
                    line.line,
                    cost,
                    Operation::Embalm,
                    Some(expression_call(Operation::TimingSorcery, vec![])),
                )?),
            ),
            ("eternalize", [cost]) => (
                None,
                Some(translate_activated_keyword_rule(
                    line.line,
                    cost,
                    Operation::Eternalize,
                    Some(expression_call(Operation::TimingSorcery, vec![])),
                )?),
            ),
            ("plot", [cost]) => (
                None,
                Some(translate_activated_keyword_rule(
                    line.line,
                    cost,
                    Operation::Plot,
                    Some(expression_call(Operation::TimingSorcery, vec![])),
                )?),
            ),
            ("warp", [cost]) => (
                None,
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::WarpCost,
                )?),
            ),
            ("sneak", [cost]) => (
                None,
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::SneakCost,
                )?),
            ),
            ("strive", [cost]) => (
                None,
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::StriveCost,
                )?),
            ),
            ("replicate", [cost]) => (
                None,
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::ReplicateCost,
                )?),
            ),
            ("miracle", [cost]) => (
                None,
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::MiracleCost,
                )?),
            ),
            ("offspring", [cost]) => (
                None,
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::OffspringCost,
                )?),
            ),
            ("firebending", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Firebending,
                )?),
            ),
            ("vanishing", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Vanishing,
                )?),
            ),
            ("fading", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Fading,
                )?),
            ),
            ("prototype", [cost, power, toughness]) => (
                None,
                Some(translate_prototype_keyword(
                    line.line, cost, power, toughness,
                )?),
            ),
            ("station", [amount]) => (
                None,
                Some(translate_numeric_activated_keyword_rule(
                    line.line,
                    amount,
                    Operation::Station,
                    None,
                )?),
            ),
            ("renown", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Renown,
                )?),
            ),
            ("bloodthirst", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Bloodthirst,
                )?),
            ),
            ("fabricate", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Fabricate,
                )?),
            ),
            ("modular", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Modular,
                )?),
            ),
            ("devour", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Devour,
                )?),
            ),
            ("teamwork", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Teamwork,
                )?),
            ),
            ("starting_intensity", [amount]) => (
                None,
                Some(translate_nonnegative_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::StartingIntensity,
                )?),
            ),
            ("casualty", [amount]) => (
                None,
                Some(translate_numeric_keyword_rule(
                    line.line,
                    amount,
                    Operation::Casualty,
                )?),
            ),
            ("reconfigure", [cost]) => (
                None,
                Some(translate_activated_keyword_rule(
                    line.line,
                    cost,
                    Operation::Reconfigure,
                    Some(expression_call(Operation::TimingSorcery, vec![])),
                )?),
            ),
            ("mutate", [cost]) => (
                None,
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::MutateCost,
                )?),
            ),
            ("emerge", [cost]) => (
                None,
                Some(translate_costed_keyword_rule(
                    line.line,
                    cost,
                    Operation::EmergeCost,
                )?),
            ),
            ("splice", [validity, cost]) => (
                None,
                Some(translate_splice_keyword(line.line, validity, cost)?),
            ),
            ("awaken", [amount, cost]) => (
                None,
                Some(translate_awaken_keyword(line.line, amount, cost)?),
            ),
            ("start_your_engines", []) => (
                None,
                Some(translate_marker_keyword(Operation::StartYourEngines)),
            ),
            ("choose_a_background", []) => (
                None,
                Some(translate_marker_keyword(Operation::ChooseBackground)),
            ),
            ("doctor's_companion", []) => (
                None,
                Some(translate_marker_keyword(Operation::DoctorsCompanion)),
            ),
            ("bargain", []) => (None, Some(translate_marker_keyword(Operation::Bargain))),
            ("partner_with", partners @ [_, ..]) => {
                let mut arguments = vec![expression_call(Operation::Source, vec![])];
                arguments.extend(
                    partners
                        .iter()
                        .map(|partner| Expression::Text(partner.clone())),
                );
                (
                    None,
                    Some(AbilityDefinition {
                        kind: AbilityKind::Static,
                        costs: Vec::new(),
                        event: None,
                        condition: None,
                        timing: None,
                        effect: expression_call(Operation::PartnerWith, arguments),
                        mana_ability: false,
                    }),
                )
            }
            ("partner", [group]) => (
                None,
                Some(AbilityDefinition {
                    kind: AbilityKind::Static,
                    costs: Vec::new(),
                    event: None,
                    condition: None,
                    timing: None,
                    effect: expression_call(
                        Operation::PartnerGroup,
                        vec![
                            expression_call(Operation::Source, vec![]),
                            Expression::Text(group.clone()),
                        ],
                    ),
                    mana_ability: false,
                }),
            ),
            (_, []) => (Some(keyword.clone()), None),
            ("cycling", [cost]) => {
                let full_cost = format!("{cost} Discard<1/CARDNAME>");
                let costs = parse_simple_cost(Some(&full_cost))
                    .map_err(|diagnostic| (line.line, diagnostic.code, diagnostic.message))?;
                (
                    Some(keyword.clone()),
                    Some(AbilityDefinition {
                        kind: AbilityKind::Activated,
                        costs,
                        event: None,
                        condition: None,
                        timing: None,
                        effect: expression_call(
                            Operation::Draw,
                            vec![
                                Expression::Integer(1),
                                expression_call(Operation::You, vec![]),
                            ],
                        ),
                        mana_ability: false,
                    }),
                )
            }
            ("typecycling", [validity, cost]) | ("typecycling", [validity, cost, _]) => {
                let full_cost = format!("{cost} Discard<1/CARDNAME>");
                let costs = parse_simple_cost(Some(&full_cost))
                    .map_err(|diagnostic| (line.line, diagnostic.code, diagnostic.message))?;
                (
                    Some(keyword.clone()),
                    Some(AbilityDefinition {
                        kind: AbilityKind::Activated,
                        costs,
                        event: None,
                        condition: None,
                        timing: None,
                        effect: expression_call(
                            Operation::SearchLibrary,
                            vec![
                                card_selector_in_zone(validity, "library").map_err(
                                    |diagnostic| (line.line, diagnostic.code, diagnostic.message),
                                )?,
                                expression_call(Operation::You, vec![]),
                            ],
                        ),
                        mana_ability: false,
                    }),
                )
            }
            ("crew", [amount]) => (
                Some(keyword.clone()),
                Some(translate_crew_keyword(line.line, name, amount, None)?),
            ),
            ("crew", [amount, activation]) if activation == "ActivationLimit$ 1" => (
                Some(keyword.clone()),
                Some(translate_crew_keyword(
                    line.line,
                    name,
                    amount,
                    Some(expression_call(Operation::TimingOnceEachTurn, vec![])),
                )?),
            ),
            (
                "flashback" | "madness" | "foretell" | "dash" | "spectacle" | "overload" | "escape"
                | "blitz" | "disturb" | "evoke" | "bestow",
                [cost],
            ) => (
                Some(keyword.clone()),
                Some(alternative_cost_ability(cost, line.line)?),
            ),
            ("equip", [cost]) => {
                let costs = parse_simple_cost(Some(cost))
                    .map_err(|diagnostic| (line.line, diagnostic.code, diagnostic.message))?;
                (
                    Some(keyword.clone()),
                    Some(AbilityDefinition {
                        kind: AbilityKind::Activated,
                        costs,
                        event: None,
                        condition: None,
                        timing: Some(expression_call(Operation::TimingSorcery, vec![])),
                        effect: expression_call(
                            Operation::Attach,
                            vec![
                                expression_call(Operation::Source, vec![]),
                                valid_target_selector("Creature.YouCtrl").map_err(
                                    |diagnostic| (line.line, diagnostic.code, diagnostic.message),
                                )?,
                            ],
                        ),
                        mana_ability: false,
                    }),
                )
            }
            ("enchant", [validity]) | ("enchant", [validity, _]) => (
                Some(keyword.clone()),
                Some(AbilityDefinition {
                    kind: AbilityKind::Spell,
                    costs: Vec::new(),
                    event: None,
                    condition: None,
                    timing: None,
                    effect: expression_call(
                        Operation::Attach,
                        vec![
                            expression_call(Operation::Source, vec![]),
                            valid_target_selector(validity).map_err(|diagnostic| {
                                (line.line, diagnostic.code, diagnostic.message)
                            })?,
                        ],
                    ),
                    mana_ability: false,
                }),
            ),
            _ => {
                return Err((
                    line.line,
                    "UNSUPPORTED_KEYWORD".to_string(),
                    format!("parameterized keyword `{name}` requires a typed mapper"),
                ));
            }
        };
        if let Some(keyword) = keyword_id {
            if !is_known_keyword(&keyword) {
                return Err((
                    line.line,
                    "UNSUPPORTED_KEYWORD".to_string(),
                    format!("keyword `{name}` is outside the closed translation pack"),
                ));
            }
            translated.ids.push(keyword);
        }
        translated.abilities.extend(ability);
    }
    translated.ids.sort();
    translated.ids.dedup();
    Ok(translated)
}

fn translate_crew_keyword(
    line: usize,
    name: &str,
    amount: &str,
    timing: Option<Expression>,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let amount = amount.parse::<i64>().map_err(|_| {
        (
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!("keyword `{name}` crew amount `{amount}` is not a positive integer"),
        )
    })?;
    if amount <= 0 {
        return Err((
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!("keyword `{name}` crew amount `{amount}` is not a positive integer"),
        ));
    }
    Ok(AbilityDefinition {
        kind: AbilityKind::Activated,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing,
        effect: expression_call(
            Operation::Crew,
            vec![
                expression_call(Operation::Source, vec![]),
                Expression::Integer(amount),
            ],
        ),
        mana_ability: false,
    })
}

fn translate_etb_extra_counter(
    line: usize,
    affected: &str,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let affected = closed_etb_extra_counter_selector(line, affected)?;
    let triggered = expression_call(Operation::Triggered, vec![]);
    Ok(AbilityDefinition {
        kind: AbilityKind::Replacement,
        costs: Vec::new(),
        event: Some(expression_call(Operation::EventEnters, vec![affected])),
        condition: None,
        timing: None,
        effect: expression_call(
            Operation::ReplaceEvent,
            vec![
                triggered.clone(),
                expression_call(
                    Operation::AddCounter,
                    vec![
                        triggered,
                        Expression::Text("plus1_plus1".to_string()),
                        Expression::Integer(1),
                    ],
                ),
            ],
        ),
        mana_ability: false,
    })
}

fn translate_suspend_keyword(
    line: usize,
    time: &str,
    cost: &String,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let time = time.parse::<i64>().map_err(|_| {
        (
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!("suspend time `{time}` is not a positive integer"),
        )
    })?;
    if time <= 0 {
        return Err((
            line,
            "UNSUPPORTED_VALUE".to_string(),
            "suspend time must be positive".to_string(),
        ));
    }
    let costs = parse_simple_cost(Some(cost))
        .map_err(|diagnostic| (line, diagnostic.code, diagnostic.message))?;
    let mut arguments = vec![
        expression_call(Operation::Source, vec![]),
        Expression::Integer(time),
    ];
    arguments.extend(costs);
    Ok(AbilityDefinition {
        kind: AbilityKind::Static,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing: None,
        effect: expression_call(Operation::Suspend, arguments),
        mana_ability: false,
    })
}

fn translate_costed_keyword_rule(
    line: usize,
    cost: &String,
    operation: Operation,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let costs = parse_simple_cost(Some(cost))
        .map_err(|diagnostic| (line, diagnostic.code, diagnostic.message))?;
    if costs.is_empty() {
        return Err((
            line,
            "MISSING_COST".to_string(),
            format!("{} requires a cost", operation.as_str()),
        ));
    }
    let mut arguments = vec![expression_call(Operation::Source, vec![])];
    arguments.extend(costs);
    Ok(AbilityDefinition {
        kind: AbilityKind::Static,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing: None,
        effect: expression_call(operation, arguments),
        mana_ability: false,
    })
}

fn translate_marker_keyword(operation: Operation) -> AbilityDefinition {
    AbilityDefinition {
        kind: AbilityKind::Static,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing: None,
        effect: expression_call(operation, vec![expression_call(Operation::Source, vec![])]),
        mana_ability: false,
    }
}

fn translate_keyword_cost_options(
    line: usize,
    cost_options: &[String],
    operation: Operation,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let mut arguments = vec![expression_call(Operation::Source, vec![])];
    for cost in cost_options {
        let costs = parse_simple_cost(Some(cost))
            .map_err(|diagnostic| (line, diagnostic.code, diagnostic.message))?;
        if costs.is_empty() {
            return Err((
                line,
                "MISSING_COST".to_string(),
                format!("{} requires non-empty cost options", operation.as_str()),
            ));
        }
        arguments.push(expression_call(Operation::CostBundle, costs));
    }
    Ok(AbilityDefinition {
        kind: AbilityKind::Static,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing: None,
        effect: expression_call(operation, arguments),
        mana_ability: false,
    })
}

fn translate_numeric_keyword_rule(
    line: usize,
    amount: &str,
    operation: Operation,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let amount = amount.parse::<i64>().map_err(|_| {
        (
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!(
                "{} amount `{amount}` is not a positive integer",
                operation.as_str()
            ),
        )
    })?;
    if amount <= 0 {
        return Err((
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!("{} amount must be positive", operation.as_str()),
        ));
    }
    Ok(AbilityDefinition {
        kind: AbilityKind::Static,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing: None,
        effect: expression_call(
            operation,
            vec![
                expression_call(Operation::Source, vec![]),
                Expression::Integer(amount),
            ],
        ),
        mana_ability: false,
    })
}

fn translate_nonnegative_numeric_keyword_rule(
    line: usize,
    amount: &str,
    operation: Operation,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let amount = amount.parse::<i64>().map_err(|_| {
        (
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!(
                "{} amount `{amount}` is not a nonnegative integer",
                operation.as_str()
            ),
        )
    })?;
    if amount < 0 {
        return Err((
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!("{} amount must be nonnegative", operation.as_str()),
        ));
    }
    Ok(AbilityDefinition {
        kind: AbilityKind::Static,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing: None,
        effect: expression_call(
            operation,
            vec![
                expression_call(Operation::Source, vec![]),
                Expression::Integer(amount),
            ],
        ),
        mana_ability: false,
    })
}

fn translate_activated_keyword_rule(
    line: usize,
    cost: &String,
    operation: Operation,
    timing: Option<Expression>,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let costs = parse_simple_cost(Some(cost))
        .map_err(|diagnostic| (line, diagnostic.code, diagnostic.message))?;
    if costs.is_empty() {
        return Err((
            line,
            "MISSING_COST".to_string(),
            format!("{} requires a cost", operation.as_str()),
        ));
    }
    Ok(AbilityDefinition {
        kind: AbilityKind::Activated,
        costs,
        event: None,
        condition: None,
        timing,
        effect: expression_call(operation, vec![expression_call(Operation::Source, vec![])]),
        mana_ability: false,
    })
}

fn translate_numeric_activated_keyword_rule(
    line: usize,
    amount: &str,
    operation: Operation,
    timing: Option<Expression>,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let amount = amount.parse::<i64>().map_err(|_| {
        (
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!(
                "{} amount `{amount}` is not a positive integer",
                operation.as_str()
            ),
        )
    })?;
    if amount <= 0 {
        return Err((
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!("{} amount must be positive", operation.as_str()),
        ));
    }
    Ok(AbilityDefinition {
        kind: AbilityKind::Activated,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing,
        effect: expression_call(
            operation,
            vec![
                expression_call(Operation::Source, vec![]),
                Expression::Integer(amount),
            ],
        ),
        mana_ability: false,
    })
}

fn translate_prototype_keyword(
    line: usize,
    cost: &String,
    power: &str,
    toughness: &str,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let costs = parse_simple_cost(Some(cost))
        .map_err(|diagnostic| (line, diagnostic.code, diagnostic.message))?;
    if costs.is_empty() {
        return Err((
            line,
            "MISSING_COST".to_string(),
            "prototype requires a cost".to_string(),
        ));
    }
    let parse_stat = |value: &str, field: &str| {
        value.parse::<i64>().map_err(|_| {
            (
                line,
                "UNSUPPORTED_VALUE".to_string(),
                format!("prototype {field} `{value}` is not a nonnegative integer"),
            )
        })
    };
    let power = parse_stat(power, "power")?;
    let toughness = parse_stat(toughness, "toughness")?;
    if power < 0 || toughness < 0 {
        return Err((
            line,
            "UNSUPPORTED_VALUE".to_string(),
            "prototype power and toughness must be nonnegative".to_string(),
        ));
    }
    Ok(AbilityDefinition {
        kind: AbilityKind::Static,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing: None,
        effect: expression_call(
            Operation::Prototype,
            vec![
                expression_call(Operation::Source, vec![]),
                expression_call(Operation::CostBundle, costs),
                Expression::Integer(power),
                Expression::Integer(toughness),
            ],
        ),
        mana_ability: false,
    })
}

fn translate_splice_keyword(
    line: usize,
    validity: &str,
    cost: &String,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let costs = parse_simple_cost(Some(cost))
        .map_err(|diagnostic| (line, diagnostic.code, diagnostic.message))?;
    if costs.is_empty() || validity.trim().is_empty() {
        return Err((
            line,
            "MISSING_PARAMETER".to_string(),
            "splice requires a validity and cost".to_string(),
        ));
    }
    let mut arguments = vec![
        expression_call(Operation::Source, vec![]),
        Expression::Text(validity.to_string()),
    ];
    arguments.extend(costs);
    Ok(AbilityDefinition {
        kind: AbilityKind::Static,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing: None,
        effect: expression_call(Operation::SpliceCost, arguments),
        mana_ability: false,
    })
}

fn translate_awaken_keyword(
    line: usize,
    amount: &str,
    cost: &String,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let amount = amount.parse::<i64>().map_err(|_| {
        (
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!("awaken amount `{amount}` is not a positive integer"),
        )
    })?;
    if amount <= 0 {
        return Err((
            line,
            "UNSUPPORTED_VALUE".to_string(),
            "awaken amount must be positive".to_string(),
        ));
    }
    let costs = parse_simple_cost(Some(cost))
        .map_err(|diagnostic| (line, diagnostic.code, diagnostic.message))?;
    if costs.is_empty() {
        return Err((
            line,
            "MISSING_COST".to_string(),
            "awaken requires a cost".to_string(),
        ));
    }
    let mut arguments = vec![
        expression_call(Operation::Source, vec![]),
        Expression::Integer(amount),
    ];
    arguments.extend(costs);
    Ok(AbilityDefinition {
        kind: AbilityKind::Static,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing: None,
        effect: expression_call(Operation::AwakenCost, arguments),
        mana_ability: false,
    })
}

fn translate_affinity_keyword(
    line: usize,
    validity: &str,
) -> Result<AbilityDefinition, (usize, String, String)> {
    if validity == "Historic"
        || validity.is_empty()
        || !validity
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '.')
    {
        return Err((
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!("affinity validity `{validity}` has no closed selector"),
        ));
    }
    let controlled = if validity.contains('.') {
        format!("{validity}+YouCtrl")
    } else {
        format!("{validity}.YouCtrl")
    };
    let counted = affected_selector(&controlled)
        .map_err(|diagnostic| (line, diagnostic.code, diagnostic.message))?;
    Ok(AbilityDefinition {
        kind: AbilityKind::Static,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing: None,
        effect: expression_call(Operation::AffinityCostReduction, vec![counted]),
        mana_ability: false,
    })
}

fn closed_etb_extra_counter_selector(
    line: usize,
    value: &str,
) -> Result<Expression, (usize, String, String)> {
    if !matches!(
        value,
        "Creature.Other+YouCtrl"
            | "Creature.YouCtrl+Other"
            | "Creature.YouCtrl"
            | "Creature.Angel+YouCtrl+Other"
            | "Creature.Wizard+Other+YouCtrl"
            | "Creature.Beast+YouCtrl+Other"
            | "Creature.Legendary+YouCtrl+Other"
            | "Creature.Warrior+YouCtrl+Other"
            | "Creature.Dragon+YouCtrl"
            | "Creature.Rogue+Other+YouCtrl"
            | "Vampire.YouCtrl+Other"
            | "Creature.Wolf+YouCtrl"
            | "Creature.Werewolf+YouCtrl"
            | "Creature.Wolf+YouCtrl,Creature.Werewolf+YouCtrl"
            | "Creature.Colorless+YouCtrl"
            | "Creature.YouCtrl+Other+nonHuman"
            | "Creature.YouCtrl+Other,Vehicle.YouCtrl+Other"
            | "Planeswalker.YouCtrl"
    ) {
        return Err((
            line,
            "UNSUPPORTED_VALUE".to_string(),
            format!("ETB extra-counter selector `{value}` has no closed lowering"),
        ));
    }
    affected_selector(value).map_err(|diagnostic| (line, diagnostic.code, diagnostic.message))
}

fn translate_etb_counter(
    script: &LegacyScript,
    line: usize,
    name: &str,
    counter: &str,
    amount: &str,
    condition_svar: Option<&str>,
) -> Result<AbilityDefinition, (usize, String, String)> {
    if counter.trim().is_empty() {
        return Err((
            line,
            "UNSUPPORTED_KEYWORD".to_string(),
            format!("keyword `{name}` has an empty counter type"),
        ));
    }
    let amount = match amount.parse::<i64>() {
        Ok(value) if value > 0 => Expression::Integer(value),
        Ok(_) => {
            return Err((
                line,
                "UNSUPPORTED_KEYWORD".to_string(),
                format!("keyword `{name}` counter amount `{amount}` is not positive"),
            ));
        }
        Err(_) => {
            let context = MappingContext::from_script(script);
            resolve_value_svar(amount, &context)
                .map_err(|diagnostic| (line, diagnostic.code.to_string(), diagnostic.message))?
        }
    };
    let condition = condition_svar
        .map(|reference| {
            let context = MappingContext::from_script(script);
            resolve_value_svar(reference, &context)
                .map(|value| {
                    expression_call(Operation::GreaterThan, vec![value, Expression::Integer(0)])
                })
                .map_err(|diagnostic| (line, diagnostic.code, diagnostic.message))
        })
        .transpose()?;
    Ok(AbilityDefinition {
        kind: AbilityKind::Replacement,
        costs: Vec::new(),
        event: Some(expression_call(
            Operation::EventEnters,
            vec![expression_call(Operation::Source, vec![])],
        )),
        condition,
        timing: None,
        effect: expression_call(
            Operation::AddCounter,
            vec![
                expression_call(Operation::Source, vec![]),
                Expression::Text(counter.to_ascii_lowercase()),
                amount,
            ],
        ),
        mana_ability: false,
    })
}

fn expression_call(operation: Operation, arguments: Vec<Expression>) -> Expression {
    Expression::Call {
        operation,
        arguments,
    }
}

fn alternative_cost_ability(
    cost: &str,
    line: usize,
) -> Result<AbilityDefinition, (usize, String, String)> {
    let cost = cost.to_string();
    let costs = parse_simple_cost(Some(&cost))
        .map_err(|diagnostic| (line, diagnostic.code, diagnostic.message))?;
    if costs.is_empty() {
        return Err((
            line,
            "UNSUPPORTED_KEYWORD".to_string(),
            "alternative keyword cost is empty".to_string(),
        ));
    }
    let mut arguments = vec![expression_call(Operation::Source, vec![])];
    arguments.extend(costs);
    Ok(AbilityDefinition {
        kind: AbilityKind::Static,
        costs: Vec::new(),
        event: None,
        condition: None,
        timing: None,
        effect: expression_call(
            Operation::Continuous,
            vec![
                expression_call(Operation::Source, vec![]),
                expression_call(Operation::AlternateCost, arguments),
            ],
        ),
        mana_ability: false,
    })
}

fn catalog_identities(catalog: &CardCatalog) -> CatalogIdentities {
    let mut identities = CatalogIdentities::default();
    for identity in &catalog.identities {
        if !matches!(
            identity.classification,
            CardClassification::VerifiedPlayable | CardClassification::UnverifiedPlayable
        ) {
            continue;
        }
        let value = CatalogIdentity {
            id: identity.id.clone(),
            name: identity.name.clone(),
            layout: identity.layout,
            face_names: identity.face_names.clone(),
        };
        match identities.by_name.entry(identity.name.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Some(value.clone()));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                entry.insert(None);
            }
        }
        match identities.by_faces.entry(identity.face_names.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Some(value));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                entry.insert(None);
            }
        }
    }
    identities
}

fn catalog_identity<'a>(
    identities: &'a CatalogIdentities,
    face_names: &[String],
) -> Result<&'a CatalogIdentity, (usize, String, String)> {
    let identity = if face_names.len() == 1 {
        identities.by_name.get(&face_names[0])
    } else {
        identities.by_faces.get(face_names)
    }
    .ok_or_else(|| {
        (
            1,
            "MISSING_CATALOG_IDENTITY".to_string(),
            format!(
                "catalog has no exact identity for faces `{}`",
                face_names.join(" // ")
            ),
        )
    })?;
    identity.as_ref().ok_or_else(|| {
        (
            1,
            "AMBIGUOUS_CATALOG_IDENTITY".to_string(),
            format!(
                "catalog faces `{}` resolve to multiple identities",
                face_names.join(" // ")
            ),
        )
    })
}

fn candidate_names(script: &LegacyScript) -> Vec<String> {
    let mut names = script
        .lines
        .iter()
        .filter_map(|line| match &line.kind {
            LegacyLineKind::Property { key, value } if key == "Name" => Some(value.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn read_priority_tiers(path: &Path) -> Result<Vec<PriorityTier>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let mut tiers = Vec::new();
    let mut current: Option<PriorityTier> = None;
    let mut seen_names = std::collections::BTreeSet::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix("# TIER ") {
            if let Some(tier) = current.take() {
                tiers.push(tier);
            }
            let (tier, label) = header.split_once(" - ").ok_or_else(|| {
                format!(
                    "{}:{} priority tier header requires `# TIER N - label`",
                    path.display(),
                    index + 1
                )
            })?;
            let tier = tier.parse::<u8>().map_err(|_| {
                format!(
                    "{}:{} priority tier `{tier}` is not an integer",
                    path.display(),
                    index + 1
                )
            })?;
            current = Some(PriorityTier {
                tier,
                label: label.to_string(),
                names: Vec::new(),
            });
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let tier = current.as_mut().ok_or_else(|| {
            format!(
                "{}:{} priority names appear before a tier header",
                path.display(),
                index + 1
            )
        })?;
        for name in line
            .split('|')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if !seen_names.insert(name.to_string()) {
                return Err(format!(
                    "{}:{} duplicate priority card `{name}`",
                    path.display(),
                    index + 1
                ));
            }
            tier.names.push(name.to_string());
        }
    }
    if let Some(tier) = current {
        tiers.push(tier);
    }
    if tiers.is_empty() || tiers.iter().any(|tier| tier.names.is_empty()) {
        return Err(format!(
            "{} must contain at least one nonempty priority tier",
            path.display()
        ));
    }
    tiers.sort_by_key(|tier| tier.tier);
    Ok(tiers)
}

fn build_priority_report(
    source: &Path,
    tiers: &[PriorityTier],
    catalog: &CardCatalog,
    outcomes: &[TranslationOutcome],
) -> PriorityCoverageReport {
    let mut tier_reports = Vec::new();
    for tier in tiers {
        let mut cards = Vec::new();
        for requested_name in &tier.names {
            cards.push(priority_card_result(requested_name, catalog, outcomes));
        }
        let catalog_resolved = cards
            .iter()
            .filter(|card| card.catalog_name.is_some())
            .count();
        let emitted = cards.iter().filter(|card| card.status == "emitted").count();
        tier_reports.push(PriorityTierReport {
            tier: tier.tier,
            label: tier.label.clone(),
            requested: cards.len(),
            catalog_resolved,
            emitted,
            emitted_percent: percent(emitted, cards.len()),
            cards,
        });
    }
    let total_requested = tier_reports.iter().map(|tier| tier.requested).sum();
    let catalog_resolved = tier_reports.iter().map(|tier| tier.catalog_resolved).sum();
    let emitted = tier_reports.iter().map(|tier| tier.emitted).sum();
    PriorityCoverageReport {
        schema_version: 1,
        source_path: crate::repository_relative(source),
        total_requested,
        catalog_resolved,
        emitted,
        emitted_percent: percent(emitted, total_requested),
        tiers: tier_reports,
    }
}

fn priority_card_result(
    requested_name: &str,
    catalog: &CardCatalog,
    outcomes: &[TranslationOutcome],
) -> PriorityCardResult {
    let playable = catalog
        .identities
        .iter()
        .filter(|identity| {
            matches!(
                identity.classification,
                CardClassification::VerifiedPlayable | CardClassification::UnverifiedPlayable
            )
        })
        .collect::<Vec<_>>();
    let exact_matches = playable
        .iter()
        .copied()
        .filter(|identity| identity.name == requested_name)
        .collect::<Vec<_>>();
    let matches = if exact_matches.is_empty() {
        playable
            .into_iter()
            .filter(|identity| {
                identity
                    .face_names
                    .iter()
                    .any(|name| name == requested_name)
            })
            .collect::<Vec<_>>()
    } else {
        exact_matches
    };
    let Some(identity) = matches.first().copied() else {
        return PriorityCardResult {
            requested_name: requested_name.to_string(),
            catalog_name: None,
            status: "catalog_missing".to_string(),
            path: None,
            code: Some("PRIORITY_CATALOG_MISSING".to_string()),
            message: Some("no playable catalog identity matches this requested name".to_string()),
        };
    };
    if matches.len() > 1 {
        return PriorityCardResult {
            requested_name: requested_name.to_string(),
            catalog_name: None,
            status: "catalog_ambiguous".to_string(),
            path: None,
            code: Some("PRIORITY_CATALOG_AMBIGUOUS".to_string()),
            message: Some(format!(
                "{} playable catalog identities match this requested name",
                matches.len()
            )),
        };
    }
    let aliases = identity
        .face_names
        .iter()
        .chain(std::iter::once(&identity.name))
        .collect::<Vec<_>>();
    let candidates = outcomes
        .iter()
        .filter(|outcome| {
            outcome_aliases(outcome)
                .iter()
                .any(|alias| aliases.contains(&alias))
        })
        .collect::<Vec<_>>();
    if let Some(outcome) = candidates
        .iter()
        .find(|outcome| matches!(outcome, TranslationOutcome::Emitted { .. }))
    {
        return PriorityCardResult {
            requested_name: requested_name.to_string(),
            catalog_name: Some(identity.name.clone()),
            status: "emitted".to_string(),
            path: Some(outcome_path(outcome).to_string()),
            code: None,
            message: None,
        };
    }
    if let Some(TranslationOutcome::Quarantined { failure, .. }) = candidates.first().copied() {
        return PriorityCardResult {
            requested_name: requested_name.to_string(),
            catalog_name: Some(identity.name.clone()),
            status: "quarantined".to_string(),
            path: Some(failure.path.clone()),
            code: Some(failure.code.clone()),
            message: Some(failure.message.clone()),
        };
    }
    PriorityCardResult {
        requested_name: requested_name.to_string(),
        catalog_name: Some(identity.name.clone()),
        status: "legacy_missing".to_string(),
        path: None,
        code: Some("PRIORITY_LEGACY_MISSING".to_string()),
        message: Some("playable catalog identity has no matching legacy script".to_string()),
    }
}

fn percent(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 * 100.0 / denominator as f64
    }
}

fn face_lines(script: &LegacyScript) -> Vec<Vec<&LegacyLine>> {
    let mut faces = vec![Vec::new()];
    for line in &script.lines {
        if matches!(line.kind, LegacyLineKind::Alternate) {
            faces.push(Vec::new());
        } else if let Some(face) = faces.last_mut() {
            face.push(line);
        }
    }
    faces
}

fn face_index_for_line(faces: &[Vec<&LegacyLine>], line: usize) -> Option<usize> {
    faces.iter().position(|face| {
        face.first().is_some_and(|first| first.line <= line)
            && face.last().is_some_and(|last| line <= last.line)
    })
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
        TranslationOutcome::Quarantined { failure, .. } => &failure.path,
    }
}

fn outcome_aliases(outcome: &TranslationOutcome) -> &[String] {
    match outcome {
        TranslationOutcome::Emitted { aliases, .. }
        | TranslationOutcome::Quarantined { aliases, .. } => aliases,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_priority_report, collect_script_keyword_blockers, expression_call, face_lines,
        legacy_mana_cost, legacy_type_line, percent, read_priority_tiers, translate_keywords,
        translate_one_inner, translation_fingerprint, CatalogIdentities, CatalogIdentity,
        PriorityTier, TranslationFailure, TranslationOutcome,
    };
    use forge_carddef::{
        AbilityKind, CardCatalog, CardClassification, CardLayout, Expression, IdentityRecord,
        Operation, OracleId, SourceProvenance,
    };
    use std::{fs, path::PathBuf};

    fn fixture_identity(
        id: &str,
        name: &str,
        face_names: &[&str],
        classification: CardClassification,
    ) -> IdentityRecord {
        IdentityRecord {
            id: OracleId::parse(id).unwrap_or_else(|| panic!("fixture id should parse")),
            name: name.to_string(),
            layout: if face_names.len() > 1 {
                CardLayout::ModalDfc
            } else {
                CardLayout::Normal
            },
            face_names: face_names.iter().map(|name| (*name).to_string()).collect(),
            classification,
        }
    }

    fn fixture_catalog(identities: Vec<IdentityRecord>) -> CardCatalog {
        CardCatalog {
            schema_version: 1,
            provenance: SourceProvenance {
                source: "fixture".to_string(),
                source_path: "fixture.json".to_string(),
                source_updated_at: "2026-07-10".to_string(),
                source_sha256: "fixture".to_string(),
                generator: "fixture".to_string(),
            },
            identities,
            printings: Vec::new(),
        }
    }

    #[test]
    fn normalizes_closed_characteristics() {
        assert_eq!(legacy_mana_cost("1 G"), Ok("{1}{G}".to_string()));
        assert_eq!(
            legacy_type_line("Legendary Creature Elf Druid"),
            Ok("Legendary Creature \u{2014} Elf Druid".to_string())
        );
    }

    #[test]
    fn owner_priority_fixture_has_three_unique_complete_tiers() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/coverage_priority.txt");
        let tiers = read_priority_tiers(&path)
            .unwrap_or_else(|error| panic!("priority fixture should parse: {error}"));
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].names.len(), 58);
        assert_eq!(tiers[1].names.len(), 161);
        assert_eq!(tiers[2].names.len(), 146);
        assert_eq!(
            tiers.iter().map(|tier| tier.names.len()).sum::<usize>(),
            365
        );
    }

    #[test]
    fn priority_parser_rejects_malformed_empty_and_duplicate_tiers() {
        let root = std::env::temp_dir().join(format!(
            "forge-porttools-priority-parser-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("could not create priority fixture: {error}"));
        let path = root.join("priority.txt");

        for (source, expected) in [
            ("Card Before Tier\n", "before a tier header"),
            ("# TIER 0\nCard\n", "requires `# TIER N - label`"),
            ("# TIER X - Invalid\nCard\n", "is not an integer"),
            ("# TIER 0 - Empty\n", "at least one nonempty priority tier"),
            (
                "# TIER 0 - Duplicate\nCard|Card\n",
                "duplicate priority card",
            ),
        ] {
            fs::write(&path, source)
                .unwrap_or_else(|error| panic!("could not write priority fixture: {error}"));
            let Err(error) = read_priority_tiers(&path) else {
                panic!("malformed priority fixture should be rejected");
            };
            assert!(error.contains(expected), "unexpected error: {error}");
        }

        fs::write(
            &path,
            "# ignored\n# TIER 2 - Later\nSecond\n# TIER 0 - First\nFirst|Another\n",
        )
        .unwrap_or_else(|error| panic!("could not write priority fixture: {error}"));
        let tiers = read_priority_tiers(&path)
            .unwrap_or_else(|error| panic!("valid priority fixture should parse: {error}"));
        assert_eq!(
            tiers.iter().map(|tier| tier.tier).collect::<Vec<_>>(),
            [0, 2]
        );
        assert_eq!(tiers[0].names, ["First", "Another"]);

        fs::remove_dir_all(&root)
            .unwrap_or_else(|error| panic!("could not remove priority fixture: {error}"));
    }

    #[test]
    fn priority_report_prefers_exact_names_and_records_every_status() {
        let catalog = fixture_catalog(vec![
            fixture_identity(
                "fixture-front",
                "Front",
                &["Front"],
                CardClassification::UnverifiedPlayable,
            ),
            fixture_identity(
                "fixture-modal",
                "Front // Back",
                &["Front", "Back"],
                CardClassification::UnverifiedPlayable,
            ),
            fixture_identity(
                "fixture-blocked",
                "Blocked",
                &["Blocked"],
                CardClassification::UnverifiedPlayable,
            ),
            fixture_identity(
                "fixture-no-legacy",
                "No Legacy",
                &["No Legacy"],
                CardClassification::VerifiedPlayable,
            ),
            fixture_identity(
                "fixture-shared-one",
                "One // Shared",
                &["One", "Shared"],
                CardClassification::UnverifiedPlayable,
            ),
            fixture_identity(
                "fixture-shared-two",
                "Two // Shared",
                &["Two", "Shared"],
                CardClassification::UnverifiedPlayable,
            ),
            fixture_identity(
                "fixture-catalog-only",
                "Catalog Only",
                &["Catalog Only"],
                CardClassification::CatalogOnly("fixture".to_string()),
            ),
        ]);
        let tiers = vec![PriorityTier {
            tier: 0,
            label: "Fixture".to_string(),
            names: [
                "Front",
                "Back",
                "Blocked",
                "No Legacy",
                "Shared",
                "Catalog Only",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }];
        let outcomes = vec![
            TranslationOutcome::Emitted {
                relative: "f/front.txt".to_string(),
                source: "front".to_string(),
                aliases: vec!["Front".to_string()],
            },
            TranslationOutcome::Emitted {
                relative: "f/modal.txt".to_string(),
                source: "modal".to_string(),
                aliases: vec!["Front // Back".to_string(), "Back".to_string()],
            },
            TranslationOutcome::Quarantined {
                failure: TranslationFailure {
                    path: "b/blocked.txt".to_string(),
                    line: 7,
                    code: "UNSUPPORTED_VALUE".to_string(),
                    message: "fixture blocker".to_string(),
                },
                aliases: vec!["Blocked".to_string()],
            },
        ];

        let report = build_priority_report(
            std::path::Path::new("assets/priority.txt"),
            &tiers,
            &catalog,
            &outcomes,
        );
        assert_eq!(report.total_requested, 6);
        assert_eq!(report.catalog_resolved, 4);
        assert_eq!(report.emitted, 2);
        assert_eq!(report.emitted_percent, 100.0 / 3.0);
        assert_eq!(report.tiers[0].catalog_resolved, 4);
        assert_eq!(report.tiers[0].emitted_percent, 100.0 / 3.0);

        let cards = &report.tiers[0].cards;
        assert_eq!(cards[0].catalog_name.as_deref(), Some("Front"));
        assert_eq!(cards[0].status, "emitted");
        assert_eq!(cards[1].catalog_name.as_deref(), Some("Front // Back"));
        assert_eq!(cards[1].status, "emitted");
        assert_eq!(cards[2].status, "quarantined");
        assert_eq!(cards[2].code.as_deref(), Some("UNSUPPORTED_VALUE"));
        assert_eq!(cards[3].status, "legacy_missing");
        assert_eq!(cards[4].status, "catalog_ambiguous");
        assert_eq!(cards[5].status, "catalog_missing");
        assert_eq!(percent(0, 0), 0.0);
    }

    #[test]
    fn output_fingerprint_is_deterministic_and_content_sensitive() {
        let first = vec![
            TranslationOutcome::Emitted {
                relative: "a/alpha.txt".to_string(),
                source: "card alpha".to_string(),
                aliases: Vec::new(),
            },
            TranslationOutcome::Quarantined {
                failure: TranslationFailure {
                    path: "b/blocked.txt".to_string(),
                    line: 1,
                    code: "FIXTURE".to_string(),
                    message: "blocked".to_string(),
                },
                aliases: Vec::new(),
            },
            TranslationOutcome::Emitted {
                relative: "z/zeta.txt".to_string(),
                source: "card zeta".to_string(),
                aliases: Vec::new(),
            },
        ];
        let same = vec![
            TranslationOutcome::Emitted {
                relative: "a/alpha.txt".to_string(),
                source: "card alpha".to_string(),
                aliases: vec!["ignored alias".to_string()],
            },
            TranslationOutcome::Emitted {
                relative: "z/zeta.txt".to_string(),
                source: "card zeta".to_string(),
                aliases: Vec::new(),
            },
        ];
        let changed = vec![TranslationOutcome::Emitted {
            relative: "a/alpha.txt".to_string(),
            source: "card alpha changed".to_string(),
            aliases: Vec::new(),
        }];

        assert_eq!(
            translation_fingerprint(&first),
            translation_fingerprint(&same)
        );
        assert_ne!(
            translation_fingerprint(&first),
            translation_fingerprint(&changed)
        );
        assert_eq!(translation_fingerprint(&first).len(), 32);
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
        let mut identities = CatalogIdentities::default();
        identities.by_name.insert(
            "Grizzly Bears".to_string(),
            Some(CatalogIdentity {
                id: OracleId::parse("fixture-grizzly")
                    .unwrap_or_else(|| panic!("fixture id should parse")),
                name: "Grizzly Bears".to_string(),
                layout: CardLayout::Normal,
                face_names: vec!["Grizzly Bears".to_string()],
            }),
        );
        let emitted = translate_one_inner(&path, "g/grizzly_bears.txt", &identities)
            .unwrap_or_else(|error| panic!("fixture should translate: {error:?}"));
        assert!(emitted.contains("card \"Grizzly Bears\""));
        assert!(emitted.contains("types: \"Creature "));
        fs::remove_dir_all(&root)
            .unwrap_or_else(|error| panic!("could not remove translator fixture: {error}"));
    }

    #[test]
    fn emits_and_roundtrips_ordered_transform_faces() {
        let root = std::env::temp_dir().join(format!(
            "forge-porttools-transform-translation-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("could not create translator fixture: {error}"));
        let path = root.join("fixture_transform.txt");
        fs::write(
            &path,
            concat!(
                "Name:Front Face\n",
                "ManaCost:1 U\n",
                "Types:Creature Human\n",
                "PT:1/1\n",
                "Oracle:\n",
                "ALTERNATE\n",
                "Name:Back Face\n",
                "ManaCost:no cost\n",
                "Types:Creature Spirit\n",
                "PT:2/2\n",
                "K:Flying\n",
                "Oracle:Flying\n",
            ),
        )
        .unwrap_or_else(|error| panic!("could not write translator fixture: {error}"));
        let face_names = vec!["Front Face".to_string(), "Back Face".to_string()];
        let mut identities = CatalogIdentities::default();
        identities.by_faces.insert(
            face_names,
            Some(CatalogIdentity {
                id: OracleId::parse("fixture-transform")
                    .unwrap_or_else(|| panic!("fixture id should parse")),
                name: "Front Face // Back Face".to_string(),
                layout: CardLayout::Transform,
                face_names: vec!["Front Face".to_string(), "Back Face".to_string()],
            }),
        );
        let emitted = translate_one_inner(&path, "f/fixture_transform.txt", &identities)
            .unwrap_or_else(|error| panic!("fixture should translate: {error:?}"));
        assert!(emitted.contains("layout: transform"));
        assert!(emitted.contains("face \"Front Face\""));
        assert!(emitted.contains("face \"Back Face\""));
        fs::remove_dir_all(&root)
            .unwrap_or_else(|error| panic!("could not remove translator fixture: {error}"));
    }

    #[test]
    fn desugars_parameterized_attachment_keywords() {
        let root = std::env::temp_dir().join(format!(
            "forge-porttools-keyword-translation-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("could not create translator fixture: {error}"));
        let path = root.join("fixture_equipment.txt");
        fs::write(
            &path,
            concat!(
                "Name:Fixture Equipment\n",
                "ManaCost:2\n",
                "Types:Artifact Equipment\n",
                "K:Equip:3\n",
                "Oracle:Equip {3}\n",
            ),
        )
        .unwrap_or_else(|error| panic!("could not write translator fixture: {error}"));
        let mut identities = CatalogIdentities::default();
        identities.by_name.insert(
            "Fixture Equipment".to_string(),
            Some(CatalogIdentity {
                id: OracleId::parse("fixture-equipment")
                    .unwrap_or_else(|| panic!("fixture id should parse")),
                name: "Fixture Equipment".to_string(),
                layout: CardLayout::Normal,
                face_names: vec!["Fixture Equipment".to_string()],
            }),
        );
        let emitted = translate_one_inner(&path, "f/fixture_equipment.txt", &identities)
            .unwrap_or_else(|error| panic!("fixture should translate: {error:?}"));
        assert!(emitted.contains("keywords: [equip]"));
        assert!(emitted.contains("costs: [mana_cost(\"{3}\")]"));
        assert!(emitted.contains("effect: attach(source(), target(permanents("));
        fs::remove_dir_all(&root)
            .unwrap_or_else(|error| panic!("could not remove translator fixture: {error}"));
    }

    #[test]
    fn desugars_fixed_enters_with_counters_pseudo_keyword() {
        let script = crate::legacy::parse_legacy_script("fixture.txt", "K:etbCounter:P1P1:2\n")
            .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("keyword should translate: {error:?}"));
        assert!(translated.ids.is_empty());
        assert_eq!(translated.abilities.len(), 1);
        assert_eq!(
            translated.abilities[0].kind,
            forge_carddef::AbilityKind::Replacement
        );
    }

    #[test]
    fn desugars_closed_dynamic_etb_counter_and_landwalk_keywords() {
        let script = crate::legacy::parse_legacy_script(
            "fixture.txt",
            concat!(
                "K:etbCounter:P1P1:X\n",
                "SVar:X:Count$xPaid\n",
                "K:Landwalk:Swamp\n",
                "K:Landwalk:Land.Legendary\n",
            ),
        )
        .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("closed keyword fixture should translate: {error:?}"));
        assert_eq!(translated.ids, vec!["legendarylandwalk", "swampwalk"]);
        assert!(matches!(
            &translated.abilities[0].effect,
            Expression::Call {
                operation: Operation::AddCounter,
                arguments,
            } if matches!(
                arguments.get(2),
                Some(Expression::Call {
                    operation: Operation::PaidX,
                    ..
                })
            )
        ));

        let script = crate::legacy::parse_legacy_script(
            "fixture.txt",
            concat!(
                "K:etbCounter:P1P1:X:no Condition:CARDNAME enters with X counters.\n",
                "SVar:X:Count$ValidGraveyard Instant,Sorcery\n",
            ),
        )
        .unwrap_or_else(|error| panic!("graveyard counter fixture should parse: {error}"));
        let faces = face_lines(&script);
        translate_keywords(&script, &faces[0]).unwrap_or_else(|error| {
            panic!("graveyard counter fixture should translate: {error:?}")
        });

        let script = crate::legacy::parse_legacy_script(
            "fixture.txt",
            concat!(
                "K:etbCounter:P1P1:X:no Condition:CARDNAME enters with one counter for each other Ooze.\n",
                "SVar:X:Count$LastStateBattlefield Ooze.YouCtrl+Other\n",
            ),
        )
        .unwrap_or_else(|error| panic!("last-state counter fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0]).unwrap_or_else(|error| {
            panic!("last-state counter fixture should translate: {error:?}")
        });
        assert!(matches!(
            &translated.abilities[0].effect,
            Expression::Call {
                operation: Operation::AddCounter,
                arguments,
            } if matches!(
                arguments.get(2),
                Some(Expression::Call {
                    operation: Operation::Count,
                    ..
                })
            )
        ));
    }

    #[test]
    fn desugars_closed_conditional_etb_counter_keyword() {
        let script = crate::legacy::parse_legacy_script(
            "fixture.txt",
            concat!(
                "K:etbCounter:P1P1:2:CheckSVar$ WasKicked:If kicked, this enters with counters.\n",
                "SVar:WasKicked:Count$Kicked.1.0\n",
            ),
        )
        .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("conditional keyword should translate: {error:?}"));
        assert!(matches!(
            translated.abilities[0].condition,
            Some(Expression::Call {
                operation: Operation::GreaterThan,
                ..
            })
        ));
    }

    #[test]
    fn rejects_open_etb_counter_and_landwalk_forms() {
        for keyword in [
            "K:etbCounter:P1P1:X:CheckSVar$ WasKicked",
            "K:Landwalk:Artifact",
        ] {
            let script = crate::legacy::parse_legacy_script("fixture.txt", keyword)
                .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
            let faces = face_lines(&script);
            let error = translate_keywords(&script, &faces[0])
                .err()
                .unwrap_or_else(|| panic!("open keyword must fail closed: {keyword}"));
            assert!(matches!(
                error.1.as_str(),
                "UNSUPPORTED_KEYWORD" | "UNSUPPORTED_VALUE"
            ));
        }
    }

    #[test]
    fn desugars_fixed_cost_cycling() {
        let script = crate::legacy::parse_legacy_script("fixture.txt", "K:Cycling:2\n")
            .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("keyword should translate: {error:?}"));
        assert_eq!(translated.ids, vec!["cycling"]);
        assert_eq!(translated.abilities.len(), 1);
        assert_eq!(translated.abilities[0].costs.len(), 2);
        assert_eq!(
            translated.abilities[0].kind,
            forge_carddef::AbilityKind::Activated
        );
    }

    #[test]
    fn desugars_closed_affinity_selectors() {
        for keyword in [
            "K:Affinity:Artifact",
            "K:Affinity:Creature.Artifact:artifact creature",
        ] {
            let script = crate::legacy::parse_legacy_script("fixture.txt", keyword)
                .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
            let faces = face_lines(&script);
            let translated = translate_keywords(&script, &faces[0])
                .unwrap_or_else(|error| panic!("affinity should translate: {error:?}"));
            assert_eq!(translated.ids, vec!["affinity"]);
            assert!(matches!(
                translated.abilities[0].effect,
                Expression::Call {
                    operation: Operation::AffinityCostReduction,
                    ..
                }
            ));
        }
    }

    #[test]
    fn desugars_fixed_cost_unearth() {
        let script = crate::legacy::parse_legacy_script("fixture.txt", "K:Unearth:2 B\n")
            .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("unearth should translate: {error:?}"));
        assert_eq!(translated.ids, vec!["unearth"]);
        assert_eq!(translated.abilities[0].costs.len(), 1);
        assert!(matches!(
            translated.abilities[0].effect,
            Expression::Call {
                operation: Operation::Unearth,
                ..
            }
        ));
        assert!(matches!(
            translated.abilities[0].timing,
            Some(Expression::Call {
                operation: Operation::TimingSorcery,
                ..
            })
        ));
    }

    #[test]
    fn desugars_fixed_cost_morph() {
        let script = crate::legacy::parse_legacy_script("fixture.txt", "K:Morph:2 U\n")
            .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("morph should translate: {error:?}"));
        assert_eq!(translated.ids, vec!["morph"]);
        assert_eq!(translated.abilities[0].costs.len(), 1);
        assert!(matches!(
            translated.abilities[0].effect,
            Expression::Call {
                operation: Operation::Morph,
                ..
            }
        ));
    }

    #[test]
    fn desugars_closed_ward_and_echo_costs() {
        for (keyword, operation) in [
            ("K:Ward:Discard<1/Card>", Operation::WardCost),
            ("K:Echo:2 B", Operation::EchoCost),
        ] {
            let script = crate::legacy::parse_legacy_script("fixture.txt", keyword)
                .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
            let faces = face_lines(&script);
            let translated = translate_keywords(&script, &faces[0])
                .unwrap_or_else(|error| panic!("costed keyword should translate: {error:?}"));
            assert!(matches!(
                translated.abilities[0].effect,
                Expression::Call {
                    operation: actual,
                    ..
                } if actual == operation
            ));
        }
    }

    #[test]
    fn desugars_cumulative_upkeep_and_suspend() {
        for (keyword, operation) in [
            (
                "K:Cumulative upkeep:PayLife<1>:Pay 1 life.",
                Operation::CumulativeUpkeepCost,
            ),
            ("K:Suspend:4:1 R", Operation::Suspend),
        ] {
            let script = crate::legacy::parse_legacy_script("fixture.txt", keyword)
                .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
            let faces = face_lines(&script);
            let translated = translate_keywords(&script, &faces[0])
                .unwrap_or_else(|error| panic!("timed keyword should translate: {error:?}"));
            assert!(matches!(
                translated.abilities[0].effect,
                Expression::Call {
                    operation: actual,
                    ..
                } if actual == operation
            ));
        }
    }

    #[test]
    fn desugars_repeatable_and_alternative_keyword_costs() {
        for (keyword, operation, option_count) in [
            ("K:Kicker:2 U:1 G", Operation::KickerCost, 2),
            ("K:Multikicker:1 G", Operation::MultikickerCost, 1),
            ("K:Buyback:Sac<1/Land>", Operation::BuybackCost, 1),
            (
                "K:AlternateAdditionalCost:Sac<1/Creature>:Discard<1/Card>",
                Operation::AlternateAdditionalCost,
                2,
            ),
        ] {
            let script = crate::legacy::parse_legacy_script("fixture.txt", keyword)
                .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
            let faces = face_lines(&script);
            let translated = translate_keywords(&script, &faces[0])
                .unwrap_or_else(|error| panic!("cost keyword should translate: {error:?}"));
            let Expression::Call {
                operation: actual,
                arguments,
            } = &translated.abilities[0].effect
            else {
                panic!("cost keyword should emit an operation call");
            };
            assert_eq!(*actual, operation);
            assert_eq!(arguments.len(), option_count + 1);
            assert!(arguments[1..].iter().all(|argument| matches!(
                argument,
                Expression::Call {
                    operation: Operation::CostBundle,
                    ..
                }
            )));
        }
    }

    #[test]
    fn desugars_fixed_protection_keywords() {
        for (keyword, protected_from) in [
            ("K:Protection from black", "black"),
            ("K:Protection from each color", "each_color"),
            ("K:Protection from everything", "everything"),
        ] {
            let script = crate::legacy::parse_legacy_script("fixture.txt", keyword)
                .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
            let faces = face_lines(&script);
            let translated = translate_keywords(&script, &faces[0])
                .unwrap_or_else(|error| panic!("protection should translate: {error:?}"));
            assert_eq!(
                translated.abilities[0].effect,
                expression_call(
                    Operation::ProtectionFrom,
                    vec![
                        expression_call(Operation::Source, vec![]),
                        Expression::Text(protected_from.to_string()),
                    ],
                )
            );
        }
    }

    #[test]
    fn desugars_closed_parameterized_keywords() {
        for (keyword, operation) in [
            ("K:Disguise:2 W", Operation::Disguise),
            ("K:Megamorph:3 G", Operation::Megamorph),
            ("K:Entwine:Sac<2/Land>", Operation::EntwineCost),
            ("K:Toxic:2", Operation::Toxic),
            ("K:Bushido:1", Operation::Bushido),
            ("K:Soulshift:4", Operation::Soulshift),
            ("K:Ninjutsu:1 U", Operation::Ninjutsu),
            ("K:Saddle:3", Operation::Saddle),
            ("K:Level up:2 W", Operation::LevelUp),
            ("K:Encore:3 B", Operation::Encore),
            ("K:Embalm:3 U", Operation::Embalm),
            ("K:Eternalize:4 W W", Operation::Eternalize),
            ("K:Plot:1 R", Operation::Plot),
            ("K:Warp:1 U", Operation::WarpCost),
            ("K:Sneak:1 B", Operation::SneakCost),
            ("K:Strive:2 R", Operation::StriveCost),
            ("K:Replicate:U", Operation::ReplicateCost),
            ("K:Miracle:1 W", Operation::MiracleCost),
            ("K:Offspring:2", Operation::OffspringCost),
            ("K:Firebending:2", Operation::Firebending),
            ("K:Vanishing:3", Operation::Vanishing),
            ("K:Fading:4", Operation::Fading),
            ("K:Prototype:2 W:1:1", Operation::Prototype),
            ("K:Station:8", Operation::Station),
            ("K:Renown:2", Operation::Renown),
            ("K:Bloodthirst:3", Operation::Bloodthirst),
            ("K:Fabricate:2", Operation::Fabricate),
            ("K:Modular:4", Operation::Modular),
            ("K:Devour:2", Operation::Devour),
            ("K:Teamwork:3", Operation::Teamwork),
            ("K:Starting intensity:0", Operation::StartingIntensity),
            ("K:Casualty:2", Operation::Casualty),
            ("K:Reconfigure:2", Operation::Reconfigure),
            ("K:Mutate:3 U", Operation::MutateCost),
            ("K:Emerge:6 G", Operation::EmergeCost),
            ("K:Splice:Arcane:1 U", Operation::SpliceCost),
            ("K:Awaken:3:4 U", Operation::AwakenCost),
            ("K:Start your engines", Operation::StartYourEngines),
            ("K:Choose a Background", Operation::ChooseBackground),
            ("K:Doctor's companion", Operation::DoctorsCompanion),
            ("K:Bargain", Operation::Bargain),
            (
                "K:Partner with:Pir, Imaginative Rascal:Pir",
                Operation::PartnerWith,
            ),
            ("K:Partner:Friends forever", Operation::PartnerGroup),
        ] {
            let script = crate::legacy::parse_legacy_script("fixture.txt", keyword)
                .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
            let faces = face_lines(&script);
            let translated = translate_keywords(&script, &faces[0])
                .unwrap_or_else(|error| panic!("keyword should translate: {error:?}"));
            assert!(matches!(
                translated.abilities[0].effect,
                Expression::Call {
                    operation: actual,
                    ..
                } if actual == operation
            ));
        }
    }

    #[test]
    fn desugars_fixed_keyword_alternative_cost() {
        let script = crate::legacy::parse_legacy_script("fixture.txt", "K:Flashback:2 U\n")
            .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("keyword should translate: {error:?}"));
        assert_eq!(translated.ids, vec!["flashback"]);
        assert_eq!(translated.abilities.len(), 1);
        assert_eq!(
            translated.abilities[0].kind,
            forge_carddef::AbilityKind::Static
        );
    }

    #[test]
    fn desugars_closed_crew_keyword() {
        let script = crate::legacy::parse_legacy_script("fixture.txt", "K:Crew:3\n")
            .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("crew should translate: {error:?}"));
        assert_eq!(translated.ids, vec!["crew"]);
        assert!(matches!(
            &translated.abilities[0].effect,
            Expression::Call {
                operation: Operation::Crew,
                arguments
            } if *arguments == vec![
                expression_call(Operation::Source, vec![]),
                Expression::Integer(3)
            ]
        ));

        let limited =
            crate::legacy::parse_legacy_script("fixture.txt", "K:Crew:3:ActivationLimit$ 1\n")
                .unwrap_or_else(|error| panic!("limited crew fixture should parse: {error}"));
        let limited_faces = face_lines(&limited);
        let limited = translate_keywords(&limited, &limited_faces[0])
            .unwrap_or_else(|error| panic!("limited crew should translate: {error:?}"));
        assert!(limited.abilities[0].timing.is_some());
    }

    #[test]
    fn rejects_open_crew_keyword_shapes() {
        for source in ["K:Crew:0\n", "K:Crew:X\n", "K:Crew:3:ActivationLimit$ 2\n"] {
            let script = crate::legacy::parse_legacy_script("fixture.txt", source)
                .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
            let faces = face_lines(&script);
            let error = match translate_keywords(&script, &faces[0]) {
                Ok(_) => panic!("open crew shape must fail closed"),
                Err(error) => error,
            };
            assert!(matches!(
                error.1.as_str(),
                "UNSUPPORTED_VALUE" | "UNSUPPORTED_KEYWORD"
            ));
        }
    }

    #[test]
    fn desugars_closed_etb_extra_counter_replacement() {
        let script = crate::legacy::parse_legacy_script(
            "fixture.txt",
            "K:ETBReplacement:Other:AddExtraCounter:Mandatory:Battlefield:Creature.Other+YouCtrl\n",
        )
        .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("ETB replacement should translate: {error:?}"));
        assert!(translated.ids.is_empty());
        assert_eq!(translated.abilities.len(), 1);
        assert_eq!(translated.abilities[0].kind, AbilityKind::Replacement);
        assert!(matches!(
            &translated.abilities[0].event,
            Some(Expression::Call {
                operation: Operation::EventEnters,
                ..
            })
        ));
        assert!(matches!(
            &translated.abilities[0].effect,
            Expression::Call {
                operation: Operation::ReplaceEvent,
                ..
            }
        ));
    }

    #[test]
    fn rejects_open_etb_extra_counter_replacement_shapes() {
        for source in [
            "K:ETBReplacement:Other:AddExtraCounter:Optional:Battlefield:Creature.YouCtrl\n",
            "K:ETBReplacement:Other:AddExtraCounter:Mandatory:Graveyard:Creature.YouCtrl\n",
            "K:ETBReplacement:Other:AddExtraCounter:Mandatory:Battlefield:Creature.CrewedThisTurn\n",
        ] {
            let script = crate::legacy::parse_legacy_script("fixture.txt", source)
                .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
            let faces = face_lines(&script);
            let error = match translate_keywords(&script, &faces[0]) {
                Ok(_) => panic!("open ETB replacement shape must fail closed"),
                Err(error) => error,
            };
            assert!(matches!(
                error.1.as_str(),
                "UNSUPPORTED_KEYWORD" | "UNSUPPORTED_VALUE"
            ));
        }
    }

    #[test]
    fn desugars_closed_linked_etb_choice_replacement() {
        let script = crate::legacy::parse_legacy_script(
            "fixture.txt",
            concat!(
                "K:ETBReplacement:Other:ChooseColor\n",
                "SVar:ChooseColor:DB$ ChooseColor | Defined$ You | Exclude$ green\n",
            ),
        )
        .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("linked ETB choice should translate: {error:?}"));
        assert_eq!(translated.abilities.len(), 1);
        assert_eq!(translated.abilities[0].kind, AbilityKind::Replacement);
        assert!(matches!(
            translated.abilities[0].effect,
            Expression::Call {
                operation: Operation::ChooseType,
                ..
            }
        ));

        let optional_copy = crate::legacy::parse_legacy_script(
            "fixture.txt",
            concat!(
                "K:ETBReplacement:Copy:DBCopy:Optional\n",
                "SVar:DBCopy:DB$ Clone | Choices$ Creature | ChoiceZone$ Battlefield\n",
            ),
        )
        .unwrap_or_else(|error| panic!("copy fixture should parse: {error}"));
        let optional_faces = face_lines(&optional_copy);
        let optional = translate_keywords(&optional_copy, &optional_faces[0])
            .unwrap_or_else(|error| panic!("optional copy should translate: {error:?}"));
        assert!(matches!(
            optional.abilities[0].effect,
            Expression::Call {
                operation: Operation::ChooseUpTo,
                ..
            }
        ));

        let open = crate::legacy::parse_legacy_script(
            "fixture.txt",
            concat!(
                "K:ETBReplacement:Copy:DBCopy\n",
                "SVar:DBCopy:DB$ ChooseColor | Defined$ You\n",
            ),
        )
        .unwrap_or_else(|error| panic!("open fixture should parse: {error}"));
        let open_faces = face_lines(&open);
        assert!(translate_keywords(&open, &open_faces[0]).is_err());
    }

    #[test]
    fn desugars_fixed_typecycling_search() {
        let script = crate::legacy::parse_legacy_script("fixture.txt", "K:TypeCycling:Forest:2\n")
            .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("keyword should translate: {error:?}"));
        assert_eq!(translated.ids, vec!["typecycling"]);
        assert_eq!(translated.abilities.len(), 1);
        assert_eq!(translated.abilities[0].costs.len(), 2);
    }

    #[test]
    fn lowers_three_chapters_with_a_repeated_svar() {
        let script = crate::legacy::parse_legacy_script(
            "fixture.txt",
            concat!(
                "K:Chapter:3:Draw,Life,Draw\n",
                "SVar:Draw:DB$ Draw | Defined$ You\n",
                "SVar:Life:DB$ GainLife | Defined$ You | LifeAmount$ 1\n",
            ),
        )
        .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let translated = translate_keywords(&script, &faces[0])
            .unwrap_or_else(|error| panic!("Chapter should translate: {error:?}"));

        assert!(translated.ids.is_empty());
        assert_eq!(translated.abilities.len(), 3);
        for (index, ability) in translated.abilities.iter().enumerate() {
            assert_eq!(ability.kind, AbilityKind::Triggered);
            assert!(matches!(
                &ability.event,
                Some(Expression::Call { operation: Operation::EventChapter, arguments })
                    if arguments == &vec![
                        expression_call(Operation::Source, vec![]),
                        Expression::Integer((index + 1) as i64),
                        Expression::Integer(3),
                    ]
            ));
            assert!(ability.condition.is_none());
            assert!(matches!(
                &ability.effect,
                Expression::Call { operation: Operation::Chapter, arguments }
                    if arguments.len() == 3
            ));
        }
        let linked_effect = |ability: &forge_carddef::AbilityDefinition| match &ability.effect {
            Expression::Call { arguments, .. } => arguments[2].clone(),
            _ => panic!("Chapter effect should be a call"),
        };
        assert_eq!(
            linked_effect(&translated.abilities[0]),
            linked_effect(&translated.abilities[2])
        );
    }

    #[test]
    fn rejects_malformed_chapter_maximum_or_count() {
        for keyword in [
            "K:Chapter:0:Draw\n",
            "K:Chapter:3:Draw,Life\n",
            "K:Chapter:nope:Draw\n",
        ] {
            let script = crate::legacy::parse_legacy_script("fixture.txt", keyword)
                .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
            let faces = face_lines(&script);
            let error = translate_keywords(&script, &faces[0])
                .err()
                .unwrap_or_else(|| panic!("malformed Chapter must fail closed"));
            assert_eq!(error.1, "UNSUPPORTED_KEYWORD");
        }
    }

    #[test]
    fn rejects_unsupported_chapter_linked_ability() {
        let script = crate::legacy::parse_legacy_script(
            "fixture.txt",
            "K:Chapter:1:Bad\nSVar:Bad:DB$ NotAnEffect | Defined$ You\n",
        )
        .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let faces = face_lines(&script);
        let error = translate_keywords(&script, &faces[0])
            .err()
            .unwrap_or_else(|| panic!("unsupported linked ability must fail closed"));
        assert_eq!(error.1, "UNMAPPED_API");
    }

    #[test]
    fn chapter_blockers_preserve_linked_code_and_reference_fanout() {
        let script = crate::legacy::parse_legacy_script(
            "fixture.txt",
            concat!(
                "K:Chapter:3:Bad,Good,Bad\n",
                "SVar:Bad:DB$ NotAnEffect\n",
                "SVar:Good:DB$ Draw | Defined$ You\n",
            ),
        )
        .unwrap_or_else(|error| panic!("fixture should parse: {error}"));
        let blockers = collect_script_keyword_blockers(&script);
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].1, "UNMAPPED_API");
        assert!(blockers[0].2.contains("A:NotAnEffect"));
        assert_eq!(blockers[0].3, 2);
    }
}
