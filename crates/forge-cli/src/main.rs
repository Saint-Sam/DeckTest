#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Binary entrypoint for the Forge developer CLI.

use std::{env, process};

fn main() {
    match forge_cli::run_cli(env::args().skip(1).collect()) {
        Ok(output) => print!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            process::exit(2);
        }
    }
}
