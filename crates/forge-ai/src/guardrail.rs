use std::{collections::BTreeMap, error::Error, fmt};

const BUNDLED_PRIORS: &str = include_str!("../../../assets/ai/action_priors.ron");
const RISK_COUNT: usize = 7;
const EXPECTED_RISK_ORDER: &str = "friendly_harm,opponent_benefit,unnecessary_sacrifice,missed_required_defense,unfavorable_combat_trade,pass_with_development,nonterminal_concession";
const PROFILE_NAMES: [&str; 4] = ["novice", "standard", "expert", "master"];

/// Card-agnostic action-risk family used only for prior penalties.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ActionRisk {
    /// A harmful operation is pointed at the actor's own resource.
    FriendlyHarm = 0,
    /// A beneficial operation is pointed at an opponent's resource.
    OpponentBenefit = 1,
    /// A higher-value resource is sacrificed for no compensating value.
    UnnecessarySacrifice = 2,
    /// The action ignores an exact certified required defense.
    MissedRequiredDefense = 3,
    /// Combat loses material without sufficient prevention or trade value.
    UnfavorableCombatTrade = 4,
    /// The actor passes a development-only window despite a useful legal play.
    PassWithDevelopment = 5,
    /// The actor concedes a nonterminal state.
    NonterminalConcession = 6,
}

/// Compact set of typed action risks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ActionRisks(u16);

impl ActionRisks {
    /// Returns an empty risk set.
    #[must_use]
    pub const fn none() -> Self {
        Self(0)
    }

    /// Adds one risk.
    #[must_use]
    pub const fn with(mut self, risk: ActionRisk) -> Self {
        self.0 |= 1_u16 << risk as u8;
        self
    }

    /// Returns whether one risk is present.
    #[must_use]
    pub const fn contains(self, risk: ActionRisk) -> bool {
        self.0 & (1_u16 << risk as u8) != 0
    }
}

/// Difficulty-dependent guardrail profile.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GuardrailProfile {
    /// Lenient noisy baseline.
    Novice,
    /// Standard one-ply baseline.
    Standard,
    /// Strong bounded search.
    Expert,
    /// Highest promoted search tier.
    Master,
}

impl GuardrailProfile {
    const fn key(self) -> &'static str {
        match self {
            Self::Novice => "novice",
            Self::Standard => "standard",
            Self::Expert => "expert",
            Self::Master => "master",
        }
    }
}

/// Versioned table of prior penalties.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardrailTable {
    calibration_status: String,
    profiles: BTreeMap<&'static str, [i64; RISK_COUNT]>,
}

impl GuardrailTable {
    /// Parses the repository's bundled action-prior table.
    pub fn bundled() -> Result<Self, GuardrailError> {
        Self::from_ron_str(BUNDLED_PRIORS)
    }

    /// Parses the strict flat RON-compatible table.
    pub fn from_ron_str(source: &str) -> Result<Self, GuardrailError> {
        let stripped = source
            .lines()
            .map(|line| line.split("//").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        let body = stripped
            .trim()
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
            .ok_or(GuardrailError::InvalidDocument)?;
        let mut schema = None;
        let mut status = None;
        let mut risk_order = None;
        let mut profiles = BTreeMap::new();
        for entry in split_top_level(body)? {
            let (name, raw) = entry
                .split_once(':')
                .ok_or_else(|| GuardrailError::InvalidEntry(entry.to_owned()))?;
            let name = name.trim();
            let raw = raw.trim();
            match name {
                "schema_version" => {
                    let value = raw
                        .parse::<u32>()
                        .map_err(|_| GuardrailError::InvalidEntry(entry.to_owned()))?;
                    if schema.replace(value).is_some() {
                        return Err(GuardrailError::DuplicateField(name.to_owned()));
                    }
                }
                "calibration_status" => {
                    if status.replace(parse_string(raw)?).is_some() {
                        return Err(GuardrailError::DuplicateField(name.to_owned()));
                    }
                }
                "risk_order" => {
                    if risk_order.replace(parse_string(raw)?).is_some() {
                        return Err(GuardrailError::DuplicateField(name.to_owned()));
                    }
                }
                profile if PROFILE_NAMES.contains(&profile) => {
                    let stable_name = PROFILE_NAMES
                        .iter()
                        .copied()
                        .find(|expected| expected == &profile)
                        .ok_or_else(|| GuardrailError::UnknownField(profile.to_owned()))?;
                    if profiles
                        .insert(stable_name, parse_penalties(raw)?)
                        .is_some()
                    {
                        return Err(GuardrailError::DuplicateField(profile.to_owned()));
                    }
                }
                other => return Err(GuardrailError::UnknownField(other.to_owned())),
            }
        }
        let schema = schema.ok_or(GuardrailError::MissingField("schema_version"))?;
        if schema != 1 {
            return Err(GuardrailError::UnsupportedSchema(schema));
        }
        let calibration_status =
            status.ok_or(GuardrailError::MissingField("calibration_status"))?;
        let actual_order = risk_order.ok_or(GuardrailError::MissingField("risk_order"))?;
        if actual_order != EXPECTED_RISK_ORDER {
            return Err(GuardrailError::RiskOrderMismatch(actual_order));
        }
        for profile in PROFILE_NAMES {
            if !profiles.contains_key(profile) {
                return Err(GuardrailError::MissingField(profile));
            }
        }
        Ok(Self {
            calibration_status,
            profiles,
        })
    }

    /// Returns the explicit calibration status.
    #[must_use]
    pub fn calibration_status(&self) -> &str {
        &self.calibration_status
    }

    /// Computes the sum of configured penalties for the supplied risks.
    #[must_use]
    pub fn penalty(&self, profile: GuardrailProfile, risks: ActionRisks) -> i64 {
        let penalties = match self.profiles.get(profile.key()) {
            Some(penalties) => penalties,
            None => unreachable!("validated guardrail table lost a required profile"),
        };
        let all_risks = [
            ActionRisk::FriendlyHarm,
            ActionRisk::OpponentBenefit,
            ActionRisk::UnnecessarySacrifice,
            ActionRisk::MissedRequiredDefense,
            ActionRisk::UnfavorableCombatTrade,
            ActionRisk::PassWithDevelopment,
            ActionRisk::NonterminalConcession,
        ];
        all_risks
            .into_iter()
            .enumerate()
            .filter(|(_, risk)| risks.contains(*risk))
            .fold(0_i64, |total, (index, _)| {
                total.saturating_add(penalties[index])
            })
    }
}

fn parse_penalties(raw: &str) -> Result<[i64; RISK_COUNT], GuardrailError> {
    let body = raw
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| GuardrailError::InvalidPenaltyTuple(raw.to_owned()))?;
    let values = split_top_level(body)?;
    if values.len() != RISK_COUNT {
        return Err(GuardrailError::InvalidPenaltyTuple(raw.to_owned()));
    }
    let mut penalties = [0_i64; RISK_COUNT];
    for (index, raw_value) in values.into_iter().enumerate() {
        let value = raw_value
            .parse::<i64>()
            .map_err(|_| GuardrailError::InvalidPenaltyTuple(raw.to_owned()))?;
        if value > 0 {
            return Err(GuardrailError::PositivePenalty(value));
        }
        penalties[index] = value;
    }
    Ok(penalties)
}

