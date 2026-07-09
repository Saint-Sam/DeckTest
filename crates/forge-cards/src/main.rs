#![deny(missing_docs)]
#![forbid(unsafe_code)]

//! Command-line validation surface for compiled Forge card databases.

use std::{env, process};

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
        [command, path] if command == "validate" => {
            let loaded =
                forge_cards::load_card_database_file(path).map_err(|error| error.to_string())?;
            let database = loaded.database();
            Ok(format!(
                "validated {} identities, {} printings, and {} definitions\n",
                database.identities.len(),
                database.printings.len(),
                database.definitions.len()
            ))
        }
        [command, ..] => Err(format!("unknown forge-cards command `{command}`")),
        [] => Err("usage: forge-cards validate <carddb.bin>".to_string()),
    }
}
