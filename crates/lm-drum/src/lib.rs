//! Behringer LM DRUM descriptor.
//!
//! The parameter table is generated from `verified/lm-drum.toml` by
//! `cargo run -p xtask`. It contains the 17 documented CCs plus the SysEx
//! packets and block offsets measured on hardware; see
//! `verified/lm-drum-measurements.md`.

pub mod descriptors;

pub use descriptors::DEVICE;

#[cfg(test)]
mod tests;
