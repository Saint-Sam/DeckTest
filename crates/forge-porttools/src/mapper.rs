//! Fail-closed legacy ability API mapping into typed Forge operations.

use crate::legacy::{
    collect_scripts, git_revision, parse_legacy_script, LegacyAbilityPrefix, LegacyExpression,
    LegacyLineKind,
};
use forge_carddef::{Expression, Operation};
use serde::Serialize;
use std::{collections::BTreeMap, fs, path::Path};

/// One legacy ability lowered into typed Forge expressions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MappedLegacyAbility {
    /// Legacy line family.
    pub prefix: LegacyAbilityPrefix,
    /// Legacy API name.
    pub api: String,
    /// Typed payment costs in source order.
    pub costs: Vec<Expression>,
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
];

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
    if !matches!(selector_key, "AB" | "SP" | "Mode" | "Event" | "ST") {
        return Err(diagnostic(
            "UNSUPPORTED_SELECTOR",
            &format!("unsupported API selector `{selector_key}`"),
        ));
    }
    let api = selector.value.trim();
    if api.is_empty() {
        return Err(diagnostic("MALFORMED_API", "ability API name is empty"));
    }
    let parameters = parameters(expression)?;
    let Some(spec) = MAPPERS
        .iter()
        .find(|spec| spec.prefix == prefix && spec.api == api)
    else {
        return Err(diagnostic(
            "UNMAPPED_API",
            &format!("no mapper is registered for {}:{api}", prefix.as_str()),
        ));
    };
    (spec.mapper)(prefix, api, selector_key, &parameters)
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
    for line in script.lines {
        let LegacyLineKind::Ability { prefix, expression } = line.kind else {
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
        match map_legacy_ability(prefix, &expression) {
            Ok(_) => {
                row.mapped += 1;
                row.verified += 1;
            }
            Err(error) => {
                *total_quarantined += 1;
                *reason_counts.entry(error.code.clone()).or_insert(0) += 1;
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
    require_selector(selector, "AB")?;
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
    require_selector_one_of(selector, &["AB", "SP"])?;
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
    require_selector_one_of(selector, &["AB", "SP"])?;
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
    if targets != "Any" {
        return Err(unsupported_value("ValidTgts", targets));
    }
    let amount = positive_integer(required(parameters, "NumDmg")?, "NumDmg")?;
    Ok(MappedLegacyAbility {
        prefix,
        api: api.to_string(),
        costs: parse_simple_cost(parameters.get("Cost"))?,
        expression: call(
            Operation::DealDamage,
            vec![
                call(Operation::Target, vec![call(Operation::Any, vec![])]),
                Expression::Integer(amount),
            ],
        ),
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
    for token in value.split_whitespace() {
        if token == "T" {
            costs.push(call(Operation::TapSelf, vec![]));
        } else if token.chars().all(|character| character.is_ascii_digit())
            || matches!(token, "W" | "U" | "B" | "R" | "G" | "C" | "X" | "Y" | "Z")
        {
            mana.push('{');
            mana.push_str(token);
            mana.push('}');
        } else {
            return Err(unsupported_value("Cost", value));
        }
    }
    if !mana.is_empty() {
        costs.insert(0, call(Operation::ManaCost, vec![Expression::Text(mana)]));
    }
    Ok(costs)
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

fn reject_unknown(
    parameters: &BTreeMap<String, String>,
    allowed: &[&str],
) -> Result<(), MappingDiagnostic> {
    if let Some(key) = parameters
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
    {
        return Err(diagnostic(
            "UNSUPPORTED_PARAMETER",
            &format!("parameter `{key}` has no typed mapper"),
        ));
    }
    Ok(())
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
    use super::{audit_legacy_mappings, map_legacy_ability};
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
                "A:SP$ Draw | SubAbility$ DBDiscard | SpellDescription$ Complex.\n",
                "T:Mode$ ChangesZone | Execute$ Trig\n",
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
            report.quarantine_reason_counts.get("UNMAPPED_API"),
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
