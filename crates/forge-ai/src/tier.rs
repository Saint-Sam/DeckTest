use std::{collections::BTreeMap, error::Error, fmt};

const BUNDLED_TIERS: &str = include_str!("../../../assets/ai/ai_tiers.ron");
const EXPECTED_TIER_NAMES: [&str; 5] = ["random", "novice", "standard", "expert", "master"];

/// Stable product difficulty tier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DifficultyTier {
    /// Random-legal diagnostic floor.
    Random,
    /// Noisy one-ply policy.
    Novice,
    /// Smallest competent product policy before the measured search plateau.
    Standard,
    /// Stronger bounded-search policy.
    Expert,
    /// Highest promoted bounded-search policy.
    Master,
}

impl DifficultyTier {
    /// Returns the stable lowercase tier key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Random => "random",
            Self::Novice => "novice",
            Self::Standard => "standard",
            Self::Expert => "expert",
            Self::Master => "master",
        }
    }
}

/// Policy family selected by one difficulty tier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiPolicyFamily {
    /// Seeded uniform random legal action.
    RandomLegal,
    /// One-ply data-driven heuristic.
    Heuristic,
    /// Determinized wall-time-bounded UCT.
    TimedSearch,
}

/// Mulligan behavior configured for one difficulty tier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MulliganQuality {
    /// Seeded random keep or mulligan decisions.
    Random,
    /// Data-driven baseline mulligan policy.
    Baseline,
}

/// Complete data-only definition for one difficulty tier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiTierDefinition {
    policy: AiPolicyFamily,
    think_ms: u32,
    determinizations: u32,
    workers: u32,
    noise_span: i64,
    mulligan_quality: MulliganQuality,
}

impl AiTierDefinition {
    /// Returns the policy family.
    #[must_use]
    pub const fn policy(&self) -> AiPolicyFamily {
        self.policy
    }

    /// Returns the per-tree wall-time budget in milliseconds.
    #[must_use]
    pub const fn think_ms(&self) -> u32 {
        self.think_ms
    }

    /// Returns the hidden-information sample count.
    #[must_use]
    pub const fn determinizations(&self) -> u32 {
        self.determinizations
    }

    /// Returns the local worker ceiling.
    #[must_use]
    pub const fn workers(&self) -> u32 {
        self.workers
    }

    /// Returns the symmetric deterministic heuristic-noise span.
    #[must_use]
    pub const fn noise_span(&self) -> i64 {
        self.noise_span
    }

    /// Returns the configured mulligan behavior.
    #[must_use]
    pub const fn mulligan_quality(&self) -> MulliganQuality {
        self.mulligan_quality
    }
}

/// Parsed versioned difficulty-tier registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiTierSet {
    schema_version: u32,
    calibration_status: String,
    tiers: BTreeMap<&'static str, AiTierDefinition>,
}

impl AiTierSet {
    /// Parses the repository's bundled `ai_tiers.ron` registry.
    pub fn bundled() -> Result<Self, AiTierError> {
        Self::from_ron_str(BUNDLED_TIERS)
    }

