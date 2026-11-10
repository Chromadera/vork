//! Unstable, internal: all Rust APIs in this crate are free to change.
//!
//! Scene API v2. Units: metres, seconds, radians, linear RGB and millimetres for lenses.
/// Editor schema for this build's release; vork never fetches this URL.
pub const SCHEMA_URL: &str = concat!(
    "https://raw.githubusercontent.com/voyager-so/vork/v",
    env!("CARGO_PKG_VERSION"),
    "/packages/ts/scene.schema.json"
);

/// The scene created by `vork init` and warmed by default setup.
pub const STARTER_SCENE: &str = include_str!(concat!(env!("OUT_DIR"), "/starter.json"));

pub mod libraries;
pub mod v2;
pub use v2::*;

pub use vork_time::Time;
