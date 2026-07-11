#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Streaming source-catalog import and explicit translation classification tools.

pub mod legacy;
pub mod mapper;
mod planner;
pub mod translator;

use forge_carddef::{
    CardCatalog, CardClassification, CardLayout, IdentityRecord, OracleId, PrintingId,
    PrintingRecord, SourceProvenance,
};
use serde::{
    de::{DeserializeSeed, Error as _, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use std::fmt::Write as _;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fmt,
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

pub(crate) const MAX_LOCAL_WORKERS: usize = 24;

pub(crate) fn validate_local_worker_count(label: &str, jobs: usize) -> Result<(), String> {
    if jobs == 0 {
        return Err(format!("{label} jobs must be positive"));
    }
    if jobs > MAX_LOCAL_WORKERS {
        return Err(format!(
            "{label} jobs {jobs} exceed the local worker ceiling {MAX_LOCAL_WORKERS}"
        ));
    }
    Ok(())
}

/// Paths used by one deterministic Scryfall catalog import.
#[derive(Clone, Copy, Debug)]
pub struct CatalogImportOptions<'a> {
    /// Pinned local Scryfall all-cards JSON snapshot.
    pub source: &'a Path,
    /// Generated summary containing source counts, timestamp, and SHA-256.
    pub summary: &'a Path,
    /// Compact catalog JSON destination.
    pub output: &'a Path,
    /// Generated catalog metrics JSON destination.
    pub metrics: &'a Path,
}

/// Counts produced by one completed catalog import.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogImportReport {
    /// Every source record encountered while streaming.
    pub source_records: usize,
    /// English records declared by the pinned source summary.
    pub source_english_records: usize,
    /// English printings retained in the compact catalog.
    pub english_printings: usize,
    /// Distinct mechanics identities in the compact catalog.
    pub identities: usize,
    /// Catalog-only identities.
    pub catalog_only: usize,
    /// Out-of-v1 identities.
    pub out_of_v1: usize,
    /// Unverified playable identities.
    pub unverified_playable: usize,
    /// Catalog destination.
    pub output: PathBuf,
    /// Metrics destination.
    pub metrics: PathBuf,
}

/// Streams a pinned Scryfall snapshot into the compact Forge catalog.
pub fn import_scryfall_catalog(
    options: CatalogImportOptions<'_>,
) -> Result<CatalogImportReport, String> {
    let summary_file = File::open(options.summary)
        .map_err(|error| format!("could not open {}: {error}", options.summary.display()))?;
    let summary: ScryfallSummary = serde_json::from_reader(BufReader::new(summary_file))
        .map_err(|error| format!("invalid {}: {error}", options.summary.display()))?;
    validate_source_metadata(options.source, &summary)?;

    let source_file = File::open(options.source)
        .map_err(|error| format!("could not open {}: {error}", options.source.display()))?;
    let mut builder = CatalogBuilder::default();
    stream_cards(BufReader::new(source_file), &mut builder)
        .map_err(|error| format!("invalid {}: {error}", options.source.display()))?;
    if builder.total_records != summary.total_records {
        return Err(format!(
            "source record count {} does not match summary {}",
            builder.total_records, summary.total_records
        ));
    }
    if builder.english_records != summary.english_records {
        return Err(format!(
            "English record count {} does not match summary {}",
            builder.english_records, summary.english_records
        ));
    }

    let provenance = SourceProvenance {
        source: summary.source.clone(),
        source_path: repository_relative(options.source),
        source_updated_at: summary.source_updated_at.clone(),
        source_sha256: summary.local_sha256.clone(),
        generator: format!("forge-porttools {}", env!("CARGO_PKG_VERSION")),
    };
    let catalog = builder.finish(provenance)?;
    validate_catalog(&catalog)?;
    let fallback_identities = catalog
        .identities
        .iter()
        .filter(|identity| identity.id.as_str().starts_with("source:"))
        .count();
    let sourced_oracle_identities = catalog.identities.len() - fallback_identities;
    if sourced_oracle_identities != summary.unique_english_oracle_ids {
        return Err(format!(
            "source Oracle identity count {sourced_oracle_identities} does not match summary {}",
            summary.unique_english_oracle_ids
        ));
    }
    let counts = classification_counts(&catalog);
    let report = CatalogImportReport {
        source_records: summary.total_records,
        source_english_records: summary.english_records,
        english_printings: catalog.printings.len(),
        identities: catalog.identities.len(),
        catalog_only: counts.catalog_only,
        out_of_v1: counts.out_of_v1,
        unverified_playable: counts.unverified_playable,
        output: options.output.to_path_buf(),
        metrics: options.metrics.to_path_buf(),
    };
    let metrics = CatalogMetrics {
        schema_version: catalog.schema_version,
        source: catalog.provenance.clone(),
        source_records: report.source_records,
        source_english_records: summary.english_records,
        imported_english_printings: report.english_printings,
        source_unique_english_oracle_ids: summary.unique_english_oracle_ids,
        source_fallback_identities: fallback_identities,
        expected_classified_identities: summary.unique_english_oracle_ids + fallback_identities,
        classified_identities: report.identities,
        verified_playable: counts.verified_playable,
        unverified_playable: counts.unverified_playable,
        quarantined: counts.quarantined,
        out_of_v1: counts.out_of_v1,
        catalog_only: counts.catalog_only,
        dangling_printing_references: 0,
    };
    write_json(options.output, &catalog)?;
    write_json(options.metrics, &metrics)?;
    Ok(report)
}

