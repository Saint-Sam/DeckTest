#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Command-line runner for Forge scenario and oracle tests.

use forge_testkit::runtime_smoke::{run_translated_card_runtime_smoke, RuntimeSmokeResult};
use forge_testkit::{failed_report, parse_scenario_ron, reports_to_junit_xml, run_scenario_file};
use serde_json::json;
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process,
};

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("{error}");
        process::exit(2);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        print_usage();
        return Ok(());
    };
    match command {
        "lint" => lint_command(&args[1..]),
        "oracle" => oracle_command(&args[1..]),
        "runtime-smoke" => runtime_smoke_command(&args[1..]),
        "--help" | "-h" | "help" => {
            print_usage();
            Ok(())
        }
        other => Err(format!("unknown forge-testkit command `{other}`")),
    }
}

fn runtime_smoke_command(args: &[String]) -> Result<(), String> {
    let raw_path = args
        .first()
        .ok_or_else(|| "runtime-smoke requires one .frs file or directory".to_owned())?;
    let mut report_path = None;
    let mut quiet = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--report" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "runtime-smoke --report requires an output path".to_owned())?;
                report_path = Some(PathBuf::from(value));
                index += 2;
            }
            "--quiet" => {
                quiet = true;
                index += 1;
            }
            option => return Err(format!("unknown runtime-smoke option `{option}`")),
        }
    }
    let files = translated_card_files(Path::new(raw_path))?;
    if files.is_empty() {
        return Err(format!("no .frs files found under {raw_path}"));
    }

    let mut passed = 0_usize;
    let mut unsupported = 0_usize;
    let mut failed = 0_usize;
    let mut capability_counts = BTreeMap::<String, usize>::new();
    let mut unsupported_counts = BTreeMap::<String, usize>::new();
    let mut failure_counts = BTreeMap::<String, usize>::new();
    let mut entries = Vec::with_capacity(files.len());
    for file in files {
        let source = match fs::read_to_string(&file) {
            Ok(source) => source,
            Err(error) => {
                failed = failed.saturating_add(1);
                println!(
                    "FAIL runtime-smoke {} code=read_error detail={error}",
                    file.display()
                );
                *failure_counts.entry("read_error".to_owned()).or_default() += 1;
                entries.push(json!({
                    "path": file.display().to_string(),
                    "disposition": "failed",
                    "code": "read_error",
                    "detail": error.to_string(),
                }));
                continue;
            }
        };
        let definition = match forge_cardc::parse_card_named(&file.display().to_string(), &source) {
            Ok(definition) => definition,
            Err(error) => {
                failed = failed.saturating_add(1);
                println!(
                    "FAIL runtime-smoke {} code=compiler_invalid detail={error}",
                    file.display()
                );
                *failure_counts
                    .entry("compiler_invalid".to_owned())
                    .or_default() += 1;
                entries.push(json!({
                    "path": file.display().to_string(),
                    "disposition": "failed",
                    "code": "compiler_invalid",
                    "detail": error.to_string(),
                }));
                continue;
            }
        };
        let report = run_translated_card_runtime_smoke(&definition);
        match report.result() {
            RuntimeSmokeResult::Passed(pass) => {
                passed = passed.saturating_add(1);
                let capability_names = pass
                    .capabilities()
                    .iter()
                    .map(|capability| capability.as_str())
                    .collect::<Vec<_>>();
                for capability in &capability_names {
                    *capability_counts
                        .entry((*capability).to_owned())
                        .or_default() += 1;
                }
                if !quiet {
                    println!(
                        "PASS runtime-smoke {} card={} capabilities={} effect_actions={} production_actions={} destination={} final_hash={}",
                        file.display(),
                        report.oracle_id(),
                        capability_names.join(","),
                        pass.effect_actions(),
                        pass.production_actions(),
                        pass.destination(),
                        pass.final_hash()
                    );
                }
                entries.push(json!({
                    "path": file.display().to_string(),
                    "oracle_id": report.oracle_id(),
                    "card_name": report.card_name(),
                    "disposition": "passed",
                    "capabilities": capability_names,
                    "effect_actions": pass.effect_actions(),
                    "production_actions": pass.production_actions(),
                    "final_life_totals": pass.final_life_totals(),
                    "destination": pass.destination(),
                    "final_hash": pass.final_hash(),
                }));
            }
            RuntimeSmokeResult::UnsupportedSetup(result) => {
                unsupported = unsupported.saturating_add(1);
                let code = result.code().as_str();
                *unsupported_counts.entry(code.to_owned()).or_default() += 1;
                if !quiet {
                    println!(
                        "UNSUPPORTED runtime-smoke {} card={} code={} detail={}",
                        file.display(),
                        report.oracle_id(),
                        code,
                        result.detail()
                    );
                }
                entries.push(json!({
                    "path": file.display().to_string(),
                    "oracle_id": report.oracle_id(),
                    "card_name": report.card_name(),
                    "disposition": "unsupported_setup",
                    "code": code,
                    "detail": result.detail(),
                }));
            }
            RuntimeSmokeResult::Failed(result) => {
                failed = failed.saturating_add(1);
                let code = result.code().as_str();
                *failure_counts.entry(code.to_owned()).or_default() += 1;
                if !quiet {
                    println!(
                        "FAIL runtime-smoke {} card={} code={} phase={} detail={}",
                        file.display(),
                        report.oracle_id(),
                        code,
                        result.phase(),
                        result.detail()
                    );
                }
                entries.push(json!({
                    "path": file.display().to_string(),
                    "oracle_id": report.oracle_id(),
                    "card_name": report.card_name(),
                    "disposition": "failed",
                    "code": code,
                    "phase": result.phase(),
                    "detail": result.detail(),
                }));
            }
        }
    }
    if let Some(path) = report_path {
        let value = json!({
            "schema_version": 1,
            "kind": "t3_5_runtime_smoke_audit",
            "source_root": raw_path,
            "total_definitions": passed + unsupported + failed,
            "passed": passed,
            "unsupported_setup": unsupported,
            "failed": failed,
            "capability_counts": capability_counts,
            "unsupported_reason_counts": unsupported_counts,
            "failure_reason_counts": failure_counts,
            "entries": entries,
        });
        let rendered = serde_json::to_string_pretty(&value)
            .map_err(|error| format!("failed to serialize runtime report: {error}"))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create runtime report directory: {error}"))?;
        }
        fs::write(&path, format!("{rendered}\n"))
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    println!("runtime smoke: {passed} passed, {unsupported} unsupported, {failed} failed");
    if unsupported == 0 && failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "runtime smoke did not pass: {unsupported} unsupported setup(s), {failed} failure(s)"
        ))
    }
}