    /// Parses the strict data-only tier registry format.
    pub fn from_ron_str(source: &str) -> Result<Self, AiTierError> {
        let stripped = source
            .lines()
            .map(|line| line.split("//").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        let document = stripped.trim();
        let body = document
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
            .ok_or(AiTierError::InvalidDocument)?;
        let mut schema_version = None;
        let mut calibration_status = None;
        let mut tiers = BTreeMap::new();
        for entry in split_top_level(body)? {
            let (name, raw) = entry
                .split_once(':')
                .ok_or_else(|| AiTierError::InvalidEntry(entry.to_owned()))?;
            let name = name.trim();
            let raw = raw.trim();
            match name {
                "schema_version" => {
                    let value = raw
                        .parse::<u32>()
                        .map_err(|_| AiTierError::InvalidEntry(entry.to_owned()))?;
                    if schema_version.replace(value).is_some() {
                        return Err(AiTierError::DuplicateField(name.to_owned()));
                    }
                }
                "calibration_status" => {
                    let value = parse_string(raw)?;
                    if value.is_empty() {
                        return Err(AiTierError::InvalidEntry(entry.to_owned()));
                    }
                    if calibration_status.replace(value).is_some() {
                        return Err(AiTierError::DuplicateField(name.to_owned()));
                    }
                }
                tier_name if EXPECTED_TIER_NAMES.contains(&tier_name) => {
                    let stable_name = EXPECTED_TIER_NAMES
                        .iter()
                        .copied()
                        .find(|expected| expected == &tier_name)
                        .ok_or_else(|| AiTierError::UnknownField(tier_name.to_owned()))?;
                    let definition = parse_tier(raw)?;
                    if tiers.insert(stable_name, definition).is_some() {
                        return Err(AiTierError::DuplicateField(tier_name.to_owned()));
                    }
                }
                other => return Err(AiTierError::UnknownField(other.to_owned())),
            }
        }
        let schema_version = schema_version.ok_or(AiTierError::MissingField("schema_version"))?;
        if schema_version != 1 {
            return Err(AiTierError::UnsupportedSchema(schema_version));
        }
        let calibration_status =
            calibration_status.ok_or(AiTierError::MissingField("calibration_status"))?;
        for name in EXPECTED_TIER_NAMES {
            if !tiers.contains_key(name) {
                return Err(AiTierError::MissingField(name));
            }
        }
        Ok(Self {
            schema_version,
            calibration_status,
            tiers,
        })
    }

    /// Returns the registry schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the explicit calibration status string.
    #[must_use]
    pub fn calibration_status(&self) -> &str {
        &self.calibration_status
    }

    /// Returns one required tier definition.
    #[must_use]
    pub fn tier(&self, tier: DifficultyTier) -> &AiTierDefinition {
        match self.tiers.get(tier.key()) {
            Some(definition) => definition,
            None => unreachable!("validated tier registry lost a required tier"),
        }
    }
}

fn parse_tier(raw: &str) -> Result<AiTierDefinition, AiTierError> {
    let body = raw
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| AiTierError::InvalidTier(raw.to_owned()))?;
    let fields = split_top_level(body)?;
    if fields.len() != 6 {
        return Err(AiTierError::InvalidTier(raw.to_owned()));
    }
    let policy = match parse_string(fields[0])?.as_str() {
        "random_legal" => AiPolicyFamily::RandomLegal,
        "heuristic" => AiPolicyFamily::Heuristic,
        "timed_search" => AiPolicyFamily::TimedSearch,
        other => return Err(AiTierError::InvalidPolicy(other.to_owned())),
    };
    let think_ms = parse_u32(fields[1], "think_ms")?;
    let determinizations = parse_u32(fields[2], "determinizations")?;
    let workers = parse_u32(fields[3], "workers")?;
    let noise_span = fields[4]
        .trim()
        .parse::<i64>()
        .map_err(|_| AiTierError::InvalidNumber("noise_span"))?;
    let mulligan_quality = match parse_string(fields[5])?.as_str() {
        "random" => MulliganQuality::Random,
        "baseline" => MulliganQuality::Baseline,
        other => return Err(AiTierError::InvalidMulligan(other.to_owned())),
    };
    if determinizations == 0 || workers == 0 || workers > 24 || noise_span < 0 {
        return Err(AiTierError::InvalidTier(raw.to_owned()));
    }
    match policy {
        AiPolicyFamily::TimedSearch if think_ms == 0 => {
            return Err(AiTierError::InvalidTier(raw.to_owned()));
        }
        AiPolicyFamily::RandomLegal | AiPolicyFamily::Heuristic if think_ms != 0 => {
            return Err(AiTierError::InvalidTier(raw.to_owned()));
        }
        _ => {}
    }
    match (policy, mulligan_quality) {
        (AiPolicyFamily::RandomLegal, MulliganQuality::Random)
        | (AiPolicyFamily::Heuristic, MulliganQuality::Baseline)
        | (AiPolicyFamily::TimedSearch, MulliganQuality::Baseline) => {}
        _ => return Err(AiTierError::InvalidTier(raw.to_owned())),
    }
    Ok(AiTierDefinition {
        policy,
        think_ms,
        determinizations,
        workers,
        noise_span,
        mulligan_quality,
    })
}