/// Runs the Forge porttools command-line surface.
pub fn run_cli(args: Vec<String>) -> Result<String, String> {
    match args.as_slice() {
        [translate, rest @ ..] if translate == "translate" => run_translate(rest),
        [legacy, blocker_plan, rest @ ..]
            if legacy == "legacy" && blocker_plan == "blocker-plan" =>
        {
            run_blocker_plan(rest)
        }
        [legacy, map_audit, root_flag, root, metrics_flag, metrics, quarantine_flag, quarantine]
            if legacy == "legacy"
                && map_audit == "map-audit"
                && root_flag == "--root"
                && metrics_flag == "--metrics"
                && quarantine_flag == "--quarantine" =>
        {
            let report = mapper::audit_legacy_mappings(
                Path::new(root),
                Path::new(metrics),
                Path::new(quarantine),
            )?;
            Ok(format!(
                "mapped {}/{} legacy ability uses ({:.4}%)\nmetrics {}\nquarantine {}\n",
                report.mapped_uses, report.legacy_uses, report.mapped_percent, metrics, quarantine
            ))
        }
        [legacy, parse, root_flag, root, metrics_flag, metrics, failures_flag, failures]
            if legacy == "legacy"
                && parse == "parse"
                && root_flag == "--root"
                && metrics_flag == "--metrics"
                && failures_flag == "--failures" =>
        {
            let report = legacy::audit_legacy_corpus(
                Path::new(root),
                Path::new(metrics),
                Path::new(failures),
            )?;
            if !report.passed {
                return Err(format!(
                    "legacy parse coverage {:.4}% is below the {:.1}% floor; see {}",
                    report.parse_rate_percent, report.target_parse_rate_percent, failures
                ));
            }
            Ok(format!(
                "parsed {}/{} legacy scripts ({:.4}%)\nmetrics {}\nfailures {}\n",
                report.parsed_files,
                report.total_files,
                report.parse_rate_percent,
                metrics,
                failures
            ))
        }
        [catalog, import, source_flag, source, summary_flag, summary, output_flag, output, metrics_flag, metrics]
            if catalog == "catalog"
                && import == "import"
                && source_flag == "--source"
                && summary_flag == "--summary"
                && output_flag == "--output"
                && metrics_flag == "--metrics" =>
        {
            let report = import_scryfall_catalog(CatalogImportOptions {
                source: Path::new(source),
                summary: Path::new(summary),
                output: Path::new(output),
                metrics: Path::new(metrics),
            })?;
            Ok(format!(
                "imported {}/{} English printings across {} classified identities ({} playable-unverified, {} out-of-v1, {} catalog-only)\ncatalog {}\nmetrics {}\n",
                report.english_printings,
                report.source_english_records,
                report.identities,
                report.unverified_playable,
                report.out_of_v1,
                report.catalog_only,
                report.output.display(),
                report.metrics.display()
            ))
        }
        [catalog, extract, source_flag, source, summary_flag, summary, selection_flag, selection, output_flag, output]
            if catalog == "catalog"
                && extract == "extract"
                && source_flag == "--source"
                && summary_flag == "--summary"
                && selection_flag == "--selection"
                && output_flag == "--output" =>
        {
            extract_selection(
                Path::new(source),
                Path::new(summary),
                Path::new(selection),
                Path::new(output),
            )
        }
        [command, flag, catalog_flag, path]
            if command == "quarantine" && flag == "--list" && catalog_flag == "--catalog" =>
        {
            list_quarantine(Path::new(path))
        }
        [command, ..] => Err(format!(
            "unknown forge-porttools command `{command}`\n{}",
            usage()
        )),
        [] => Err(usage()),
    }
}

