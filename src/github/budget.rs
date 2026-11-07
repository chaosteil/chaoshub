//! The rate budget and the poll interval.
//!
//! GitHub gives a budget of GraphQL points per hour. A worker polls every
//! `base` interval. When the remaining points do not last until the reset
//! at the cost of the last request, the interval doubles until they do.

use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::bus::RateInfo;

/// The most doublings of the base interval.
const MAX_DOUBLINGS: u32 = 16;

/// The last rate limit info that a worker saw.
#[derive(Debug, Clone, Default)]
pub(crate) struct RateBudget {
    last: Option<RateInfo>,
}

impl RateBudget {
    /// Records the rate limit info of a response.
    pub(crate) fn observe(&mut self, rate: RateInfo) {
        self.last = Some(rate);
    }

    /// The last rate limit info, if any.
    pub(crate) fn last(&self) -> Option<&RateInfo> {
        self.last.as_ref()
    }

    /// The interval until the next poll. `base` until a response came in.
    pub(crate) fn interval(&self, base: Duration) -> Duration {
        self.last
            .as_ref()
            .map_or(base, |rate| next_interval(base, rate))
    }

    /// True when the budget makes the interval longer than `base`. The
    /// status bar then shows "slowing: rate budget".
    pub(crate) fn is_slowing(&self, base: Duration) -> bool {
        self.interval(base) > base
    }
}

/// The interval until the next poll for a `base` interval and the last
/// rate limit info.
pub(crate) fn next_interval(base: Duration, rate: &RateInfo) -> Duration {
    next_interval_at(base, rate, Utc::now())
}

/// [`next_interval`] at the time `now`. With no points left, it waits
/// until the reset. Else it doubles `base` until the requests until the
/// reset, at the cost of the last request, fit in the remaining points.
fn next_interval_at(base: Duration, rate: &RateInfo, now: DateTime<Utc>) -> Duration {
    let until_reset = (rate.reset_at - now).to_std().unwrap_or_default();
    if until_reset.is_zero() || base.is_zero() {
        return base;
    }
    if rate.remaining == 0 {
        return until_reset.max(base);
    }
    let cost = u128::from(rate.last_cost.max(1));
    let mut interval = base;
    for _ in 0..MAX_DOUBLINGS {
        let requests = until_reset.as_millis() / interval.as_millis().max(1);
        if requests * cost <= u128::from(rate.remaining) {
            break;
        }
        interval *= 2;
    }
    interval
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;
    use crate::domain::test_time;

    fn rate(remaining: u32, reset_in_secs: i64, last_cost: u32) -> RateInfo {
        RateInfo {
            remaining,
            reset_at: test_time() + TimeDelta::seconds(reset_in_secs),
            last_cost,
        }
    }

    #[test]
    fn next_interval_cases() {
        let base = Duration::from_secs(60);
        let cases = [
            ("enough points keep the base", rate(4000, 3600, 1), 60),
            ("exactly enough keeps the base", rate(60, 3600, 1), 60),
            ("half the points double", rate(30, 3600, 1), 120),
            (
                "a quarter of the points double twice",
                rate(15, 3600, 1),
                240,
            ),
            ("the cost counts", rate(120, 3600, 4), 120),
            ("a cost of 0 counts as 1", rate(60, 3600, 0), 60),
            ("no points wait for the reset", rate(0, 900, 1), 900),
            (
                "no points and a near reset keep the base",
                rate(0, 10, 1),
                60,
            ),
            ("a past reset keeps the base", rate(0, -10, 1), 60),
            ("one point waits past the reset", rate(1, 600, 50), 960),
        ];
        for (name, rate, secs) in cases {
            assert_eq!(
                next_interval_at(base, &rate, test_time()),
                Duration::from_secs(secs),
                "{name}"
            );
        }
    }

    #[test]
    fn budget_is_slowing_only_after_a_tight_response() {
        let base = Duration::from_secs(60);
        let mut budget = RateBudget::default();
        assert_eq!(budget.interval(base), base);
        assert!(!budget.is_slowing(base));
        budget.observe(RateInfo {
            remaining: 1,
            reset_at: Utc::now() + TimeDelta::seconds(3600),
            last_cost: 1,
        });
        assert!(budget.is_slowing(base));
        assert!(budget.interval(base) > base);
        assert_eq!(budget.last().map(|rate| rate.remaining), Some(1));
    }
}