fn lint_command(args: &[String]) -> Result<(), String> {
    let path = args.first().map_or("tests/oracle", String::as_str);
    let files = scenario_files(Path::new(path))?;
    if files.is_empty() {
        println!("SKIP: no oracle scenarios are present yet");
        return Ok(());
    }
    let mut failed = false;
    for file in &files {
        let input = fs::read_to_string(file)
            .map_err(|error| format!("failed to read {}: {error}", file.display()))?;
        match parse_scenario_ron(&input) {
            Ok(_) => println!("PASS lint {}", file.display()),
            Err(error) => {
                failed = true;
                println!("FAIL lint {}: {error}", file.display());
            }
        }
    }
    if failed {
        Err("one or more scenarios failed lint".to_owned())
    } else {
        Ok(())
    }
}

fn oracle_command(args: &[String]) -> Result<(), String> {
    let mut mode = OracleMode::All;
    let mut filter = None;
    let mut path = PathBuf::from("tests/oracle");
    let mut junit = Some(PathBuf::from("target/forge-testkit/oracle-junit.xml"));
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--all" => mode = OracleMode::All,
            "--changed" => mode = OracleMode::Changed,
            "--filter" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("--filter requires a value".to_owned());
                };
                filter = Some(value.clone());
                index += 1;
            }
            "--path" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("--path requires a value".to_owned());
                };
                path = PathBuf::from(value);
                index += 1;
            }
            "--junit" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("--junit requires a path".to_owned());
                };
                junit = Some(PathBuf::from(value));
                index += 1;
            }
            "--no-junit" => junit = None,
            other => return Err(format!("unknown oracle option `{other}`")),
        }
        index += 1;
    }

    let mut files = scenario_files(&path)?;
    if matches!(mode, OracleMode::Changed) {
        println!("INFO: --changed currently falls back to all oracle scenarios");
    }
    if let Some(filter) = filter {
        files.retain(|path| path.to_string_lossy().contains(&filter));
    }
    if files.is_empty() {
        println!("SKIP: no oracle scenarios are present yet");
        return Ok(());
    }

    let mut reports = Vec::new();
    for file in files {
        match run_scenario_file(&file) {
            Ok(report) => {
                print_report(&file, &report);
                reports.push(report);
            }
            Err(error) => {
                let report = failed_report(file.display().to_string(), "parse", error.to_string());
                print_report(&file, &report);
                reports.push(report);
            }
        }
    }

    if let Some(path) = junit {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        fs::write(&path, reports_to_junit_xml(&reports))
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        println!("WROTE {}", path.display());
    }

    let failed = reports.iter().filter(|report| !report.passed()).count();
    println!(
        "oracle scenarios: {} passed, {} failed",
        reports.len() - failed,
        failed
    );
    if failed == 0 {
        Ok(())
    } else {
        Err("one or more oracle scenarios failed".to_owned())
    }
}