fn run_translate(args: &[String]) -> Result<String, String> {
    if args.first().map(String::as_str) != Some("--all") {
        return Err(format!("translate requires --all\n{}", usage()));
    }
    let flags = parse_named_flags(
        &args[1..],
        &[
            "--jobs",
            "--root",
            "--catalog",
            "--output",
            "--metrics",
            "--quarantine",
            "--priority",
            "--priority-metrics",
            "--write-output",
        ],
    )?;
    let jobs = required_usize_flag(&flags, "--jobs", "translation worker count")?;
    let write_output = optional_bool_flag(&flags, "--write-output", true)?;
    let root = flags
        .get("--root")
        .map(String::as_str)
        .unwrap_or("vendor/legacy-forge/forge-gui/res/cardsfolder");
    let catalog = flags
        .get("--catalog")
        .map(String::as_str)
        .unwrap_or("assets/card_catalog.json");
    let output = flags
        .get("--output")
        .map(String::as_str)
        .unwrap_or("target/translated-cards");
    let metrics = flags
        .get("--metrics")
        .map(String::as_str)
        .unwrap_or("metrics/translation.json");
    let quarantine = flags
        .get("--quarantine")
        .map(String::as_str)
        .unwrap_or("metrics/translation_quarantine.json");
    let priority = flags
        .get("--priority")
        .map(String::as_str)
        .unwrap_or("assets/coverage_priority.txt");
    let priority_metrics = flags
        .get("--priority-metrics")
        .map(String::as_str)
        .unwrap_or("metrics/priority_coverage.json");
    let report = translator::translate_all(translator::TranslateOptions {
        root: Path::new(root),
        catalog: Path::new(catalog),
        output: Path::new(output),
        metrics: Path::new(metrics),
        quarantine: Path::new(quarantine),
        priority: Path::new(priority),
        priority_metrics: Path::new(priority_metrics),
        jobs,
        write_output,
    })?;
    let output_status = if write_output {
        output.to_string()
    } else {
        "skipped (fingerprint-only replay)".to_string()
    };
    Ok(format!(
        "emitted {}/{} legacy scripts ({:.4}%) with {} local workers\npriority {}/{} requested cards ({:.4}%)\nfingerprint {}\noutput {output_status}\nmetrics {metrics}\npriority metrics {priority_metrics}\nquarantine {quarantine}\n",
        report.emitted_scripts,
        report.total_scripts,
        report.emitted_percent,
        report.jobs,
        report.priority_emitted,
        report.priority_requested,
        report.priority_emitted_percent,
        report.output_fingerprint,
    ))
}

fn run_blocker_plan(args: &[String]) -> Result<String, String> {
    let flags = parse_named_flags(
        args,
        &[
            "--root",
            "--priority",
            "--output",
            "--details",
            "--jobs",
            "--batch-size",
            "--batch-count",
        ],
    )?;
    let jobs = required_usize_flag(&flags, "--jobs", "blocker planner worker count")?;
    let batch_size = optional_usize_flag(&flags, "--batch-size", 5)?;
    let batch_count = optional_usize_flag(&flags, "--batch-count", 6)?;
    let root = flags
        .get("--root")
        .map(String::as_str)
        .unwrap_or("vendor/legacy-forge/forge-gui/res/cardsfolder");
    let priority = flags
        .get("--priority")
        .map(String::as_str)
        .unwrap_or("assets/coverage_priority.txt");
    let output = flags
        .get("--output")
        .map(String::as_str)
        .unwrap_or("metrics/blocker_plan.json");
    let details = flags
        .get("--details")
        .map(String::as_str)
        .unwrap_or("target/t3-blocker-plan/cards.json");
    let report = planner::plan_blocker_batches(planner::BlockerPlanOptions {
        root: Path::new(root),
        priority: Path::new(priority),
        output: Path::new(output),
        details: Path::new(details),
        jobs,
        batch_size,
        batch_count,
    })?;
    Ok(format!(
        "analyzed {} scripts; {} have confirmed blockers across {} families\nrecommended {} blocker batches\nmetrics {output}\ndetails {details}\n",
        report.analyzed_scripts,
        report.scripts_with_confirmed_blockers,
        report.unique_blocker_families,
        report.recommended_batch_count(),
    ))
}

fn parse_named_flags(
    args: &[String],
    allowed: &[&str],
) -> Result<BTreeMap<String, String>, String> {
    if args.len() % 2 != 0 {
        return Err("command flags require a value".to_string());
    }
    let mut flags = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        let flag = &pair[0];
        if !allowed.contains(&flag.as_str()) {
            return Err(format!("unknown flag `{flag}`\n{}", usage()));
        }
        if flags.insert(flag.clone(), pair[1].clone()).is_some() {
            return Err(format!("duplicate flag `{flag}`"));
        }
    }
    Ok(flags)
}

fn required_usize_flag(
    flags: &BTreeMap<String, String>,
    flag: &str,
    label: &str,
) -> Result<usize, String> {
    let value = flags
        .get(flag)
        .ok_or_else(|| format!("missing required flag `{flag}`"))?;
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("invalid {label} `{value}`"))
}

fn optional_usize_flag(
    flags: &BTreeMap<String, String>,
    flag: &str,
    default: usize,
) -> Result<usize, String> {
    let Some(value) = flags.get(flag) else {
        return Ok(default);
    };
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("invalid `{flag}` value `{value}`"))
}

