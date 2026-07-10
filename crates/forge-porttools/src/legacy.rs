//! Lossless line-level parser for vendored legacy Forge card scripts.

use serde::Serialize;
use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
};

const TARGET_PARSE_RATE_BASIS_POINTS: usize = 9_950;

/// One parsed legacy card script.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LegacyScript {
    /// Repository-relative or caller-provided source name.
    pub source: String,
    /// Parsed non-comment lines in source order.
    pub lines: Vec<LegacyLine>,
    /// Number of faces separated by `ALTERNATE` markers.
    pub face_count: usize,
}

/// One positioned line in a legacy script.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LegacyLine {
    /// One-based source line number.
    pub line: usize,
    /// Parsed line payload.
    pub kind: LegacyLineKind,
}

/// Closed structural classification for a legacy script line.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LegacyLineKind {
    /// Multi-face boundary marker.
    Alternate,
    /// `A:`, `T:`, `R:`, or `S:` ability declaration.
    Ability {
        /// Legacy ability family.
        prefix: LegacyAbilityPrefix,
        /// Pipe-delimited legacy expression.
        expression: LegacyExpression,
    },
    /// `K:` keyword declaration.
    Keyword {
        /// Closed keyword head before colon-separated arguments.
        name: String,
        /// Keyword arguments retained in source order.
        arguments: Vec<String>,
        /// Original keyword payload after `K:`.
        raw: String,
    },
    /// `SVar:<name>:<value>` declaration.
    SVar {
        /// SVar identifier.
        name: String,
        /// Pipe-delimited value retained without semantic interpretation.
        expression: LegacyExpression,
    },
    /// Any other key/value property, retained for later mapping stages.
    Property {
        /// Property key before the first colon.
        key: String,
        /// Unmodified trimmed property value.
        value: String,
    },
}

/// Legacy ability line family.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyAbilityPrefix {
    /// Activated ability (`A:`).
    Activated,
    /// Triggered ability (`T:`).
    Triggered,
    /// Replacement effect (`R:`).
    Replacement,
    /// Static ability (`S:`).
    Static,
}

impl LegacyAbilityPrefix {
    const fn parse(key: &str) -> Option<Self> {
        match key.as_bytes() {
            b"A" => Some(Self::Activated),
            b"T" => Some(Self::Triggered),
            b"R" => Some(Self::Replacement),
            b"S" => Some(Self::Static),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Activated => "A",
            Self::Triggered => "T",
            Self::Replacement => "R",
            Self::Static => "S",
        }
    }
}

/// One pipe-delimited legacy expression.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LegacyExpression {
    /// Original trimmed expression text.
    pub raw: String,
    /// Pipe fields in source order.
    pub fields: Vec<LegacyPipeField>,
}

/// One field inside a legacy pipe expression.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LegacyPipeField {
    /// Optional key before the first `$` separator.
    pub key: Option<String>,
    /// Field value after `$`, or the complete field when no key exists.
    pub value: String,
}

/// Positioned parse diagnostic for one malformed legacy file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LegacyParseDiagnostic {
    /// One-based source line.
    pub line: usize,
    /// One-based source column.
    pub column: usize,
    /// Stable machine-readable reason code.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
}

impl fmt::Display for LegacyParseDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{} [{}] {}",
            self.line, self.column, self.code, self.message
        )
    }
}

/// One failed file in a full-corpus parse audit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LegacyFileFailure {
    /// Path relative to the audited cards root.
    pub path: String,
    /// First fail-closed diagnostic for the file.
    pub diagnostic: LegacyParseDiagnostic,
}

/// Deterministic metrics produced by a full legacy-corpus parse audit.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LegacyParseAuditReport {
    /// Metrics schema version.
    pub schema_version: u32,
    /// Audited cards root.
    pub source_root: String,
    /// Exact local legacy Forge revision containing the cards root.
    pub source_revision: String,
    /// Number of `.txt` scripts discovered.
    pub total_files: usize,
    /// Number of scripts parsed without diagnostics.
    pub parsed_files: usize,
    /// Number of scripts rejected.
    pub failed_files: usize,
    /// Parse-only file coverage percentage.
    pub parse_rate_percent: f64,
    /// Required parse-only coverage percentage.
    pub target_parse_rate_percent: f64,
    /// Non-comment source lines encountered in passing files.
    pub parsed_lines: usize,
    /// Ability line counts keyed by `A`, `T`, `R`, and `S`.
    pub ability_lines_by_prefix: BTreeMap<String, usize>,
    /// Parsed `K:` lines.
    pub keyword_lines: usize,
    /// Parsed `SVar:` lines.
    pub svar_lines: usize,
    /// Parsed metadata property lines.
    pub property_lines: usize,
    /// Parsed multi-face boundaries.
    pub alternate_markers: usize,
    /// Files decoded lossily because of invalid UTF-8.
    pub lossy_utf8_files: usize,
    /// Failure counts keyed by stable diagnostic code.
    pub failure_reason_counts: BTreeMap<String, usize>,
    /// Whether the 99.5% parse-only floor passed.
    pub passed: bool,
}

