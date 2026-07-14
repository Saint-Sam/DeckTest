#![forbid(unsafe_code)]

//! Command-line entry point for deterministic four-player pod campaigns.

fn main() {
    if let Err(error) = forge_game_runner::run() {
        eprintln!("T3.9 pod gate failed: {error}");
        std::process::exit(1);
    }
}