fn optional_bool_flag(
    flags: &BTreeMap<String, String>,
    flag: &str,
    default: bool,
) -> Result<bool, String> {
    let Some(value) = flags.get(flag) else {
        return Ok(default);
    };
    match value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!(
            "invalid `{flag}` value `{value}`; expected true or false"
        )),
    }
}

fn list_quarantine(path: &Path) -> Result<String, String> {
    let file = File::open(path)
        .map_err(|error| format!("could not open catalog {}: {error}", path.display()))?;
    let catalog: CardCatalog = serde_json::from_reader(BufReader::new(file))
        .map_err(|error| format!("invalid catalog {}: {error}", path.display()))?;
    validate_catalog(&catalog)?;
    if catalog.identities.is_empty() || catalog.printings.is_empty() {
        return Err("catalog is uninitialized".to_string());
    }
    let counts = classification_counts(&catalog);
    let mut output = format!(
        "classified identities: {}; verified: {}; unverified: {}; quarantined: {}; out-of-v1: {}; catalog-only: {}\n",
        catalog.identities.len(),
        counts.verified_playable,
        counts.unverified_playable,
        counts.quarantined,
        counts.out_of_v1,
        counts.catalog_only
    );
    for identity in catalog
        .identities
        .iter()
        .filter(|identity| matches!(identity.classification, CardClassification::Quarantined(_)))
    {
        if let CardClassification::Quarantined(reason) = &identity.classification {
            let _ = writeln!(
                output,
                "{}\t{}\t{}",
                identity.id.as_str(),
                identity.name,
                reason
            );
        }
    }
    Ok(output)
}

fn usage() -> String {
    "usage: forge-porttools translate --all --jobs <N> [--output <dir> --metrics <json> --quarantine <json> --priority-metrics <json> --write-output <true|false>] | forge-porttools legacy blocker-plan --jobs <N> [--output <json> --details <json> --batch-size <N> --batch-count <N>] | forge-porttools legacy parse --root <cardsfolder> --metrics <metrics.json> --failures <failures.json> | forge-porttools legacy map-audit --root <cardsfolder> --metrics <metrics.json> --quarantine <quarantine.json> | forge-porttools catalog import --source <all-cards.json> --summary <summary.json> --output <catalog.json> --metrics <metrics.json> | forge-porttools catalog extract --source <all-cards.json> --summary <summary.json> --selection <selection.json> --output <source-cards.json> | forge-porttools quarantine --list --catalog <catalog.json>".to_string()
}

#[derive(Clone, Debug, Deserialize)]
struct ScryfallSummary {
    source: String,
    source_updated_at: String,
    total_records: usize,
    english_records: usize,
    unique_english_oracle_ids: usize,
    local_size_bytes: u64,
    local_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ScryfallCard {
    id: String,
    #[serde(default)]
    oracle_id: Option<String>,
    name: String,
    lang: String,
    released_at: String,
    layout: String,
    #[serde(rename = "set")]
    set_code: String,
    collector_number: String,
    #[serde(default)]
    set_type: String,
    #[serde(default)]
    mana_cost: String,
    #[serde(default)]
    type_line: String,
    #[serde(default)]
    oracle_text: String,
    #[serde(default)]
    power: Option<String>,
    #[serde(default)]
    toughness: Option<String>,
    #[serde(default)]
    loyalty: Option<String>,
    #[serde(default)]
    defense: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    card_faces: Vec<ScryfallFace>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ScryfallFace {
    name: String,
    #[serde(default)]
    mana_cost: String,
    #[serde(default)]
    type_line: String,
    #[serde(default)]
    oracle_text: String,
    #[serde(default)]
    power: Option<String>,
    #[serde(default)]
    toughness: Option<String>,
    #[serde(default)]
    loyalty: Option<String>,
    #[serde(default)]
    defense: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
}

#[derive(Default)]
struct CatalogBuilder {
    total_records: usize,
    english_records: usize,
    identities: BTreeMap<OracleId, IdentityAccumulator>,
    printings: Vec<PrintingRecord>,
}

impl CatalogBuilder {
    fn accept(&mut self, card: ScryfallCard) -> Result<(), String> {
        self.total_records += 1;
        if card.lang != "en" {
            return Ok(());
        }
        self.english_records += 1;
        let layout = CardLayout::parse(&card.layout)
            .ok_or_else(|| format!("unknown Scryfall layout `{}`", card.layout))?;
        let printing_id = PrintingId::parse(card.id.clone())
            .ok_or_else(|| format!("invalid printing id `{}`", card.id))?;
        let identity_text = card
            .oracle_id
            .unwrap_or_else(|| format!("source:{}:{}", layout.as_str(), printing_id.as_str()));
        let oracle_id = OracleId::parse(identity_text.clone())
            .ok_or_else(|| format!("invalid Oracle identity `{identity_text}`"))?;
        let face_names = if card.card_faces.is_empty() {
            vec![card.name.clone()]
        } else {
            card.card_faces
                .iter()
                .map(|face| face.name.clone())
                .collect()
        };
        let classification = source_classification(layout, &card.set_type);
        self.identities
            .entry(oracle_id.clone())
            .and_modify(|identity| {
                identity.merge(&card.name, layout, &face_names, &classification);
            })
            .or_insert_with(|| IdentityAccumulator {
                name: card.name.clone(),
                layout,
                face_names: face_names.clone(),
                classification,
            });
        self.printings.push(PrintingRecord {
            id: printing_id,
            oracle_id,
            name: card.name,
            layout,
            set_code: card.set_code,
            collector_number: card.collector_number,
            released_at: card.released_at,
            face_names,
        });
        Ok(())
    }

    fn finish(mut self, provenance: SourceProvenance) -> Result<CardCatalog, String> {
        self.printings.sort_by(|left, right| left.id.cmp(&right.id));
        for pair in self.printings.windows(2) {
            if pair[0].id == pair[1].id {
                return Err(format!("duplicate printing id `{}`", pair[1].id.as_str()));
            }
        }
        let identities = self
            .identities
            .into_iter()
            .map(|(id, accumulator)| IdentityRecord {
                id,
                name: accumulator.name,
                layout: accumulator.layout,
                face_names: accumulator.face_names,
                classification: accumulator.classification,
            })
            .collect();
        let mut catalog = CardCatalog::empty(provenance);
        catalog.identities = identities;
        catalog.printings = self.printings;
        Ok(catalog)
    }
}

struct CardArraySeed<'a> {
    builder: &'a mut CatalogBuilder,
}

impl<'de> DeserializeSeed<'de> for CardArraySeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(CardArrayVisitor {
            builder: self.builder,
        })
    }
}

