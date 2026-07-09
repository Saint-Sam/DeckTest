#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Binary entrypoint for Forge legacy card translation tools.

use std::{env, process};

fn main() {
    match forge_porttools::run_cli(env::args().skip(1).collect()) {
        Ok(output) => print!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            process::exit(2);
        }
    }
}
