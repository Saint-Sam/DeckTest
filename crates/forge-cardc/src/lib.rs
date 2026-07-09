#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Parser, validator, canonical emitter, and database builder for Forge cards.

mod database;
mod emit;
mod error;
mod parse;
mod validate;

pub use database::{build_card_database, encode_card_database, BuildOptions, BuildReport};
pub use emit::emit_card;
pub use error::{CardcError, CardcResult};
pub use parse::{parse_card, parse_card_named};

/// Parses, canonically emits, and reparses one card source.
pub fn roundtrip_source(source: &str) -> CardcResult<String> {
    roundtrip_source_named("<memory>", source)
}

/// Parses, canonically emits, and reparses one named card source.
pub fn roundtrip_source_named(path: &str, source: &str) -> CardcResult<String> {
    let card = parse_card_named(path, source)?;
    let emitted = emit_card(&card);
    let reparsed = parse_card_named(path, &emitted)?;
    if reparsed != card {
        return Err(CardcError::new(
            path,
            1,
            1,
            "internal round-trip changed the validated card definition",
        ));
    }
    Ok(emitted)
}

#[cfg(test)]
mod tests {
    use super::{encode_card_database, parse_card, roundtrip_source};
    use forge_carddef::{CardDatabase, CardLayout, SourceProvenance, CARD_DATABASE_MAGIC};

    const LLANOWAR_ELVES: &str = r#"
card "Llanowar Elves" {
  id: "d7a9b5f2-local-llanowar"
  layout: normal
  status: verified_playable
  face "Llanowar Elves" {
    cost: "{G}"
    types: "Creature - Elf Druid"
    oracle: "{T}: Add {G}."
    power: "1"
    toughness: "1"
    keywords: []
    ability activated {
      costs: [tap_self()]
      effect: add_mana("{G}", you())
      mana_ability: true
    }
  }
}
"#;

    #[test]
    fn parses_complete_mana_ability() {
        let card = match parse_card(LLANOWAR_ELVES) {
            Ok(card) => card,
            Err(error) => panic!("{error}"),
        };
        assert_eq!(card.layout, CardLayout::Normal);
        assert_eq!(card.faces[0].power.as_deref(), Some("1"));
        assert!(card.faces[0].abilities[0].mana_ability);
    }

    #[test]
    fn preserves_split_name_comment_marker() {
        let source = LLANOWAR_ELVES.replace("Llanowar Elves", "Fire // Ice");
        let card = match parse_card(&source) {
            Ok(card) => card,
            Err(error) => panic!("{error}"),
        };
        assert_eq!(card.name, "Fire // Ice");
    }

    #[test]
    fn rejects_unknown_operation_with_position() {
        let source = LLANOWAR_ELVES.replace("add_mana", "invent_rule");
        let error = match parse_card(&source) {
            Ok(_card) => panic!("unknown operation should fail"),
            Err(error) => error,
        };
        assert!(error.line > 1);
        assert!(error.column > 1);
        assert!(error.message.contains("unknown operation"));
    }

    #[test]
    fn canonical_roundtrip_is_lossless() {
        let emitted = match roundtrip_source(LLANOWAR_ELVES) {
            Ok(emitted) => emitted,
            Err(error) => panic!("{error}"),
        };
        assert_eq!(parse_card(LLANOWAR_ELVES), parse_card(&emitted));
    }

    #[test]
    fn database_encoding_has_versioned_magic_and_is_deterministic() {
        let provenance = SourceProvenance {
            source: "unit-test".to_string(),
            source_path: "cards/test".to_string(),
            source_updated_at: "2026-07-09".to_string(),
            source_sha256: "00".repeat(32),
            generator: "test".to_string(),
        };
        let database = CardDatabase::empty(provenance);
        let first = match encode_card_database(&database) {
            Ok(bytes) => bytes,
            Err(error) => panic!("{error}"),
        };
        let second = match encode_card_database(&database) {
            Ok(bytes) => bytes,
            Err(error) => panic!("{error}"),
        };
        assert!(first.starts_with(&CARD_DATABASE_MAGIC));
        assert_eq!(first, second);
    }
}
