//! Behringer POLY D descriptor.
//!
//! The parameter table is generated from `verified/poly-d.toml` by
//! `cargo run -p xtask`; the tests here pin the wire format and the read-back
//! layout against the values observed on hardware.

pub mod descriptors;

pub use descriptors::DEVICE;

#[cfg(test)]
mod tests;
