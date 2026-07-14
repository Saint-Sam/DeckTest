#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Binary entrypoint for the Forge developer CLI.

use std::{env, io, process};

fn main() {
    let args = env::args().skip(1).collect();
    let result = {
        let stdin = io::stdin();
        let stdout = io::stdout();
        forge_cli::run_cli_with_io(args, &mut stdin.lock(), &mut stdout.lock())
    };
    match result {
        Ok(output) => print!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            process::exit(2);
        }
    }
}