#[derive(Serialize)]
struct LegacyFailureDocument<'a> {
    schema_version: u32,
    source_root: &'a str,
    failures: &'a [LegacyFileFailure],
}

/// Parses one legacy script into a lossless structural AST.
pub fn parse_legacy_script(
    source: impl Into<String>,
    text: &str,
) -> Result<LegacyScript, LegacyParseDiagnostic> {
    let mut lines = Vec::new();
    let mut face_count = 1;
    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = raw_line.trim_start_matches('\u{feff}').trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let kind = if trimmed == "ALTERNATE" {
            face_count += 1;
            LegacyLineKind::Alternate
        } else {
            parse_keyed_line(trimmed, line_number)?
        };
        lines.push(LegacyLine {
            line: line_number,
            kind,
        });
    }
    Ok(LegacyScript {
        source: source.into(),
        lines,
        face_count,
    })
}

/// Audits every `.txt` script below a local legacy cards root.
pub fn audit_legacy_corpus(
    root: &Path,
    metrics_path: &Path,
    failures_path: &Path,
) -> Result<LegacyParseAuditReport, String> {
    if !root.is_dir() {
        return Err(format!(
            "legacy cards root is not a directory: {}",
            root.display()
        ));
    }
    let mut paths = Vec::new();
    collect_scripts(root, &mut paths)?;
    paths.sort();
    if paths.is_empty() {
        return Err(format!(
            "legacy cards root contains no .txt files: {}",
            root.display()
        ));
    }

    let mut parsed_files = 0;
    let mut parsed_lines = 0;
    let mut ability_lines_by_prefix = BTreeMap::new();
    let mut keyword_lines = 0;
    let mut svar_lines = 0;
    let mut property_lines = 0;
    let mut alternate_markers = 0;
    let mut lossy_utf8_files = 0;
    let mut failures = Vec::new();
    let mut failure_reason_counts = BTreeMap::new();

    for path in &paths {
        let bytes = fs::read(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        if matches!(text, std::borrow::Cow::Owned(_)) {
            lossy_utf8_files += 1;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        match parse_legacy_script(relative.clone(), &text) {
            Ok(script) => {
                parsed_files += 1;
                parsed_lines += script.lines.len();
                for line in script.lines {
                    match line.kind {
                        LegacyLineKind::Ability { prefix, .. } => {
                            *ability_lines_by_prefix
                                .entry(prefix.as_str().to_string())
                                .or_insert(0) += 1;
                        }
                        LegacyLineKind::Keyword { .. } => keyword_lines += 1,
                        LegacyLineKind::SVar { .. } => svar_lines += 1,
                        LegacyLineKind::Property { .. } => property_lines += 1,
                        LegacyLineKind::Alternate => alternate_markers += 1,
                    }
                }
            }
            Err(diagnostic) => {
                *failure_reason_counts
                    .entry(diagnostic.code.clone())
                    .or_insert(0) += 1;
                failures.push(LegacyFileFailure {
                    path: relative,
                    diagnostic,
                });
            }
        }
    }

    let total_files = paths.len();
    let failed_files = failures.len();
    let parse_rate_percent = parsed_files as f64 * 100.0 / total_files as f64;
    let passed = parsed_files * 10_000 >= total_files * TARGET_PARSE_RATE_BASIS_POINTS;
    let source_root = super::repository_relative(root);
    let source_revision = git_revision(root)?;
    let report = LegacyParseAuditReport {
        schema_version: 1,
        source_root: source_root.clone(),
        source_revision,
        total_files,
        parsed_files,
        failed_files,
        parse_rate_percent,
        target_parse_rate_percent: 99.5,
        parsed_lines,
        ability_lines_by_prefix,
        keyword_lines,
        svar_lines,
        property_lines,
        alternate_markers,
        lossy_utf8_files,
        failure_reason_counts,
        passed,
    };
    super::write_json(metrics_path, &report)?;
    super::write_json(
        failures_path,
        &LegacyFailureDocument {
            schema_version: 1,
            source_root: &source_root,
            failures: &failures,
        },
    )?;
    Ok(report)
}

fn parse_keyed_line(
    line: &str,
    line_number: usize,
) -> Result<LegacyLineKind, LegacyParseDiagnostic> {
    let Some((key, value)) = line.split_once(':') else {
        return Err(diagnostic(
            line_number,
            1,
            "missing_key_separator",
            "expected a key/value line or ALTERNATE marker",
        ));
    };
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() {
        return Err(diagnostic(
            line_number,
            1,
            "empty_key",
            "legacy property key is empty",
        ));
    }
    if let Some(prefix) = LegacyAbilityPrefix::parse(key) {
        if value.is_empty() {
            return Err(diagnostic(
                line_number,
                key.len() + 2,
                "empty_ability",
                "legacy ability payload is empty",
            ));
        }
        return Ok(LegacyLineKind::Ability {
            prefix,
            expression: parse_expression(value),
        });
    }
    if key == "K" {
        if value.is_empty() {
            return Err(diagnostic(
                line_number,
                3,
                "empty_keyword",
                "legacy keyword payload is empty",
            ));
        }
        let mut parts = value.split(':').map(str::trim);
        let name = parts.next().unwrap_or_default().to_string();
        let arguments = parts.map(ToString::to_string).collect();
        return Ok(LegacyLineKind::Keyword {
            name,
            arguments,
            raw: value.to_string(),
        });
    }
    if key == "SVar" {
        let Some((name, expression)) = value.split_once(':') else {
            return Err(diagnostic(
                line_number,
                6,
                "missing_svar_value_separator",
                "SVar requires SVar:<name>:<value>",
            ));
        };
        let name = name.trim();
        if name.is_empty() {
            return Err(diagnostic(
                line_number,
                6,
                "empty_svar_name",
                "SVar name is empty",
            ));
        }
        return Ok(LegacyLineKind::SVar {
            name: name.to_string(),
            expression: parse_expression(expression.trim()),
        });
    }
    Ok(LegacyLineKind::Property {
        key: key.to_string(),
        value: value.to_string(),
    })
}

fn parse_expression(raw: &str) -> LegacyExpression {
    LegacyExpression {
        raw: raw.to_string(),
        fields: raw
            .split('|')
            .map(|field| {
                let field = field.trim();
                field.split_once('$').map_or_else(
                    || LegacyPipeField {
                        key: None,
                        value: field.to_string(),
                    },
                    |(key, value)| LegacyPipeField {
                        key: Some(key.trim().to_string()),
                        value: value.trim().to_string(),
                    },
                )
            })
            .collect(),
    }
}

fn diagnostic(line: usize, column: usize, code: &str, message: &str) -> LegacyParseDiagnostic {
    LegacyParseDiagnostic {
        line,
        column,
        code: code.to_string(),
        message: message.to_string(),
    }
}

fn collect_scripts(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(root)
        .map_err(|error| format!("could not read directory {}: {error}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not read directory entry: {error}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
        if file_type.is_dir() {
            collect_scripts(&path, paths)?;
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        {
            paths.push(path);
        }
    }
    Ok(())
}

fn git_revision(root: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("could not query legacy source revision: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not query legacy source revision for {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let revision = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "legacy source returned invalid revision `{revision}`"
        ));
    }
    Ok(revision)
}

#[cfg(test)]
mod tests {
    use super::{parse_legacy_script, LegacyAbilityPrefix, LegacyLineKind};

    #[test]
    fn parses_required_line_families_and_alternate_faces() {
        let source = r#"Name:Front
A:AB$ Mana | Cost$ T | Produced$ G
T:Mode$ ChangesZone | Execute$ Trig
R:Event$ Moved | ReplaceWith$ Repl
S:Mode$ Continuous | AddPower$ 1
K:TypeCycling:Basic:2
SVar:Trig:DB$ Draw | NumCards$ 1
DeckHas:Ability$Draw
ALTERNATE
Name:Back
"#;
        let script = parse_legacy_script("fixture.txt", source).unwrap_or_else(|error| {
            panic!("fixture should parse: {error}");
        });
        assert_eq!(script.face_count, 2);
        assert_eq!(script.lines.len(), 10);
        assert!(matches!(
            script.lines[1].kind,
            LegacyLineKind::Ability {
                prefix: LegacyAbilityPrefix::Activated,
                ..
            }
        ));
        assert!(matches!(
            script.lines[5].kind,
            LegacyLineKind::Keyword { .. }
        ));
        assert!(matches!(script.lines[6].kind, LegacyLineKind::SVar { .. }));
        assert!(matches!(
            script.lines[7].kind,
            LegacyLineKind::Property { .. }
        ));
        assert!(matches!(script.lines[8].kind, LegacyLineKind::Alternate));
    }

    #[test]
    fn rejects_unstructured_and_unnamed_svar_lines() {
        let missing_separator = match parse_legacy_script("bad.txt", "not a legacy line") {
            Err(error) => error,
            Ok(_) => panic!("unstructured line must fail"),
        };
        assert_eq!(missing_separator.code, "missing_key_separator");

        let empty_svar = match parse_legacy_script("bad.txt", "SVar::DB$ Draw") {
            Err(error) => error,
            Ok(_) => panic!("unnamed SVar must fail"),
        };
        assert_eq!(empty_svar.code, "empty_svar_name");
    }
}
