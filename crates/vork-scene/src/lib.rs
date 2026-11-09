//! Unstable, internal: all Rust APIs in this crate are free to change.
//!
//! Scene API v2. Units: metres, seconds, radians, linear RGB and millimetres for lenses.
/// The scene created by `vork init` and warmed by default setup.
pub const STARTER_SCENE: &str = include_str!("starter.json");

pub mod libraries;
pub mod v2;
pub use v2::*;

pub use vork_time::Time;
