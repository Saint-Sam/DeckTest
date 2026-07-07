#![no_main]

use forge_testkit::{parse_scenario_ron, run_scenario};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = core::str::from_utf8(data) {
        if input.len() <= 16_384 {
            if let Ok(scenario) = parse_scenario_ron(input) {
                let _ = run_scenario(&scenario);
            }
        }
    }
});
