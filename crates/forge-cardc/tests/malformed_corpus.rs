//! Fail-closed diagnostics over the generated malformed CP-DSL corpus.

use forge_cardc::parse_card_named;
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path};

#[test]
fn every_malformed_fixture_has_a_positioned_diagnostic() {
    let root = match Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2) {
        Some(root) => root,
        None => panic!("workspace root is unavailable"),
    };
    let manifest_path = root.join("cards/cp_dsl/malformed/manifest.json");
    let manifest_bytes = match fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(error) => panic!("could not read {}: {error}", manifest_path.display()),
    };
    let manifest: Value = match serde_json::from_slice(&manifest_bytes) {
        Ok(value) => value,
        Err(error) => panic!("invalid {}: {error}", manifest_path.display()),
    };
    let expected_count = manifest
        .get("case_count")
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize;
    let cases = match manifest.get("cases").and_then(Value::as_array) {
        Some(cases) => cases,
        None => panic!("malformed manifest has no cases"),
    };
    assert!(expected_count >= 50);
    assert_eq!(cases.len(), expected_count);

    let recursive_cases: Vec<&Value> = cases
        .iter()
        .filter(|case| case.get("category").and_then(Value::as_str) == Some("recursive_argument"))
        .collect();
    let recursive_minimum = manifest
        .get("recursive_argument_minimum_required")
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize;
    assert!(recursive_minimum >= 50);
    assert!(recursive_cases.len() >= recursive_minimum);
    assert_eq!(
        manifest
            .get("recursive_argument_case_count")
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize,
        recursive_cases.len()
    );

    let represented_kinds: BTreeSet<&str> = recursive_cases
        .iter()
        .filter_map(|case| case.get("argument_kind").and_then(Value::as_str))
        .collect();
    let required_kinds: BTreeSet<&str> = [
        "boolean",
        "comparable",
        "cost",
        "effect",
        "event",
        "integer",
        "number",
        "predicate",
        "predicate_or_text",
        "remembered_value",
        "scalar",
        "selector",
        "selector_or_event",
        "selector_or_number",
        "selector_or_predicate",
        "selector_or_text",
        "selector_text_or_number",
        "text",
        "timing",
        "value",
    ]
    .into_iter()
    .collect();
    assert_eq!(represented_kinds, required_kinds);

    let represented_depths: BTreeSet<u64> = recursive_cases
        .iter()
        .filter_map(|case| case.get("depth").and_then(Value::as_u64))
        .collect();
    assert!(represented_depths.is_superset(&[1, 2, 3, 4].into_iter().collect()));
    let represented_features: BTreeSet<&str> = recursive_cases
        .iter()
        .filter_map(|case| case.get("features").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    for required in [
        "bare_symbol",
        "category_correct_wrong_argument",
        "prose",
        "variadic",
    ] {
        assert!(
            represented_features.contains(required),
            "missing recursive diagnostic feature {required}"
        );
    }

    for case in cases {
        let relative = match case.get("file").and_then(Value::as_str) {
            Some(path) => path,
            None => panic!("malformed case has no file"),
        };
        let expected = case
            .get("expected_diagnostic")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let path = root.join(relative);
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => panic!("could not read {}: {error}", path.display()),
        };
        let error = match parse_card_named(relative, &source) {
            Ok(_card) => panic!("malformed fixture unexpectedly parsed: {relative}"),
            Err(error) => error,
        };
        assert!(error.line >= 1, "{relative}: missing line");
        assert!(error.column >= 1, "{relative}: missing column");
        assert_eq!(error.path, relative);
        if !expected.is_empty() {
            assert!(
                error.message.contains(expected),
                "{relative}: expected diagnostic containing {expected:?}, got {:?}",
                error.message
            );
        }
    }
}
