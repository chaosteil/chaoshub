//! Types and pure logic. No IO, no tokio, no ratatui.
#![allow(
    dead_code,
    reason = "the outer layers use the domain types in later phases"
)]

pub(crate) mod agent;
pub(crate) mod check;
pub(crate) mod codeview;
pub(crate) mod detail;
pub(crate) mod files;
pub(crate) mod keys;
pub(crate) mod local;
pub(crate) mod matching;
pub(crate) mod merge;
pub(crate) mod observed;
pub(crate) mod pr;
pub(crate) mod repo;
pub(crate) mod stack;
pub(crate) mod team;

/// Serializes `value` to JSON, reads it back and checks that the result is
/// equal.
#[cfg(test)]
pub(crate) fn assert_round_trip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap();
    pretty_assertions::assert_eq!(&back, value);
}

/// A fixed time for tests.
#[cfg(test)]
pub(crate) fn test_time() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap()
}
