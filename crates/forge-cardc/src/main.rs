#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Binary entrypoint for the Forge card DSL compiler.

use forge_cardc::BuildOptions;
use std::{env, fs, path::Path, process};

fn main() {
    match run(env::args().skip(1).collect()) {
        Ok(output) => print!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            process::exit(2);
        }
    }
}

fn run(args: Vec<String>) -> Result<String, String> {
    match args.as_slice() {
        [command, path] if command == "roundtrip" => roundtrip(path),
        [command, input, flag, output] if command == "build" && flag == "-o" => {
            build(input, None, output)
        }
        [command, input, catalog_flag, catalog, output_flag, output]
            if command == "build" && catalog_flag == "--catalog" && output_flag == "-o" =>
        {
            build(input, Some(catalog), output)
        }
        [command, ..] => Err(format!("unknown forge-cardc command `{command}`")),
        [] => Err(usage()),
    }
}

fn roundtrip(path: &str) -> Result<String, String> {
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    forge_cardc::roundtrip_source_named(path, &source).map_err(|error| error.to_string())
}

fn build(input: &str, catalog: Option<&String>, output: &str) -> Result<String, String> {
    let report = forge_cardc::build_card_database(BuildOptions {
        input: Path::new(input),
        catalog: catalog.map(Path::new),
        output: Path::new(output),
    })
    .map_err(|error| error.to_string())?;
    Ok(format!(
        "built {} definition(s) with {}/{} canonical round-trips, {} identities, and {} printings into {} ({} bytes)\nindex {}\n",
        report.definition_count,
        report.roundtrip_count,
        report.definition_count,
        report.identity_count,
        report.printing_count,
        report.output.display(),
        report.byte_count,
        report.index.display()
    ))
}

fn usage() -> String {
    "usage: forge-cardc roundtrip <card.frs> | forge-cardc build <dir-or-file> [--catalog <catalog.json>] -o <carddb.bin>".to_string()
}
