#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Defensive runtime loading and indexed lookup for compiled Forge card data.

/// Data-driven compilation and execution of validated card definitions.
pub mod runtime;

use forge_carddef::{
    CardClassification, CardDatabase, CardDefinition, IdentityRecord, PrintingRecord,
    CARD_DATABASE_MAGIC, CARD_DATABASE_SCHEMA_VERSION,
};
use std::{collections::BTreeMap, error::Error, fmt, fs, path::Path};

const HEADER_LEN: usize = CARD_DATABASE_MAGIC.len() + std::mem::size_of::<u32>();
const MAX_CARD_DATABASE_FILE_BYTES: usize = 128 * 1024 * 1024;
const MAX_CARD_DATABASE_DECODE_BYTES: usize = 256 * 1024 * 1024;

/// A rejected compiled card database.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardDatabaseError {
    message: String,
}

impl CardDatabaseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the diagnostic without display formatting.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CardDatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "card database rejected: {}", self.message)
    }
}

impl Error for CardDatabaseError {}

/// Validated card data plus deterministic lookup indexes.
#[derive(Clone, Debug)]
pub struct LoadedCardDatabase {
    database: CardDatabase,
    identity_index: BTreeMap<String, usize>,
    printing_index: BTreeMap<String, usize>,
    definition_index: BTreeMap<String, usize>,
    name_index: BTreeMap<String, Vec<usize>>,
}

impl LoadedCardDatabase {
    /// Returns the validated database payload.
    #[must_use]
    pub const fn database(&self) -> &CardDatabase {
        &self.database
    }

    /// Finds one Oracle identity by its canonical id.
    #[must_use]
    pub fn identity(&self, id: &str) -> Option<&IdentityRecord> {
        self.identity_index
            .get(id)
            .and_then(|index| self.database.identities.get(*index))
    }

    /// Finds one printing by its source printing id.
    #[must_use]
    pub fn printing(&self, id: &str) -> Option<&PrintingRecord> {
        self.printing_index
            .get(id)
            .and_then(|index| self.database.printings.get(*index))
    }

    /// Finds compiled mechanics by Oracle identity.
    #[must_use]
    pub fn definition(&self, oracle_id: &str) -> Option<&CardDefinition> {
        self.definition_index
            .get(oracle_id)
            .and_then(|index| self.database.definitions.get(*index))
    }

    /// Finds all identities whose canonical names match case-insensitively.
    #[must_use]
    pub fn identities_named(&self, name: &str) -> Vec<&IdentityRecord> {
        self.name_index
            .get(&name.to_lowercase())
            .map_or_else(Vec::new, |indexes| {
                indexes
                    .iter()
                    .filter_map(|index| self.database.identities.get(*index))
                    .collect()
            })
    }
}

/// Loads and validates a compiled card database from memory.
pub fn load_card_database(bytes: &[u8]) -> Result<LoadedCardDatabase, CardDatabaseError> {
    if bytes.len() > MAX_CARD_DATABASE_FILE_BYTES {
        return Err(CardDatabaseError::new(format!(
            "file exceeds the {} byte limit",
            MAX_CARD_DATABASE_FILE_BYTES
        )));
    }
    if bytes.len() < HEADER_LEN {
        return Err(CardDatabaseError::new("file is shorter than the header"));
    }
    if bytes.get(..CARD_DATABASE_MAGIC.len()) != Some(CARD_DATABASE_MAGIC.as_slice()) {
        return Err(CardDatabaseError::new("invalid FORGECDB magic"));
    }
    let schema_bytes = bytes
        .get(CARD_DATABASE_MAGIC.len()..HEADER_LEN)
        .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
        .ok_or_else(|| CardDatabaseError::new("missing schema header"))?;
    let header_schema = u32::from_le_bytes(schema_bytes);
    if header_schema != CARD_DATABASE_SCHEMA_VERSION {
        return Err(CardDatabaseError::new(format!(
            "unsupported header schema {header_schema}"
        )));
    }

    let payload = bytes
        .get(HEADER_LEN..)
        .ok_or_else(|| CardDatabaseError::new("missing payload"))?;
    let config = bincode::config::standard()
        .with_fixed_int_encoding()
        .with_little_endian()
        .with_limit::<MAX_CARD_DATABASE_DECODE_BYTES>();
    let (database, consumed): (CardDatabase, usize) =
        bincode::serde::decode_from_slice(payload, config)
            .map_err(|error| CardDatabaseError::new(format!("bincode decode failed: {error}")))?;
    if consumed != payload.len() {
        return Err(CardDatabaseError::new(format!(
            "{} trailing payload byte(s)",
            payload.len() - consumed
        )));
    }
    if database.schema_version != header_schema {
        return Err(CardDatabaseError::new(format!(
            "payload schema {} disagrees with header schema {header_schema}",
            database.schema_version
        )));
    }

    validate_database(&database)?;
    Ok(index_database(database))
}

