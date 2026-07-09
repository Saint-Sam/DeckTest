//! Fail-closed diagnostics over the generated malformed CP-DSL corpus.

use forge_cardc::parse_card_named;
use serde_json::Value;
use std::{fs, path::Path};

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