struct CardArrayVisitor<'a> {
    builder: &'a mut CatalogBuilder,
}

impl<'de> Visitor<'de> for CardArrayVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a top-level array of Scryfall card objects")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while let Some(card) = sequence.next_element::<ScryfallCard>()? {
            self.builder.accept(card).map_err(A::Error::custom)?;
        }
        Ok(())
    }
}

fn stream_cards(reader: impl Read, builder: &mut CatalogBuilder) -> serde_json::Result<()> {
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    CardArraySeed { builder }.deserialize(&mut deserializer)?;
    deserializer.end()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SelectionEntry {
    name: String,
    stratum: String,
}

#[derive(Debug, Deserialize)]
struct SelectionManifest {
    schema_version: u32,
    cards: Vec<SelectionEntry>,
}

#[derive(Serialize)]
struct SourceSelectionDocument {
    schema_version: u32,
    source: SourceProvenance,
    cards: Vec<SelectedSourceCard>,
}

#[derive(Serialize)]
struct SelectedSourceCard {
    stratum: String,
    source_card: ScryfallCard,
}

fn extract_selection(
    source_path: &Path,
    summary_path: &Path,
    selection_path: &Path,
    output_path: &Path,
) -> Result<String, String> {
    let summary_file = File::open(summary_path)
        .map_err(|error| format!("could not open {}: {error}", summary_path.display()))?;
    let summary: ScryfallSummary = serde_json::from_reader(BufReader::new(summary_file))
        .map_err(|error| format!("invalid {}: {error}", summary_path.display()))?;
    validate_source_metadata(source_path, &summary)?;

    let selection_file = File::open(selection_path)
        .map_err(|error| format!("could not open {}: {error}", selection_path.display()))?;
    let selection: SelectionManifest = serde_json::from_reader(BufReader::new(selection_file))
        .map_err(|error| format!("invalid {}: {error}", selection_path.display()))?;
    if selection.schema_version != 1 {
        return Err(format!(
            "unsupported selection schema {}",
            selection.schema_version
        ));
    }
    let targets = selection
        .cards
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<BTreeSet<_>>();
    if targets.len() != selection.cards.len() {
        return Err("selection contains duplicate card names".to_string());
    }
    if targets.is_empty() {
        return Err("selection contains no cards".to_string());
    }

    let source_file = File::open(source_path)
        .map_err(|error| format!("could not open {}: {error}", source_path.display()))?;
    let mut builder = SelectionBuilder {
        targets: &targets,
        found: BTreeMap::new(),
    };
    stream_selected_cards(BufReader::new(source_file), &mut builder)
        .map_err(|error| format!("invalid {}: {error}", source_path.display()))?;

    let missing = targets
        .iter()
        .filter(|name| !builder.found.contains_key(*name))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!("selection names not found: {}", missing.join(", ")));
    }
    let cards = selection
        .cards
        .into_iter()
        .map(|entry| {
            let source_card = builder
                .found
                .remove(&entry.name)
                .ok_or_else(|| format!("selection disappeared for {}", entry.name))?;
            Ok(SelectedSourceCard {
                stratum: entry.stratum,
                source_card,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let document = SourceSelectionDocument {
        schema_version: 1,
        source: SourceProvenance {
            source: summary.source,
            source_path: repository_relative(source_path),
            source_updated_at: summary.source_updated_at,
            source_sha256: summary.local_sha256,
            generator: format!("forge-porttools {}", env!("CARGO_PKG_VERSION")),
        },
        cards,
    };
    write_json(output_path, &document)?;
    Ok(format!(
        "extracted {} selected source card(s) into {}\n",
        document.cards.len(),
        output_path.display()
    ))
}

struct SelectionBuilder<'a> {
    targets: &'a BTreeSet<String>,
    found: BTreeMap<String, ScryfallCard>,
}

impl SelectionBuilder<'_> {
    fn accept(&mut self, card: ScryfallCard) {
        if card.lang != "en" || !self.targets.contains(&card.name) {
            return;
        }
        match self.found.get(&card.name) {
            Some(current) if !prefer_selection(&card, current) => {}
            _ => {
                self.found.insert(card.name.clone(), card);
            }
        }
    }
}

struct SelectionArraySeed<'a, 'b> {
    builder: &'a mut SelectionBuilder<'b>,
}

impl<'de> DeserializeSeed<'de> for SelectionArraySeed<'_, '_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(SelectionArrayVisitor {
            builder: self.builder,
        })
    }
}

