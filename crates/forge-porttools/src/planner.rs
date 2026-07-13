//! Corpus-wide multi-blocker dependency planning for translation batches.

use crate::{
    legacy::{collect_scripts, git_revision, parse_legacy_script, LegacyLineKind, LegacyScript},
    mapper::collect_script_mapping_blockers,
    translator::collect_script_keyword_blockers,
};
use rayon::{prelude::*, ThreadPoolBuilder};
use serde::Serialize;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct BlockerPlanOptions<'a> {
    pub root: &'a Path,
    pub priority: &'a Path,
    pub output: &'a Path,
    pub details: &'a Path,
    pub jobs: usize,
    pub batch_size: usize,
    pub batch_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct BlockerPlanReport {
    pub schema_version: u32,
    pub source_root: String,
    pub source_revision: String,
    pub jobs: usize,
    pub analyzed_scripts: usize,
    pub scripts_with_confirmed_blockers: usize,
    pub priority_scripts_with_confirmed_blockers: usize,
    pub unique_blocker_families: usize,
    pub confirmed_observations: usize,
    pub linked_root_fanout: usize,
    families: Vec<BlockerFamilyReport>,
    blocker_sets: Vec<BlockerSetReport>,
    recommended_batches: Vec<RecommendedBatch>,
    pub caveat: String,
}

