#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Command-line runner for Forge scenario and oracle tests.

use forge_testkit::{failed_report, parse_scenario_ron, reports_to_junit_xml, run_scenario_file};
use std::{
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
        "--help" | "-h" | "help" => {
            print_usage();
            Ok(())
        }
        other => Err(format!("unknown forge-testkit command `{other}`")),
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

    let mut files = scenario_files(Path::new("tests/oracle"))?;
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
        "forge-testkit commands:\n  lint [path]\n  oracle [--all|--changed] [--filter TEXT] [--junit PATH|--no-junit]"
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
    fn oracle_changed_succeeds_with_junit() {
        assert!(run(vec!["oracle".to_owned(), "--changed".to_owned()]).is_ok());
    }

    #[test]
    fn unknown_command_is_an_error() {
        assert!(run(vec!["nope".to_owned()]).is_err());
    }

    #[test]
    fn malformed_oracle_option_is_an_error() {
        assert!(run(vec!["oracle".to_owned(), "--filter".to_owned()]).is_err());
        assert!(run(vec!["oracle".to_owned(), "--junit".to_owned()]).is_err());
        assert!(run(vec!["oracle".to_owned(), "--surprise".to_owned()]).is_err());
    }

    #[test]
    fn absent_scenario_path_is_empty() {
        let files = scenario_files(Path::new("tests/oracle/absent"))
            .unwrap_or_else(|error| panic!("unexpected scenario file error: {error}"));
        assert!(files.is_empty());
    }
}