struct SelectionArrayVisitor<'a, 'b> {
    builder: &'a mut SelectionBuilder<'b>,
}

impl<'de> Visitor<'de> for SelectionArrayVisitor<'_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a top-level array of Scryfall card objects")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while let Some(card) = sequence.next_element::<ScryfallCard>()? {
            self.builder.accept(card);
        }
        Ok(())
    }
}

fn stream_selected_cards(
    reader: impl Read,
    builder: &mut SelectionBuilder<'_>,
) -> serde_json::Result<()> {
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    SelectionArraySeed { builder }.deserialize(&mut deserializer)?;
    deserializer.end()
}

fn prefer_selection(candidate: &ScryfallCard, current: &ScryfallCard) -> bool {
    let candidate_rank = classification_rank(&source_classification(
        CardLayout::parse(&candidate.layout).unwrap_or(CardLayout::ArtSeries),
        &candidate.set_type,
    ));
    let current_rank = classification_rank(&source_classification(
        CardLayout::parse(&current.layout).unwrap_or(CardLayout::ArtSeries),
        &current.set_type,
    ));
    candidate_rank > current_rank || (candidate_rank == current_rank && candidate.id < current.id)
}

const fn classification_rank(classification: &CardClassification) -> u8 {
    match classification {
        CardClassification::VerifiedPlayable => 4,
        CardClassification::UnverifiedPlayable => 3,
        CardClassification::Quarantined(_) => 2,
        CardClassification::OutOfV1(_) => 1,
        CardClassification::CatalogOnly(_) => 0,
    }
}

#[derive(Clone, Debug)]
struct IdentityAccumulator {
    name: String,
    layout: CardLayout,
    face_names: Vec<String>,
    classification: CardClassification,
}

impl IdentityAccumulator {
    fn merge(
        &mut self,
        name: &str,
        layout: CardLayout,
        face_names: &[String],
        classification: &CardClassification,
    ) {
        if name < self.name.as_str() {
            self.name = name.to_string();
        }
        if layout_rank(layout) > layout_rank(self.layout) {
            self.layout = layout;
        }
        if face_names.len() > self.face_names.len()
            || (face_names.len() == self.face_names.len()
                && face_names < self.face_names.as_slice())
        {
            self.face_names = face_names.to_vec();
        }
        self.classification = merge_classification(&self.classification, classification);
    }
}

fn source_classification(layout: CardLayout, set_type: &str) -> CardClassification {
    if set_type == "token" {
        return CardClassification::CatalogOnly(
            "token-set records are catalog metadata, not independent playable cards".to_string(),
        );
    }
    if layout.catalog_only_by_default() {
        return CardClassification::CatalogOnly(format!(
            "{} records are catalog metadata, not independent playable cards",
            layout.as_str()
        ));
    }
    if set_type == "funny" {
        return CardClassification::OutOfV1(
            "silver-border, acorn, or other non-eternal mechanics".to_string(),
        );
    }
    if matches!(
        layout,
        CardLayout::Host
            | CardLayout::Augment
            | CardLayout::Prepare
            | CardLayout::Planar
            | CardLayout::Scheme
            | CardLayout::Vanguard
    ) {
        return CardClassification::OutOfV1(format!(
            "{} rules are outside the Forge v1 game-mode scope",
            layout.as_str()
        ));
    }
    CardClassification::UnverifiedPlayable
}