/// Loads and validates a compiled card database from a file.
pub fn load_card_database_file(
    path: impl AsRef<Path>,
) -> Result<LoadedCardDatabase, CardDatabaseError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|error| {
        CardDatabaseError::new(format!("could not read {}: {error}", path.display()))
    })?;
    load_card_database(&bytes)
}

fn validate_database(database: &CardDatabase) -> Result<(), CardDatabaseError> {
    validate_identity_order(&database.identities)?;
    validate_printing_order(&database.printings)?;
    validate_definition_order(&database.definitions)?;

    let identity_ids = database
        .identities
        .iter()
        .map(|identity| identity.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for identity in &database.identities {
        if identity.name.trim().is_empty() {
            return Err(CardDatabaseError::new(format!(
                "identity {} has an empty name",
                identity.id.as_str()
            )));
        }
    }
    for printing in &database.printings {
        if !identity_ids.contains(printing.oracle_id.as_str()) {
            return Err(CardDatabaseError::new(format!(
                "printing {} references missing identity {}",
                printing.id.as_str(),
                printing.oracle_id.as_str()
            )));
        }
    }
    for definition in &database.definitions {
        if !identity_ids.contains(definition.id.as_str()) {
            return Err(CardDatabaseError::new(format!(
                "definition {} references missing identity",
                definition.id.as_str()
            )));
        }
        if !matches!(
            definition.status,
            CardClassification::VerifiedPlayable | CardClassification::UnverifiedPlayable
        ) {
            return Err(CardDatabaseError::new(format!(
                "definition {} has a non-playable classification",
                definition.id.as_str()
            )));
        }
    }
    Ok(())
}

fn validate_identity_order(items: &[IdentityRecord]) -> Result<(), CardDatabaseError> {
    for pair in items.windows(2) {
        if pair[0].id >= pair[1].id {
            return Err(CardDatabaseError::new(format!(
                "identity ids are duplicate or unsorted at {}",
                pair[1].id.as_str()
            )));
        }
    }
    Ok(())
}

fn validate_printing_order(items: &[PrintingRecord]) -> Result<(), CardDatabaseError> {
    for pair in items.windows(2) {
        if pair[0].id >= pair[1].id {
            return Err(CardDatabaseError::new(format!(
                "printing ids are duplicate or unsorted at {}",
                pair[1].id.as_str()
            )));
        }
    }
    Ok(())
}

fn validate_definition_order(items: &[CardDefinition]) -> Result<(), CardDatabaseError> {
    for pair in items.windows(2) {
        if pair[0].id >= pair[1].id {
            return Err(CardDatabaseError::new(format!(
                "definition ids are duplicate or unsorted at {}",
                pair[1].id.as_str()
            )));
        }
    }
    Ok(())
}

fn index_database(database: CardDatabase) -> LoadedCardDatabase {
    let identity_index = database
        .identities
        .iter()
        .enumerate()
        .map(|(index, identity)| (identity.id.as_str().to_string(), index))
        .collect();
    let printing_index = database
        .printings
        .iter()
        .enumerate()
        .map(|(index, printing)| (printing.id.as_str().to_string(), index))
        .collect();
    let definition_index = database
        .definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| (definition.id.as_str().to_string(), index))
        .collect();
    let mut name_index: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, identity) in database.identities.iter().enumerate() {
        name_index
            .entry(identity.name.to_lowercase())
            .or_default()
            .push(index);
    }
    LoadedCardDatabase {
        database,
        identity_index,
        printing_index,
        definition_index,
        name_index,
    }
}

#[cfg(test)]
mod tests {
    use super::{load_card_database, CardDatabaseError};
    use forge_carddef::{
        CardClassification, CardDatabase, CardLayout, IdentityRecord, OracleId, SourceProvenance,
        CARD_DATABASE_MAGIC, CARD_DATABASE_SCHEMA_VERSION,
    };

    fn id(value: &str) -> OracleId {
        match OracleId::parse(value) {
            Some(id) => id,
            None => panic!("invalid test id"),
        }
    }

