//! `opentribe-poly-d` — controller for the Behringer POLY D.

fn main() {
    std::process::exit(tribe_core::cli::main_for(tribe_poly_d::DEVICE));
}