fn split_top_level(input: &str) -> Result<Vec<&str>, GuardrailError> {
    let mut values = Vec::new();
    let mut depth = 0_u32;
    let mut quoted = false;
    let mut start = 0_usize;
    for (index, character) in input.char_indices() {
        match character {
            '"' => quoted = !quoted,
            '(' if !quoted => depth = depth.saturating_add(1),
            ')' if !quoted => {
                depth = depth
                    .checked_sub(1)
                    .ok_or(GuardrailError::InvalidDocument)?;
            }
            ',' if !quoted && depth == 0 => {
                let value = input[start..index].trim();
                if !value.is_empty() {
                    values.push(value);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    if quoted || depth != 0 {
        return Err(GuardrailError::InvalidDocument);
    }
    let tail = input[start..].trim();
    if !tail.is_empty() {
        values.push(tail);
    }
    Ok(values)
}

fn parse_string(raw: &str) -> Result<String, GuardrailError> {
    let value = raw
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| GuardrailError::InvalidString(raw.to_owned()))?;
    if value.contains('"') || value.contains('\\') {
        return Err(GuardrailError::InvalidString(raw.to_owned()));
    }
    Ok(value.to_owned())
}

/// Fail-closed action-prior table errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuardrailError {
    /// The outer document is malformed.
    InvalidDocument,
    /// One top-level entry is malformed.
    InvalidEntry(String),
    /// A required field is absent.
    MissingField(&'static str),
    /// A field appears more than once.
    DuplicateField(String),
    /// An unknown field was supplied.
    UnknownField(String),
    /// The schema version is unsupported.
    UnsupportedSchema(u32),
    /// Risk ordering differs from the compiled typed enum.
    RiskOrderMismatch(String),
    /// A profile tuple is malformed.
    InvalidPenaltyTuple(String),
    /// A penalty attempted to become an action bonus.
    PositivePenalty(i64),
    /// A string literal is malformed.
    InvalidString(String),
}

impl fmt::Display for GuardrailError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid action-prior table: {self:?}")
    }
}

impl Error for GuardrailError {}

#[cfg(test)]
mod tests {
    use super::{ActionRisk, ActionRisks, GuardrailError, GuardrailProfile, GuardrailTable};

    #[test]
    fn bundled_prior_penalties_are_soft_and_tiered() {
        let table = GuardrailTable::bundled()
            .unwrap_or_else(|error| panic!("bundled guardrails should parse: {error}"));
        let risks = ActionRisks::none()
            .with(ActionRisk::FriendlyHarm)
            .with(ActionRisk::UnfavorableCombatTrade);
        let novice = table.penalty(GuardrailProfile::Novice, risks);
        let expert = table.penalty(GuardrailProfile::Expert, risks);
        assert!(novice < 0);
        assert!(expert < novice);
        assert_eq!(
            table.penalty(GuardrailProfile::Master, ActionRisks::none()),
            0
        );
        assert_eq!(table.calibration_status(), "provisional_unpromoted");
    }

    #[test]
    fn parser_rejects_positive_values() {
        let source = r#"(
            schema_version: 1,
            calibration_status: "x",
            risk_order: "friendly_harm,opponent_benefit,unnecessary_sacrifice,missed_required_defense,unfavorable_combat_trade,pass_with_development,nonterminal_concession",
            novice: (1, 0, 0, 0, 0, 0, 0),
            standard: (0, 0, 0, 0, 0, 0, 0),
            expert: (0, 0, 0, 0, 0, 0, 0),
            master: (0, 0, 0, 0, 0, 0, 0),
        )"#;
        assert!(matches!(
            GuardrailTable::from_ron_str(source),
            Err(GuardrailError::PositivePenalty(1))
        ));
    }
}