impl BlockerPlanReport {
    pub(crate) fn recommended_batch_count(&self) -> usize {
        self.recommended_batches.len()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct BlockerFamilyReport {
    id: String,
    family: String,
    code: String,
    label: String,
    estimated_effort_points: u8,
    blocked_cards: usize,
    priority_cards: usize,
    weighted_card_impact: usize,
    linked_root_fanout: usize,
    observations: usize,
    diagnostic_variants: usize,
    sample_paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct BlockerSetReport {
    family_ids: Vec<String>,
    cards: usize,
    priority_cards: usize,
    weighted_card_impact: usize,
    sample_paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct RecommendedBatch {
    batch: usize,
    families: Vec<BatchFamily>,
    newly_confirmed_complete_cards: usize,
    newly_confirmed_complete_priority_cards: usize,
    cumulative_confirmed_complete_cards: usize,
    cumulative_confirmed_complete_priority_cards: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct BatchFamily {
    id: String,
    family: String,
    label: String,
    estimated_effort_points: u8,
    blocked_cards: usize,
    priority_cards: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct BlockerDetailsReport {
    schema_version: u32,
    source_root: String,
    source_revision: String,
    cards: Vec<CardDetails>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct CardDetails {
    path: String,
    names: Vec<String>,
    priority_tier: Option<u8>,
    family_ids: Vec<String>,
    observations: Vec<ObservationDetails>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct ObservationDetails {
    family_id: String,
    line: usize,
    source: String,
    code: String,
    message: String,
    linked_root_fanout: usize,
}

#[derive(Clone, Debug)]
struct CardAnalysis {
    path: String,
    names: Vec<String>,
    priority_tier: Option<u8>,
    blockers: Vec<ObservedBlocker>,
}

#[derive(Clone, Debug)]
struct ObservedBlocker {
    family: String,
    line: usize,
    source: String,
    code: String,
    message: String,
    linked_root_fanout: usize,
}

#[derive(Default)]
struct FamilyAccumulator {
    code: String,
    label: String,
    cards: BTreeSet<String>,
    priority_cards: BTreeSet<String>,
    weighted_card_impact: usize,
    linked_root_fanout: usize,
    observations: usize,
    diagnostic_variants: BTreeSet<String>,
}

#[derive(Default)]
struct SetAccumulator {
    cards: usize,
    priority_cards: usize,
    weighted_card_impact: usize,
    sample_paths: BTreeSet<String>,
}

struct CardModel {
    families: BTreeSet<String>,
    weight: usize,
    priority: bool,
}

pub(crate) fn plan_blocker_batches(
    options: BlockerPlanOptions<'_>,
) -> Result<BlockerPlanReport, String> {
    crate::validate_local_worker_count("blocker planner", options.jobs)?;
    if options.batch_size == 0 || options.batch_count == 0 {
        return Err("blocker planner batch dimensions must be positive".to_string());
    }
    let priorities = read_priority_names(options.priority)?;
    let mut paths = Vec::new();
    collect_scripts(options.root, &mut paths)?;
    paths.sort();
    let pool = ThreadPoolBuilder::new()
        .num_threads(options.jobs)
        .build()
        .map_err(|error| format!("could not create blocker planner pool: {error}"))?;
    let analyses = pool.install(|| {
        paths
            .par_iter()
            .map(|path| analyze_card(options.root, path, &priorities))
            .collect::<Vec<_>>()
    });

    let source_revision = git_revision(options.root)?;
    let mut families = BTreeMap::<String, FamilyAccumulator>::new();
    let mut scripts_with_blockers = 0;
    let mut priority_scripts_with_blockers = 0;
    let mut confirmed_observations = 0;
    let mut linked_root_fanout = 0;
    for card in &analyses {
        if card.blockers.is_empty() {
            continue;
        }
        scripts_with_blockers += 1;
        if card.priority_tier.is_some() {
            priority_scripts_with_blockers += 1;
        }
        let weight = priority_weight(card.priority_tier);
        for blocker in &card.blockers {
            confirmed_observations += 1;
            linked_root_fanout += blocker.linked_root_fanout;
            let accumulator = families.entry(blocker.family.clone()).or_default();
            if accumulator.code.is_empty() {
                accumulator.code.clone_from(&blocker.code);
                accumulator.label = family_label(&blocker.family);
            }
            if accumulator.cards.insert(card.path.clone()) {
                accumulator.weighted_card_impact += weight;
            }
            if card.priority_tier.is_some() {
                accumulator.priority_cards.insert(card.path.clone());
            }
            accumulator.linked_root_fanout += blocker.linked_root_fanout;
            accumulator.observations += 1;
            accumulator
                .diagnostic_variants
                .insert(blocker.message.clone());
        }
    }

    let family_ids = families
        .keys()
        .enumerate()
        .map(|(index, family)| (family.clone(), format!("B{:04}", index + 1)))
        .collect::<BTreeMap<_, _>>();
    let mut family_reports = families
        .iter()
        .map(|(family, accumulator)| BlockerFamilyReport {
            id: family_ids.get(family).cloned().unwrap_or_default(),
            family: family.clone(),
            code: accumulator.code.clone(),
            label: accumulator.label.clone(),
            estimated_effort_points: estimated_effort_points(&accumulator.code),
            blocked_cards: accumulator.cards.len(),
            priority_cards: accumulator.priority_cards.len(),
            weighted_card_impact: accumulator.weighted_card_impact,
            linked_root_fanout: accumulator.linked_root_fanout,
            observations: accumulator.observations,
            diagnostic_variants: accumulator.diagnostic_variants.len(),
            sample_paths: accumulator.cards.iter().take(8).cloned().collect(),
        })
        .collect::<Vec<_>>();
    family_reports.sort_by_key(|family| {
        (
            Reverse(family.weighted_card_impact),
            Reverse(family.linked_root_fanout),
            family.id.clone(),
        )
    });

    let card_models = analyses
        .iter()
        .filter(|card| !card.blockers.is_empty())
        .map(|card| CardModel {
            families: card
                .blockers
                .iter()
                .filter_map(|blocker| family_ids.get(&blocker.family).cloned())
                .collect(),
            weight: priority_weight(card.priority_tier),
            priority: card.priority_tier.is_some(),
        })
        .collect::<Vec<_>>();
    let blocker_sets = summarize_blocker_sets(&analyses, &family_ids);
    let recommended_batches = recommend_batches(
        &card_models,
        &family_reports,
        options.batch_size,
        options.batch_count,
    );

    let details = BlockerDetailsReport {
        schema_version: 1,
        source_root: crate::repository_relative(options.root),
        source_revision: source_revision.clone(),
        cards: analyses
            .iter()
            .filter(|card| !card.blockers.is_empty())
            .map(|card| card_details(card, &family_ids))
            .collect(),
    };
    let report = BlockerPlanReport {
        schema_version: 2,
        source_root: crate::repository_relative(options.root),
        source_revision,
        jobs: options.jobs,
        analyzed_scripts: analyses.len(),
        scripts_with_confirmed_blockers: scripts_with_blockers,
        priority_scripts_with_confirmed_blockers: priority_scripts_with_blockers,
        unique_blocker_families: family_reports.len(),
        confirmed_observations,
        linked_root_fanout,
        families: family_reports,
        blocker_sets,
        recommended_batches,
        caveat: "The planner evaluates every root, reachable SVar ability, and keyword independently and repeatedly peels unknown parameters to expose all such confirmed gaps in one pass. It records the first remaining non-parameter diagnostic per node; another value-level blocker may appear after that diagnostic is fixed, so projected complete-card counts are confirmed-set estimates, not release claims.".to_string(),
    };
    crate::write_json(options.details, &details)?;
    crate::write_json(options.output, &report)?;
    Ok(report)
}

fn analyze_card(root: &Path, path: &Path, priorities: &BTreeMap<String, u8>) -> CardAnalysis {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            return CardAnalysis {
                path: relative,
                names: Vec::new(),
                priority_tier: None,
                blockers: vec![observed_blocker(
                    1,
                    "file",
                    "READ_ERROR",
                    &format!("could not read source: {error}"),
                    1,
                )],
            };
        }
    };
    let script = match parse_legacy_script(&relative, &text) {
        Ok(script) => script,
        Err(error) => {
            return CardAnalysis {
                path: relative,
                names: Vec::new(),
                priority_tier: None,
                blockers: vec![observed_blocker(
                    error.line,
                    "parser",
                    "PARSE_ERROR",
                    &error.to_string(),
                    1,
                )],
            };
        }
    };
    analyze_parsed_card(relative, &script, priorities)
}

fn analyze_parsed_card(
    path: String,
    script: &LegacyScript,
    priorities: &BTreeMap<String, u8>,
) -> CardAnalysis {
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
    let priority_tier = names
        .iter()
        .filter_map(|name| priorities.get(name).copied())
        .min();

    let mut blockers = collect_script_mapping_blockers(script)
        .into_iter()
        .map(|blocker| {
            observed_blocker(
                blocker.line,
                &blocker.source,
                &blocker.code,
                &blocker.message,
                blocker.linked_root_fanout,
            )
        })
        .collect::<Vec<_>>();
    blockers.extend(collect_script_keyword_blockers(script).into_iter().map(
        |(line, code, message, fanout)| observed_blocker(line, "keyword", &code, &message, fanout),
    ));
    let mut deduplicated = BTreeMap::<String, ObservedBlocker>::new();
    for blocker in blockers {
        deduplicated
            .entry(blocker.family.clone())
            .and_modify(|current| {
                current.linked_root_fanout =
                    if current.source == "keyword" && blocker.source == "keyword" {
                        current
                            .linked_root_fanout
                            .saturating_add(blocker.linked_root_fanout)
                    } else {
                        current.linked_root_fanout.max(blocker.linked_root_fanout)
                    };
                if blocker.line < current.line {
                    current.line = blocker.line;
                    current.source.clone_from(&blocker.source);
                    current.code.clone_from(&blocker.code);
                    current.message.clone_from(&blocker.message);
                }
            })
            .or_insert(blocker);
    }
    CardAnalysis {
        path,
        names,
        priority_tier,
        blockers: deduplicated.into_values().collect(),
    }
}

fn observed_blocker(
    line: usize,
    source: &str,
    code: &str,
    message: &str,
    linked_root_fanout: usize,
) -> ObservedBlocker {
    ObservedBlocker {
        family: normalize_family(code, message),
        line,
        source: source.to_string(),
        code: code.to_string(),
        message: message.to_string(),
        linked_root_fanout,
    }
}

fn normalize_family(code: &str, message: &str) -> String {
    let quoted = backtick_values(message);
    let suffix = match code {
        "UNSUPPORTED_PARAMETER" | "UNSUPPORTED_KEYWORD" => {
            quoted.first().copied().unwrap_or(message).to_string()
        }
        "UNSUPPORTED_VALUE" if quoted.len() >= 2 => format!("{}={}", quoted[0], quoted[1]),
        "UNSUPPORTED_VALUE_SVAR" if quoted.len() >= 2 => {
            quoted[1].split('$').next().unwrap_or(quoted[1]).to_string()
        }
        "UNMAPPED_API" => message
            .rsplit_once(" for ")
            .map(|(_, api)| api)
            .unwrap_or(message)
            .to_string(),
        "DUPLICATE_SVAR" | "MISSING_SVAR" | "CYCLIC_SVAR" => code.to_string(),
        _ => message.to_string(),
    };
    format!("{code}:{suffix}")
}

fn family_label(family: &str) -> String {
    family
        .split_once(':')
        .map(|(_, label)| label)
        .unwrap_or(family)
        .to_string()
}

fn backtick_values(message: &str) -> Vec<&str> {
    message
        .split('`')
        .enumerate()
        .filter_map(|(index, value)| (index % 2 == 1).then_some(value))
        .collect()
}

fn priority_weight(tier: Option<u8>) -> usize {
    match tier {
        Some(0) => 20,
        Some(1) => 10,
        Some(2) => 5,
        Some(_) => 3,
        None => 1,
    }
}

fn read_priority_names(path: &Path) -> Result<BTreeMap<String, u8>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let mut current_tier = None;
    let mut priorities = BTreeMap::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix("# TIER ") {
            let tier = header
                .split_once(" - ")
                .map(|(tier, _)| tier)
                .unwrap_or(header)
                .parse::<u8>()
                .map_err(|_| {
                    format!(
                        "{}:{} priority tier is not an integer",
                        path.display(),
                        index + 1
                    )
                })?;
            current_tier = Some(tier);
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let tier = current_tier.ok_or_else(|| {
            format!(
                "{}:{} priority card appears before a tier",
                path.display(),
                index + 1
            )
        })?;
        for name in line
            .split('|')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if priorities.insert(name.to_string(), tier).is_some() {
                return Err(format!(
                    "{}:{} duplicate priority card `{name}`",
                    path.display(),
                    index + 1
                ));
            }
        }
    }
    if priorities.is_empty() {
        return Err(format!("{} contains no priority cards", path.display()));
    }
    Ok(priorities)
}

fn summarize_blocker_sets(
    cards: &[CardAnalysis],
    family_ids: &BTreeMap<String, String>,
) -> Vec<BlockerSetReport> {
    let mut sets = BTreeMap::<Vec<String>, SetAccumulator>::new();
    for card in cards.iter().filter(|card| !card.blockers.is_empty()) {
        let family_ids = card
            .blockers
            .iter()
            .filter_map(|blocker| family_ids.get(&blocker.family).cloned())
            .collect::<Vec<_>>();
        let accumulator = sets.entry(family_ids).or_default();
        accumulator.cards += 1;
        accumulator.priority_cards += usize::from(card.priority_tier.is_some());
        accumulator.weighted_card_impact += priority_weight(card.priority_tier);
        accumulator.sample_paths.insert(card.path.clone());
    }
    let mut reports = sets
        .into_iter()
        .map(|(family_ids, accumulator)| BlockerSetReport {
            family_ids,
            cards: accumulator.cards,
            priority_cards: accumulator.priority_cards,
            weighted_card_impact: accumulator.weighted_card_impact,
            sample_paths: accumulator.sample_paths.into_iter().take(8).collect(),
        })
        .collect::<Vec<_>>();
    reports.sort_by_key(|set| {
        (
            Reverse(set.weighted_card_impact),
            Reverse(set.cards),
            set.family_ids.clone(),
        )
    });
    reports.truncate(100);
    reports
}

fn recommend_batches(
    cards: &[CardModel],
    families: &[BlockerFamilyReport],
    batch_size: usize,
    batch_count: usize,
) -> Vec<RecommendedBatch> {
    let family_by_id = families
        .iter()
        .map(|family| (family.id.clone(), family))
        .collect::<BTreeMap<_, _>>();
    let mut card_indices_by_family = BTreeMap::<String, Vec<usize>>::new();
    for (index, card) in cards.iter().enumerate() {
        for family in &card.families {
            card_indices_by_family
                .entry(family.clone())
                .or_default()
                .push(index);
        }
    }
    let mut selected = BTreeSet::new();
    let mut batches = Vec::new();
    for batch_index in 0..batch_count {
        let before = completed_cards(cards, &selected);
        let mut batch_ids = Vec::new();
        for _ in 0..batch_size {
            let mut best: Option<(u64, String)> = None;
            for family in families {
                if selected.contains(&family.id) {
                    continue;
                }
                let score = candidate_score(
                    cards,
                    card_indices_by_family
                        .get(&family.id)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                    &selected,
                    family,
                );
                let replace = match best.as_ref() {
                    None => true,
                    Some((best_score, best_id)) => {
                        score > *best_score || score == *best_score && family.id < *best_id
                    }
                };
                if replace {
                    best = Some((score, family.id.clone()));
                }
            }
            let Some((score, id)) = best else {
                break;
            };
            if score == 0 {
                break;
            }
            selected.insert(id.clone());
            batch_ids.push(id);
        }
        if batch_ids.is_empty() {
            break;
        }
        let after = completed_cards(cards, &selected);
        let families = batch_ids
            .iter()
            .filter_map(|id| family_by_id.get(id).copied())
            .map(|family| BatchFamily {
                id: family.id.clone(),
                family: family.family.clone(),
                label: family.label.clone(),
                estimated_effort_points: family.estimated_effort_points,
                blocked_cards: family.blocked_cards,
                priority_cards: family.priority_cards,
            })
            .collect();
        batches.push(RecommendedBatch {
            batch: batch_index + 1,
            families,
            newly_confirmed_complete_cards: after.0.saturating_sub(before.0),
            newly_confirmed_complete_priority_cards: after.1.saturating_sub(before.1),
            cumulative_confirmed_complete_cards: after.0,
            cumulative_confirmed_complete_priority_cards: after.1,
        });
    }
    batches
}

fn candidate_score(
    cards: &[CardModel],
    card_indices: &[usize],
    selected: &BTreeSet<String>,
    family: &BlockerFamilyReport,
) -> u64 {
    let mut newly_complete_weight = 0_u64;
    let mut progress = 0_u64;
    for card in card_indices.iter().filter_map(|index| cards.get(*index)) {
        let complete_before = card.families.is_subset(selected);
        let complete_after = card
            .families
            .iter()
            .all(|id| selected.contains(id) || id == &family.id);
        if !complete_before && complete_after {
            newly_complete_weight += card.weight as u64;
        }
        progress += (card.weight.saturating_mul(1_000) / card.families.len().max(1)) as u64;
    }
    let impact = newly_complete_weight
        .saturating_mul(1_000_000_000)
        .saturating_add(progress.saturating_mul(1_000))
        .saturating_add(family.weighted_card_impact as u64 * 10)
        .saturating_add(family.linked_root_fanout as u64);
    impact / u64::from(family.estimated_effort_points.max(1))
}

fn estimated_effort_points(code: &str) -> u8 {
    match code {
        "UNSUPPORTED_PARAMETER" => 1,
        "UNSUPPORTED_VALUE" | "UNSUPPORTED_VALUE_SVAR" => 2,
        "UNSUPPORTED_KEYWORD" => 3,
        "UNMAPPED_API" => 4,
        _ => 3,
    }
}

fn completed_cards(cards: &[CardModel], selected: &BTreeSet<String>) -> (usize, usize) {
    let mut complete = 0;
    let mut priority = 0;
    for card in cards
        .iter()
        .filter(|card| card.families.is_subset(selected))
    {
        complete += 1;
        priority += usize::from(card.priority);
    }
    (complete, priority)
}

fn card_details(card: &CardAnalysis, family_ids: &BTreeMap<String, String>) -> CardDetails {
    let mut observations = card
        .blockers
        .iter()
        .filter_map(|blocker| {
            family_ids
                .get(&blocker.family)
                .map(|family_id| ObservationDetails {
                    family_id: family_id.clone(),
                    line: blocker.line,
                    source: blocker.source.clone(),
                    code: blocker.code.clone(),
                    message: blocker.message.clone(),
                    linked_root_fanout: blocker.linked_root_fanout,
                })
        })
        .collect::<Vec<_>>();
    observations.sort_by(|left, right| {
        left.family_id
            .cmp(&right.family_id)
            .then_with(|| left.line.cmp(&right.line))
    });
    CardDetails {
        path: card.path.clone(),
        names: card.names.clone(),
        priority_tier: card.priority_tier,
        family_ids: observations
            .iter()
            .map(|observation| observation.family_id.clone())
            .collect(),
        observations,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        analyze_parsed_card, normalize_family, plan_blocker_batches, recommend_batches,
        BlockerFamilyReport, BlockerPlanOptions,
    };
    use crate::legacy::parse_legacy_script;
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::Path,
        process::Command,
    };

    #[test]
    fn discovers_multiple_independent_blockers_and_keyword_gaps() {
        let script = parse_legacy_script(
            "fixture.txt",
            concat!(
                "Name:Planner Fixture\n",
                "K:Ward:2:Open\n",
                "A:SP$ PlannerProbe | Valid$ Card | SubAbility$ Extra | SpellDescription$ Probe.\n",
                "SVar:Extra:DB$ Effect | StaticAbilities$ KWPump | SubAbility$ Tail\n",
                "SVar:Tail:DB$ Draw | Defined$ You | ConditionPlayerTurn$ True\n",
                "SVar:X:Count$Valid Creature.YouCtrl\n",
            ),
        )
        .unwrap_or_else(|error| panic!("planner fixture should parse: {error}"));
        let priorities = BTreeMap::from([("Planner Fixture".to_string(), 0)]);
        let card = analyze_parsed_card("fixture.txt".to_string(), &script, &priorities);
        let families = card
            .blockers
            .iter()
            .map(|blocker| blocker.family.clone())
            .collect::<BTreeSet<_>>();
        assert!(families
            .iter()
            .any(|family| family.contains("A:PlannerProbe")));
        assert!(families
            .iter()
            .any(|family| family.contains("StaticAbilities")));
        assert!(families
            .iter()
            .any(|family| family.contains("ConditionPlayerTurn")));
        assert!(families.iter().any(|family| family.contains("Ward")));
        assert_eq!(card.priority_tier, Some(0));
    }

    #[test]
    fn normalizes_actionable_families() {
        assert_eq!(
            normalize_family(
                "UNSUPPORTED_PARAMETER",
                "parameter `TargetMax` has no typed mapper"
            ),
            "UNSUPPORTED_PARAMETER:TargetMax"
        );
        assert_eq!(
            normalize_family("UNMAPPED_API", "no mapper is registered for A:Dig"),
            "UNMAPPED_API:A:Dig"
        );
        assert_eq!(
            normalize_family(
                "UNSUPPORTED_VALUE",
                "parameter `Phase` value `End of Turn` has no exact lowering"
            ),
            "UNSUPPORTED_VALUE:Phase=End of Turn"
        );
    }

    #[test]
    fn combines_distinct_chapter_roots_in_the_same_blocker_family() {
        let script = parse_legacy_script(
            "fixture.txt",
            concat!(
                "Name:Chapter Fanout\n",
                "K:Chapter:2:BadA,BadB\n",
                "SVar:BadA:DB$ NotAnEffect\n",
                "SVar:BadB:DB$ NotAnEffect\n",
            ),
        )
        .unwrap_or_else(|error| panic!("Chapter fanout fixture should parse: {error}"));
        let card = analyze_parsed_card("fixture.txt".to_string(), &script, &BTreeMap::new());
        let blocker = card
            .blockers
            .iter()
            .find(|blocker| blocker.family.contains("A:NotAnEffect"))
            .unwrap_or_else(|| panic!("Chapter API blocker should be present"));
        assert_eq!(blocker.linked_root_fanout, 2);
    }

    #[test]
    fn greedy_batches_are_deterministic() {
        let families = vec![
            BlockerFamilyReport {
                id: "B0001".to_string(),
                family: "one".to_string(),
                code: "one".to_string(),
                label: "one".to_string(),
                estimated_effort_points: 3,
                blocked_cards: 2,
                priority_cards: 1,
                weighted_card_impact: 21,
                linked_root_fanout: 2,
                observations: 2,
                diagnostic_variants: 1,
                sample_paths: Vec::new(),
            },
            BlockerFamilyReport {
                id: "B0002".to_string(),
                family: "two".to_string(),
                code: "two".to_string(),
                label: "two".to_string(),
                estimated_effort_points: 3,
                blocked_cards: 1,
                priority_cards: 0,
                weighted_card_impact: 1,
                linked_root_fanout: 1,
                observations: 1,
                diagnostic_variants: 1,
                sample_paths: Vec::new(),
            },
        ];
        let cards = vec![
            super::CardModel {
                families: BTreeSet::from(["B0001".to_string()]),
                weight: 20,
                priority: true,
            },
            super::CardModel {
                families: BTreeSet::from(["B0001".to_string(), "B0002".to_string()]),
                weight: 1,
                priority: false,
            },
        ];
        let batches = recommend_batches(&cards, &families, 2, 1);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].families[0].id, "B0001");
        assert_eq!(batches[0].cumulative_confirmed_complete_cards, 2);
    }

    #[test]
    fn greedy_batches_prefer_lower_effort_at_equal_impact() {
        let families = vec![
            BlockerFamilyReport {
                id: "B0001".to_string(),
                family: "api".to_string(),
                code: "UNMAPPED_API".to_string(),
                label: "api".to_string(),
                estimated_effort_points: 4,
                blocked_cards: 1,
                priority_cards: 0,
                weighted_card_impact: 1,
                linked_root_fanout: 1,
                observations: 1,
                diagnostic_variants: 1,
                sample_paths: Vec::new(),
            },
            BlockerFamilyReport {
                id: "B0002".to_string(),
                family: "parameter".to_string(),
                code: "UNSUPPORTED_PARAMETER".to_string(),
                label: "parameter".to_string(),
                estimated_effort_points: 1,
                blocked_cards: 1,
                priority_cards: 0,
                weighted_card_impact: 1,
                linked_root_fanout: 1,
                observations: 1,
                diagnostic_variants: 1,
                sample_paths: Vec::new(),
            },
        ];
        let cards = vec![
            super::CardModel {
                families: BTreeSet::from(["B0001".to_string()]),
                weight: 1,
                priority: false,
            },
            super::CardModel {
                families: BTreeSet::from(["B0002".to_string()]),
                weight: 1,
                priority: false,
            },
        ];

        let batches = recommend_batches(&cards, &families, 1, 1);
        assert_eq!(batches[0].families[0].id, "B0002");
    }

    #[test]
    fn writes_a_deterministic_corpus_plan_and_details() {
        let root = std::env::temp_dir().join(format!(
            "forge-porttools-blocker-plan-{}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root)
                .unwrap_or_else(|error| panic!("could not clear planner fixture: {error}"));
        }
        let cards = root.join("cards");
        fs::create_dir_all(&cards)
            .unwrap_or_else(|error| panic!("could not create planner fixture: {error}"));
        fs::write(
            cards.join("blocked.txt"),
            concat!(
                "Name:Planner Fixture\n",
                "K:Ward:2:Open\n",
                "A:SP$ DigUntil | Valid$ Card | SubAbility$ Extra | SpellDescription$ Dig.\n",
                "SVar:Extra:DB$ Effect\n",
            ),
        )
        .unwrap_or_else(|error| panic!("could not write blocked fixture: {error}"));
        fs::write(
            cards.join("supported.txt"),
            concat!(
                "Name:Supported Fixture\n",
                "A:SP$ Draw | Defined$ You | NumCards$ 1 | SpellDescription$ Draw.\n",
            ),
        )
        .unwrap_or_else(|error| panic!("could not write supported fixture: {error}"));
        let priority = root.join("priority.list");
        fs::write(&priority, "# TIER 0 - fixture\nPlanner Fixture\n")
            .unwrap_or_else(|error| panic!("could not write priority fixture: {error}"));
        run_git(&root, &["init", "-q"]);
        run_git(&root, &["config", "user.email", "fixture@example.invalid"]);
        run_git(&root, &["config", "user.name", "Fixture"]);
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "-qm", "fixture"]);

        let output = root.join("blocker-plan.json");
        let details = root.join("blocker-details.json");
        let options = |jobs| BlockerPlanOptions {
            root: &cards,
            priority: &priority,
            output: &output,
            details: &details,
            jobs,
            batch_size: 2,
            batch_count: 2,
        };
        let report = plan_blocker_batches(options(2))
            .unwrap_or_else(|error| panic!("planner fixture should pass: {error}"));
        assert_eq!(report.analyzed_scripts, 2);
        assert_eq!(report.scripts_with_confirmed_blockers, 1);
        assert_eq!(report.priority_scripts_with_confirmed_blockers, 1);
        assert!(report.unique_blocker_families >= 3);
        assert!(report.confirmed_observations >= 3);
        assert!(output.is_file());
        assert!(details.is_file());

        let first = fs::read(&output)
            .unwrap_or_else(|error| panic!("could not read first planner output: {error}"));
        plan_blocker_batches(options(1))
            .unwrap_or_else(|error| panic!("repeated planner fixture should pass: {error}"));
        let second = fs::read(&output)
            .unwrap_or_else(|error| panic!("could not read second planner output: {error}"));
        let mut first_report: serde_json::Value = serde_json::from_slice(&first)
            .unwrap_or_else(|error| panic!("could not decode first planner output: {error}"));
        let mut second_report: serde_json::Value = serde_json::from_slice(&second)
            .unwrap_or_else(|error| panic!("could not decode second planner output: {error}"));
        first_report["jobs"] = serde_json::Value::Null;
        second_report["jobs"] = serde_json::Value::Null;
        assert_eq!(first_report, second_report);

        let details_at_one = fs::read(&details)
            .unwrap_or_else(|error| panic!("could not read one-worker details: {error}"));
        plan_blocker_batches(options(2))
            .unwrap_or_else(|error| panic!("final planner fixture should pass: {error}"));
        let details_at_two = fs::read(&details)
            .unwrap_or_else(|error| panic!("could not read two-worker details: {error}"));
        assert_eq!(details_at_one, details_at_two);

        fs::remove_dir_all(&root)
            .unwrap_or_else(|error| panic!("could not remove planner fixture: {error}"));
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
