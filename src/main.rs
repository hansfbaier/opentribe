//! `opentribe` — the default opentribe app: the LM DRUM controller.
//!
//! The workspace also ships one binary per instrument
//! (`opentribe-lm-drum`, `opentribe-poly-d`); this package is the portable
//! entry point that starts the LM DRUM editor, so `cargo run` in the
//! workspace root opens the app the person asked for.

fn main() {
    std::process::exit(tribe_core::cli::main_for(tribe_lm_drum::DEVICE));
}
