//! A value with the time it was observed, and the freshness rule.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A value and the time when it was observed at its source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Observed<T> {
    pub value: T,
    pub observed_at: DateTime<Utc>,
}

/// Replaces `current` with `incoming` unless `incoming` is older. Returns
/// true when it replaced the value. An equal time counts as not older.
pub(crate) fn apply_newer<T>(current: &mut Observed<T>, incoming: Observed<T>) -> bool {
    if incoming.observed_at < current.observed_at {
        return false;
    }
    *current = incoming;
    true
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;
    use crate::domain::test_time;

    fn at(value: &'static str, offset_secs: i64) -> Observed<&'static str> {
        Observed {
            value,
            observed_at: test_time() + TimeDelta::seconds(offset_secs),
        }
    }

    #[test]
    fn apply_newer_cases() {
        let cases = [
            ("newer replaces", 10, true, "new"),
            ("equal replaces", 0, true, "new"),
            ("older is dropped", -10, false, "old"),
        ];
        for (name, offset, applied, value) in cases {
            let mut current = at("old", 0);
            assert_eq!(
                apply_newer(&mut current, at("new", offset)),
                applied,
                "{name}"
            );
            assert_eq!(current.value, value, "{name}");
        }
    }

    #[test]
    fn apply_newer_keeps_the_time_of_the_kept_value() {
        let mut current = at("old", 0);
        apply_newer(&mut current, at("new", -5));
        assert_eq!(current.observed_at, test_time());
        apply_newer(&mut current, at("new", 5));
        assert_eq!(current.observed_at, test_time() + TimeDelta::seconds(5));
    }
}
