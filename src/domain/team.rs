//! The ready rule and the sort order of the team tab.

use std::cmp::Ordering;

use crate::domain::pr::{CiState, PrState, PullRequest};

/// True when a pull request of a colleague is ready for review: open, not
/// a draft, and CI green. Any reviewer counts.
pub(crate) fn is_ready(pr: &PullRequest) -> bool {
    let head = &pr.head.value;
    head.state == PrState::Open && !head.is_draft && pr.ci_state.value == CiState::Success
}

/// The order of the team tab: pull requests that request the review of
/// the user first, then the newest update first, then by key.
pub(crate) fn team_order(a: &PullRequest, b: &PullRequest) -> Ordering {
    let (ha, hb) = (&a.head.value, &b.head.value);
    hb.review_requested
        .cmp(&ha.review_requested)
        .then_with(|| hb.updated_at.cmp(&ha.updated_at))
        .then_with(|| a.key.cmp(&b.key))
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;
    use crate::domain::matching::tests::pr;

    fn with(edit: impl FnOnce(&mut PullRequest)) -> PullRequest {
        let mut pr = pr(1, "", 0);
        pr.ci_state.value = CiState::Success;
        edit(&mut pr);
        pr
    }

    #[test]
    fn is_ready_cases() {
        let cases = [
            ("open, green", with(|_| {}), true),
            ("draft", with(|pr| pr.head.value.is_draft = true), false),
            (
                "ci pending",
                with(|pr| pr.ci_state.value = CiState::Pending),
                false,
            ),
            (
                "ci failed",
                with(|pr| pr.ci_state.value = CiState::Failure),
                false,
            ),
            (
                "no checks",
                with(|pr| pr.ci_state.value = CiState::None),
                false,
            ),
            (
                "closed",
                with(|pr| pr.head.value.state = PrState::Closed),
                false,
            ),
            (
                "merged",
                with(|pr| pr.head.value.state = PrState::Merged),
                false,
            ),
            (
                "review requested does not matter",
                with(|pr| pr.head.value.review_requested = true),
                true,
            ),
        ];
        for (name, pr, ready) in cases {
            assert_eq!(is_ready(&pr), ready, "{name}");
        }
    }

    #[test]
    fn team_order_puts_requests_first_then_the_newest() {
        let make = |number: u64, requested: bool, hours: i64| {
            let mut pr = pr(number, "", 0);
            pr.head.value.review_requested = requested;
            pr.head.value.updated_at += TimeDelta::hours(hours);
            pr
        };
        let mut prs = [
            make(1, false, 5),
            make(2, true, 1),
            make(3, false, 9),
            make(4, true, 3),
            make(5, false, 9),
        ];
        prs.sort_by(team_order);
        let order: Vec<u64> = prs.iter().map(|pr| pr.key.number).collect();
        assert_eq!(order, [4, 2, 3, 5, 1]);
    }
}
