#![no_main]

use forge_cards::load_card_database;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 1_048_576 {
        return;
    }
    if let Ok(loaded) = load_card_database(data) {
        let database = loaded.database();
        for identity in &database.identities {
            assert!(loaded.identity(identity.id.as_str()).is_some());
            assert!(!identity.name.trim().is_empty());
        }
        for printing in &database.printings {
            assert!(loaded.printing(printing.id.as_str()).is_some());
            assert!(loaded.identity(printing.oracle_id.as_str()).is_some());
        }
        for definition in &database.definitions {
            assert!(loaded.definition(definition.id.as_str()).is_some());
        }
    }
});
