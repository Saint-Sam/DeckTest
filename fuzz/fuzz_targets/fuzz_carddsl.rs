#![no_main]

use forge_cardc::{emit_card, parse_card_named};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    let Ok(source) = core::str::from_utf8(data) else {
        return;
    };
    if let Ok(card) = parse_card_named("<fuzz>", source) {
        let emitted = emit_card(&card);
        let reparsed = parse_card_named("<fuzz-emitted>", &emitted)
            .unwrap_or_else(|error| panic!("emitter produced invalid source: {error}"));
        assert_eq!(reparsed, card, "card DSL round-trip changed semantics");
    }
});