fn merge_classification(
    left: &CardClassification,
    right: &CardClassification,
) -> CardClassification {
    if matches!(
        (left, right),
        (CardClassification::VerifiedPlayable, _) | (_, CardClassification::VerifiedPlayable)
    ) {
        return CardClassification::VerifiedPlayable;
    }
    if matches!(
        (left, right),
        (CardClassification::UnverifiedPlayable, _) | (_, CardClassification::UnverifiedPlayable)
    ) {
        return CardClassification::UnverifiedPlayable;
    }
    match (left, right) {
        (CardClassification::Quarantined(reason), _)
        | (_, CardClassification::Quarantined(reason)) => {
            CardClassification::Quarantined(reason.clone())
        }
        (CardClassification::OutOfV1(reason), _) | (_, CardClassification::OutOfV1(reason)) => {
            CardClassification::OutOfV1(reason.clone())
        }
        (CardClassification::CatalogOnly(reason), CardClassification::CatalogOnly(_)) => {
            CardClassification::CatalogOnly(reason.clone())
        }
        _ => CardClassification::UnverifiedPlayable,
    }
}

fn layout_rank(layout: CardLayout) -> u8 {
    match layout {
        CardLayout::Normal => 0,
        CardLayout::Token
        | CardLayout::DoubleFacedToken
        | CardLayout::Emblem
        | CardLayout::ArtSeries => 1,
        CardLayout::Host
        | CardLayout::Augment
        | CardLayout::Prepare
        | CardLayout::Planar
        | CardLayout::Scheme
        | CardLayout::Vanguard => 2,
        CardLayout::Split
        | CardLayout::Flip
        | CardLayout::Transform
        | CardLayout::ModalDfc
        | CardLayout::Meld
        | CardLayout::Adventure
        | CardLayout::Leveler
        | CardLayout::Class
        | CardLayout::Case
        | CardLayout::Saga
        | CardLayout::Mutate
        | CardLayout::Prototype
        | CardLayout::ReversibleCard => 3,
    }
}

fn validate_source_metadata(path: &Path, summary: &ScryfallSummary) -> Result<(), String> {
    let size = fs::metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?
        .len();
    if size != summary.local_size_bytes {
        return Err(format!(
            "source size {size} does not match summary {}",
            summary.local_size_bytes
        ));
    }
    if summary.local_sha256.len() != 64
        || !summary
            .local_sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err("summary local_sha256 is not a 64-digit hex SHA-256".to_string());
    }
    Ok(())
}

fn validate_catalog(catalog: &CardCatalog) -> Result<(), String> {
    let identity_ids = catalog
        .identities
        .iter()
        .map(|identity| identity.id.as_str())
        .collect::<BTreeSet<_>>();
    if identity_ids.len() != catalog.identities.len() {
        return Err("catalog has duplicate identity ids".to_string());
    }
    for pair in catalog.identities.windows(2) {
        if pair[0].id >= pair[1].id {
            return Err("catalog identities are not strictly sorted".to_string());
        }
    }
    let mut printing_ids = BTreeSet::new();
    for printing in &catalog.printings {
        if !printing_ids.insert(printing.id.as_str()) {
            return Err(format!("duplicate printing `{}`", printing.id.as_str()));
        }
        if !identity_ids.contains(printing.oracle_id.as_str()) {
            return Err(format!(
                "printing {} references missing identity {}",
                printing.id.as_str(),
                printing.oracle_id.as_str()
            ));
        }
    }
    for pair in catalog.printings.windows(2) {
        if pair[0].id >= pair[1].id {
            return Err("catalog printings are not strictly sorted".to_string());
        }
    }
    Ok(())
}

#[derive(Default)]
struct ClassificationCounts {
    verified_playable: usize,
    unverified_playable: usize,
    quarantined: usize,
    out_of_v1: usize,
    catalog_only: usize,
}

fn classification_counts(catalog: &CardCatalog) -> ClassificationCounts {
    let mut counts = ClassificationCounts::default();
    for identity in &catalog.identities {
        match identity.classification {
            CardClassification::VerifiedPlayable => counts.verified_playable += 1,
            CardClassification::UnverifiedPlayable => counts.unverified_playable += 1,
            CardClassification::Quarantined(_) => counts.quarantined += 1,
            CardClassification::OutOfV1(_) => counts.out_of_v1 += 1,
            CardClassification::CatalogOnly(_) => counts.catalog_only += 1,
        }
    }
    counts
}

#[derive(Serialize)]
struct CatalogMetrics {
    schema_version: u32,
    source: SourceProvenance,
    source_records: usize,
    source_english_records: usize,
    imported_english_printings: usize,
    source_unique_english_oracle_ids: usize,
    source_fallback_identities: usize,
    expected_classified_identities: usize,
    classified_identities: usize,
    verified_playable: usize,
    unverified_playable: usize,
    quarantined: usize,
    out_of_v1: usize,
    catalog_only: usize,
    dangling_printing_references: usize,
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let file = File::create(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, value)
        .map_err(|error| format!("could not serialize {}: {error}", path.display()))?;
    writer
        .write_all(b"\n")
        .and_then(|()| writer.flush())
        .map_err(|error| format!("could not finish {}: {error}", path.display()))
}