    fn database() -> CardDatabase {
        CardDatabase {
            schema_version: CARD_DATABASE_SCHEMA_VERSION,
            provenance: SourceProvenance {
                source: "test".to_string(),
                source_path: "test.json".to_string(),
                source_updated_at: "2026-07-09".to_string(),
                source_sha256: "00".repeat(32),
                generator: "test".to_string(),
            },
            identities: vec![IdentityRecord {
                id: id("oracle-a"),
                name: "Alpha".to_string(),
                layout: CardLayout::Normal,
                face_names: vec!["Alpha".to_string()],
                classification: CardClassification::UnverifiedPlayable,
            }],
            printings: Vec::new(),
            definitions: Vec::new(),
        }
    }

    fn encode(database: &CardDatabase) -> Result<Vec<u8>, CardDatabaseError> {
        let config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_little_endian();
        let payload = bincode::serde::encode_to_vec(database, config)
            .map_err(|error| CardDatabaseError::new(error.to_string()))?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&CARD_DATABASE_MAGIC);
        bytes.extend_from_slice(&CARD_DATABASE_SCHEMA_VERSION.to_le_bytes());
        bytes.extend_from_slice(&payload);
        Ok(bytes)
    }

    #[test]
    fn loads_and_indexes_valid_data() {
        let bytes = match encode(&database()) {
            Ok(bytes) => bytes,
            Err(error) => panic!("{error}"),
        };
        let loaded = match load_card_database(&bytes) {
            Ok(loaded) => loaded,
            Err(error) => panic!("{error}"),
        };
        assert_eq!(
            loaded.identity("oracle-a").map(|row| row.name.as_str()),
            Some("Alpha")
        );
        assert_eq!(loaded.identities_named("ALPHA").len(), 1);
    }

    #[test]
    fn rejects_bad_magic_and_trailing_data() {
        let mut bytes = match encode(&database()) {
            Ok(bytes) => bytes,
            Err(error) => panic!("{error}"),
        };
        bytes[0] = b'X';
        assert!(load_card_database(&bytes).is_err());

        let mut bytes = match encode(&database()) {
            Ok(bytes) => bytes,
            Err(error) => panic!("{error}"),
        };
        bytes.push(0);
        let error = match load_card_database(&bytes) {
            Ok(_database) => panic!("trailing data should fail"),
            Err(error) => error,
        };
        assert!(error.message().contains("trailing"));
    }

    #[test]
    fn rejects_header_and_payload_schema_mismatches() {
        let mut bytes = match encode(&database()) {
            Ok(bytes) => bytes,
            Err(error) => panic!("{error}"),
        };
        match bytes.get_mut(8..12) {
            Some(schema) => schema.copy_from_slice(&99_u32.to_le_bytes()),
            None => panic!("test database header is missing"),
        }
        let error = match load_card_database(&bytes) {
            Ok(_database) => panic!("unsupported header schema should fail"),
            Err(error) => error,
        };
        assert!(error.message().contains("unsupported header schema"));

        let mut database = database();
        database.schema_version = 99;
        let bytes = match encode(&database) {
            Ok(bytes) => bytes,
            Err(error) => panic!("{error}"),
        };
        let error = match load_card_database(&bytes) {
            Ok(_database) => panic!("payload schema mismatch should fail"),
            Err(error) => error,
        };
        assert!(error.message().contains("disagrees"));
    }

    #[test]
    fn rejects_unsorted_identities() {
        let mut database = database();
        database.identities.insert(
            0,
            IdentityRecord {
                id: id("oracle-z"),
                name: "Zulu".to_string(),
                layout: CardLayout::Normal,
                face_names: vec!["Zulu".to_string()],
                classification: CardClassification::UnverifiedPlayable,
            },
        );
        let bytes = match encode(&database) {
            Ok(bytes) => bytes,
            Err(error) => panic!("{error}"),
        };
        assert!(load_card_database(&bytes).is_err());
    }

    #[test]
    fn rejects_container_lengths_before_allocating() {
        let oversized_length_claim = [
            0x46, 0x4f, 0x52, 0x47, 0x45, 0x43, 0x44, 0x42, 0x01, 0x00, 0x00, 0x00, 0x00, 0x73,
            0x73, 0x73, 0x73, 0x73, 0x73, 0x73, 0x73, 0x73, 0x00, 0x00, 0x00, 0x11,
        ];
        let error = match load_card_database(&oversized_length_claim) {
            Ok(_database) => panic!("oversized allocation claim should fail"),
            Err(error) => error,
        };
        assert!(error.message().contains("decode failed"));
    }
}