fn print_report(path: &Path, report: &forge_testkit::ScenarioReport) {
    if report.passed() {
        println!("PASS {}", path.display());
    } else {
        println!("FAIL {}", path.display());
        for failure in report.failures() {
            println!("  {}: {}", failure.phase(), failure.message());
        }
    }
}

fn scenario_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_scenario_files(path, &mut files)?;
    files.sort();
    Ok(files)
}

fn translated_card_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_translated_card_files(path, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_translated_card_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        if path.extension().is_some_and(|extension| extension == "frs") {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }
    for entry in fs::read_dir(path)
        .map_err(|error| format!("failed to read directory {}: {error}", path.display()))?
    {
        let entry = entry
            .map_err(|error| format!("failed to read entry in {}: {error}", path.display()))?;
        collect_translated_card_files(&entry.path(), files)?;
    }
    Ok(())
}

fn collect_scenario_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        if path.extension().is_some_and(|extension| extension == "ron") {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }
    for entry in fs::read_dir(path)
        .map_err(|error| format!("failed to read directory {}: {error}", path.display()))?
    {
        let entry = entry
            .map_err(|error| format!("failed to read entry in {}: {error}", path.display()))?;
        collect_scenario_files(&entry.path(), files)?;
    }
    Ok(())
}

fn print_usage() {
    println!(
        "forge-testkit commands:\n  lint [path]\n  oracle [--all|--changed] [--path PATH] [--filter TEXT] [--junit PATH|--no-junit]\n  runtime-smoke <FILE-OR-DIRECTORY> [--report PATH] [--quiet]"
    );
}

enum OracleMode {
    All,
    Changed,
}

#[cfg(test)]
mod tests {
    use super::{run, scenario_files};
    use std::path::Path;

    #[test]
    fn help_command_succeeds() {
        assert!(run(vec!["help".to_owned()]).is_ok());
    }

    #[test]
    fn lint_oracle_directory_succeeds() {
        assert!(run(vec!["lint".to_owned(), "tests/oracle".to_owned()]).is_ok());
    }

    #[test]
    fn oracle_all_succeeds_without_junit() {
        assert!(run(vec![
            "oracle".to_owned(),
            "--all".to_owned(),
            "--no-junit".to_owned(),
        ])
        .is_ok());
    }

    #[test]
    fn oracle_filter_succeeds() {
        assert!(run(vec![
            "oracle".to_owned(),
            "--filter".to_owned(),
            "t1_9".to_owned(),
            "--no-junit".to_owned(),
        ])
        .is_ok());
    }

    #[test]
    fn oracle_path_succeeds() {
        assert!(run(vec![
            "oracle".to_owned(),
            "--path".to_owned(),
            "tests/oracle".to_owned(),
            "--filter".to_owned(),
            "t1_9".to_owned(),
            "--no-junit".to_owned(),
        ])
        .is_ok());
    }

    #[test]
    fn oracle_changed_succeeds_with_junit() {
        assert!(run(vec!["oracle".to_owned(), "--changed".to_owned()]).is_ok());
    }

    #[test]
    fn unknown_command_is_an_error() {
        assert!(run(vec!["nope".to_owned()]).is_err());
    }

    #[test]
    fn runtime_smoke_supported_fixture_succeeds() {
        assert!(run(vec![
            "runtime-smoke".to_owned(),
            runtime_smoke_fixture("supported_life_spell.frs"),
        ])
        .is_ok());
    }

    #[test]
    fn runtime_smoke_unsupported_fixture_does_not_pass() {
        assert!(run(vec![
            "runtime-smoke".to_owned(),
            runtime_smoke_fixture("unsupported_mill_spell.frs"),
        ])
        .is_err());
    }

    #[test]
    fn runtime_smoke_requires_one_path() {
        assert!(run(vec!["runtime-smoke".to_owned()]).is_err());
    }

    #[test]
    fn malformed_oracle_option_is_an_error() {
        assert!(run(vec!["oracle".to_owned(), "--filter".to_owned()]).is_err());
        assert!(run(vec!["oracle".to_owned(), "--path".to_owned()]).is_err());
        assert!(run(vec!["oracle".to_owned(), "--junit".to_owned()]).is_err());
        assert!(run(vec!["oracle".to_owned(), "--surprise".to_owned()]).is_err());
    }

    #[test]
    fn absent_scenario_path_is_empty() {
        let files = scenario_files(Path::new("tests/oracle/absent"))
            .unwrap_or_else(|error| panic!("unexpected scenario file error: {error}"));
        assert!(files.is_empty());
    }

    fn runtime_smoke_fixture(name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/runtime_smoke")
            .join(name)
            .to_string_lossy()
            .into_owned()
    }
}
