//! Shared resource admission diagnostic for renderer and encoder.
pub fn limit_message(label: impl std::fmt::Display, requested: impl std::fmt::Display, limit: impl std::fmt::Display, remedy: &str) -> String {
    format!("{label} exceeds device limit: requested {requested}; limit {limit}; {remedy}; see vork help limits")
}
