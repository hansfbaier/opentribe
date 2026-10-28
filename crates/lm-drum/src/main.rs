//! `opentribe-lm-drum` — controller for the Behringer LM DRUM.

fn main() {
    std::process::exit(tribe_core::cli::main_for(tribe_lm_drum::DEVICE));
}