fn split_top_level(input: &str) -> Result<Vec<&str>, AiTierError> {
    let mut fields = Vec::new();
    let mut depth = 0_u32;
    let mut quoted = false;
    let mut escaped = false;
    let mut start = 0_usize;
    for (index, character) in input.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.checked_sub(1).ok_or(AiTierError::InvalidDocument)?;
            }
            ',' if depth == 0 => {
                let field = input[start..index].trim();
                if !field.is_empty() {
                    fields.push(field);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    if quoted || depth != 0 {
        return Err(AiTierError::InvalidDocument);
    }
    let tail = input[start..].trim();
    if !tail.is_empty() {
        fields.push(tail);
    }
    Ok(fields)
}

fn parse_string(raw: &str) -> Result<String, AiTierError> {
    let value = raw
        .trim()
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| AiTierError::InvalidString(raw.to_owned()))?;
    if value.contains('\\') || value.contains('"') {
        return Err(AiTierError::InvalidString(raw.to_owned()));
    }
    Ok(value.to_owned())
}

fn parse_u32(raw: &str, name: &'static str) -> Result<u32, AiTierError> {
    raw.trim()
        .parse::<u32>()
        .map_err(|_| AiTierError::InvalidNumber(name))
}

/// Fail-closed difficulty-tier registry errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AiTierError {
    /// The outer document is malformed.
    InvalidDocument,
    /// A top-level entry is malformed.
    InvalidEntry(String),
    /// A required field is absent.
    MissingField(&'static str),
    /// A field appears more than once.
    DuplicateField(String),
    /// An unknown field was supplied.
    UnknownField(String),
    /// The schema version is unsupported.
    UnsupportedSchema(u32),
    /// A tier tuple is malformed or internally inconsistent.
    InvalidTier(String),
    /// A string literal is malformed.
    InvalidString(String),
    /// A numeric field is malformed.
    InvalidNumber(&'static str),
    /// The policy family is unknown.
    InvalidPolicy(String),
    /// The mulligan policy is unknown.
    InvalidMulligan(String),
}

impl fmt::Display for AiTierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid AI tier registry: {self:?}")
    }
}

impl Error for AiTierError {}

#[cfg(test)]
mod tests {
    use super::{AiPolicyFamily, AiTierError, AiTierSet, DifficultyTier};

    #[test]
    fn bundled_tiers_are_explicitly_unpromoted() {
        let tiers = AiTierSet::bundled()
            .unwrap_or_else(|error| panic!("bundled tiers should parse: {error}"));
        assert_eq!(tiers.schema_version(), 1);
        assert_eq!(tiers.calibration_status(), "provisional_unpromoted");
        assert_eq!(
            tiers.tier(DifficultyTier::Master).policy(),
            AiPolicyFamily::TimedSearch
        );
        assert_eq!(tiers.tier(DifficultyTier::Master).think_ms(), 100);
    }

    #[test]
    fn parser_rejects_unknown_fields_and_zero_search_budget() {
        let unknown = "(schema_version: 1, calibration_status: \"x\", extra: 1,)";
        assert!(matches!(
            AiTierSet::from_ron_str(unknown),
            Err(AiTierError::UnknownField(field)) if field == "extra"
        ));
        let invalid = r#"(
            schema_version: 1,
            calibration_status: "x",
            random: ("random_legal", 0, 1, 1, 0, "random"),
            novice: ("heuristic", 0, 1, 1, 1, "baseline"),
            standard: ("heuristic", 0, 1, 1, 0, "baseline"),
            expert: ("timed_search", 0, 1, 1, 0, "baseline"),
            master: ("timed_search", 1, 1, 1, 0, "baseline"),
        )"#;
        assert!(matches!(
            AiTierSet::from_ron_str(invalid),
            Err(AiTierError::InvalidTier(_))
        ));
    }
}
