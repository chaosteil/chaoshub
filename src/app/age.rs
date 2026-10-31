//! Short ages like `5m` or `3h`.
//!
//! Under one hour the age has minute granularity, so an idle tick
//! changes a rendered age at most once per minute.

use chrono::{DateTime, Utc};

/// The age of `from` at `now`: `<1m`, `5m`, `3h` or `2d`. A time in the
/// future counts as `<1m`.
pub(crate) fn age(from: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - from).num_seconds().max(0);
    match secs {
        0..60 => "<1m".to_owned(),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;
    use crate::domain::test_time;

    #[test]
    fn age_cases() {
        let cases = [
            (-5, "<1m"),
            (0, "<1m"),
            (59, "<1m"),
            (60, "1m"),
            (3599, "59m"),
            (3600, "1h"),
            (86_399, "23h"),
            (86_400, "1d"),
            (10 * 86_400, "10d"),
        ];
        for (secs, text) in cases {
            let now = test_time() + TimeDelta::seconds(secs);
            assert_eq!(age(test_time(), now), text, "{secs} s");
        }
    }
}