fn repository_relative(path: &Path) -> String {
    if path.is_relative() {
        return path.display().to_string();
    }
    env::current_dir()
        .ok()
        .and_then(|root| path.strip_prefix(root).ok().map(Path::to_path_buf))
        .map_or_else(
            || path.display().to_string(),
            |path| path.display().to_string(),
        )
}

#[cfg(test)]
mod tests {
    use super::{
        optional_bool_flag, optional_usize_flag, parse_named_flags, required_usize_flag,
        source_classification, stream_cards, validate_local_worker_count, CatalogBuilder,
        MAX_LOCAL_WORKERS,
    };
    use forge_carddef::{CardClassification, CardLayout};
    use std::io::Cursor;

    const SOURCE: &str = r#"[
      {"id":"print-a","oracle_id":"oracle-a","name":"Alpha","lang":"en","released_at":"2026-01-01","layout":"normal","set":"tst","collector_number":"1","set_type":"expansion"},
      {"id":"print-b","name":"Token","lang":"en","released_at":"2026-01-01","layout":"token","set":"ttk","collector_number":"2","set_type":"token"},
      {"id":"print-c","oracle_id":"oracle-c","name":"Ignored","lang":"ja","released_at":"2026-01-01","layout":"normal","set":"tst","collector_number":"3","set_type":"expansion"}
    ]"#;

    #[test]
    fn streams_english_records_and_classifies_fallback_identity() {
        let mut builder = CatalogBuilder::default();
        if let Err(error) = stream_cards(Cursor::new(SOURCE), &mut builder) {
            panic!("{error}");
        }
        assert_eq!(builder.total_records, 3);
        assert_eq!(builder.english_records, 2);
        assert_eq!(builder.identities.len(), 2);
        assert!(builder
            .identities
            .keys()
            .any(|id| id.as_str().starts_with("source:token:print-b")));
        assert!(builder
            .identities
            .values()
            .any(|identity| matches!(identity.classification, CardClassification::CatalogOnly(_))));
    }

    #[test]
    fn token_set_records_are_catalog_only_even_with_normal_layout() {
        assert!(matches!(
            source_classification(CardLayout::Normal, "token"),
            CardClassification::CatalogOnly(_)
        ));
    }

    #[test]
    fn parses_configurable_parallel_sweep_flags() {
        let args = [
            "--jobs".to_string(),
            "12".to_string(),
            "--output".to_string(),
            "target/secondary".to_string(),
        ];
        let flags = parse_named_flags(&args, &["--jobs", "--output"])
            .unwrap_or_else(|error| panic!("valid flags should parse: {error}"));
        assert_eq!(required_usize_flag(&flags, "--jobs", "workers"), Ok(12));
        assert_eq!(optional_usize_flag(&flags, "--batch-size", 5), Ok(5));
        assert_eq!(optional_bool_flag(&flags, "--write-output", true), Ok(true));

        let bool_args = ["--write-output".to_string(), "false".to_string()];
        let bool_flags = parse_named_flags(&bool_args, &["--write-output"])
            .unwrap_or_else(|error| panic!("valid bool flag should parse: {error}"));
        assert_eq!(
            optional_bool_flag(&bool_flags, "--write-output", true),
            Ok(false)
        );
        let invalid_bool_args = ["--write-output".to_string(), "yes".to_string()];
        let invalid_bool_flags = parse_named_flags(&invalid_bool_args, &["--write-output"])
            .unwrap_or_else(|error| panic!("named flag should parse: {error}"));
        assert!(optional_bool_flag(&invalid_bool_flags, "--write-output", true).is_err());

        let duplicate = [
            "--jobs".to_string(),
            "6".to_string(),
            "--jobs".to_string(),
            "8".to_string(),
        ];
        assert!(parse_named_flags(&duplicate, &["--jobs"]).is_err());
        assert!(parse_named_flags(&args, &["--jobs"]).is_err());
        assert!(parse_named_flags(&args[..1], &["--jobs"]).is_err());
    }

    #[test]
    fn enforces_the_local_worker_ceiling() {
        assert_eq!(validate_local_worker_count("translation", 1), Ok(()));
        assert_eq!(
            validate_local_worker_count("translation", MAX_LOCAL_WORKERS),
            Ok(())
        );
        assert!(validate_local_worker_count("translation", 0).is_err());
        let error = match validate_local_worker_count("translation", MAX_LOCAL_WORKERS + 1) {
            Ok(()) => panic!("oversubscription must fail closed"),
            Err(error) => error,
        };
        assert!(error.contains("exceed the local worker ceiling 24"));
    }
}
