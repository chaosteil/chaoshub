//! The pull requests of `chaoshub list`: the open pull requests of the
//! cache that the tabs of the list show, in the order of the tabs.
//!
//! The list uses the rules of the normal view of each tab: the age limit
//! of `[filters]` for every tab, and the ready rule for the Review tab
//! and the Team tab. With `all`, the list skips both rules, as the
//! `show_hidden` key (`T`) does. The text filter and the author filter
//! of the tabs do not apply.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::app::term::{pr_overview, pr_status};
use crate::bus::Scope;
use crate::config::Config;
use crate::domain::pr::{PrState, PullRequest};
use crate::domain::team::{is_ready, team_order};
use crate::store::Snapshot;

/// The version of the JSON of `chaoshub list --json`. A change that
/// removes a field, renames a field or changes the meaning of a field
/// increments it. A new field does not.
pub(crate) const VERSION: u32 = 1;

/// The output of `chaoshub list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Listing {
    /// `VERSION`.
    pub version: u32,
    /// The newest sync time of the cached pull requests: the age of the
    /// cache. `None` when the cache holds none.
    pub synced_at: Option<DateTime<Utc>>,
    pub prs: Vec<ListedPr>,
}

/// One pull request of `chaoshub list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ListedPr {
    /// The repo in the `owner/name` form.
    pub repo: String,
    pub number: u64,
    pub title: String,
    /// The login of the author.
    pub author: String,
    pub url: String,
    /// The tab of the pull request: `mine`, `review` or `team`.
    pub tab: &'static str,
    /// `open`, `closed` or `merged`. The list holds only open pull
    /// requests now.
    pub state: &'static str,
    pub draft: bool,
    /// True when GitHub requests the review of the user or of one of
    /// the user's teams.
    pub review_requested: bool,
    /// The verdict, the CI and the reviews, for example
    /// `ready · CI passed · approved`.
    pub status: String,
    /// The PR overview, for example `#7 Fix the merge · ready · CI
    /// passed`. The detail pane inside Rex uses it as the block label.
    pub label: String,
    /// The local checkout of the repo from its `[[repo]]` entry. `None`
    /// when the entry has no path, or when the configuration has no
    /// entry for the repo.
    pub path: Option<String>,
}

/// The pull requests of the tabs in `snapshot` at `now`, with the rules
/// of `config`. With `all`, every open pull request of the cache.
pub(crate) fn listing(
    snapshot: &Snapshot,
    config: &Config,
    now: DateTime<Utc>,
    all: bool,
) -> Listing {
    let open = |scope: Scope| {
        snapshot
            .prs
            .iter()
            .filter(move |(s, pr)| *s == scope && pr.head.value.state == PrState::Open)
            .map(|(_, pr)| pr)
            .filter(|pr| all || config.filters.matches(pr, now))
    };
    let mut mine: Vec<&PullRequest> = open(Scope::Mine).collect();
    mine.sort_by(|a, b| {
        b.head
            .value
            .updated_at
            .cmp(&a.head.value.updated_at)
            .then_with(|| a.key.cmp(&b.key))
    });
    let mut team: Vec<&PullRequest> = open(Scope::Team).filter(|pr| all || is_ready(pr)).collect();
    team.sort_by(|a, b| team_order(a, b));
    let (review, team): (Vec<&PullRequest>, Vec<&PullRequest>) = team
        .into_iter()
        .partition(|pr| pr.head.value.review_requested);

    let repos = config.repos();
    let item = |tab: &'static str, pr: &PullRequest| {
        let head = &pr.head.value;
        let path = repos
            .iter()
            .find(|repo| repo.key == pr.key.repo)
            .and_then(|repo| repo.path.as_ref())
            .map(|path| path.to_string_lossy().into_owned());
        ListedPr {
            repo: format!("{}/{}", pr.key.repo.owner, pr.key.repo.name),
            number: pr.key.number,
            title: head.title.clone(),
            author: head.author.clone(),
            url: head.url.clone(),
            tab,
            state: state_text(head.state),
            draft: head.is_draft,
            review_requested: head.review_requested,
            status: pr_status(pr),
            label: pr_overview(pr),
            path,
        }
    };
    let prs = [("mine", mine), ("review", review), ("team", team)]
        .into_iter()
        .flat_map(|(tab, prs)| prs.into_iter().map(move |pr| (tab, pr)))
        .map(|(tab, pr)| item(tab, pr))
        .collect();
    Listing {
        version: VERSION,
        synced_at: snapshot.synced_at,
        prs,
    }
}

/// The JSON text of `listing`, on one line.
pub(crate) fn json(listing: &Listing) -> serde_json::Result<String> {
    serde_json::to_string(listing)
}

/// The text of `listing` for a person: one line for each pull request,
/// with the tab, `owner/name#N`, the author, the status and the title,
/// split by tabs.
pub(crate) fn text(listing: &Listing) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for pr in &listing.prs {
        // A write to a String does not fail.
        let _ = writeln!(
            out,
            "{}\t{}#{}\t{}\t{}\t{}",
            pr.tab, pr.repo, pr.number, pr.author, pr.status, pr.title
        );
    }
    out
}

fn state_text(state: PrState) -> &'static str {
    match state {
        PrState::Open => "open",
        PrState::Closed => "closed",
        PrState::Merged => "merged",
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;
    use crate::domain::matching::tests::pr;
    use crate::domain::pr::CiState;
    use crate::domain::test_time;

    /// A pull request #`number` of `o/r` by `author`, green, updated
    /// `minutes` after the test time.
    fn sample(number: u64, author: &str, minutes: i64) -> PullRequest {
        let mut pr = pr(number, "", 0);
        pr.head.value.author = author.into();
        pr.head.value.url = format!("https://github.com/o/r/pull/{number}");
        pr.head.value.updated_at = test_time() + TimeDelta::minutes(minutes);
        pr.ci_state.value = CiState::Success;
        pr
    }

    fn numbers(listing: &Listing) -> Vec<(&'static str, u64)> {
        listing.prs.iter().map(|pr| (pr.tab, pr.number)).collect()
    }

    fn config(text: &str) -> Config {
        Config::parse(text, None).unwrap()
    }

    /// Mine #1 (old) and #2 (new), a merged #3, a review request #4, a
    /// ready team #5, a draft team #6 and an old team #7.
    fn snapshot() -> Snapshot {
        let mut merged = sample(3, "me", 0);
        merged.head.value.state = PrState::Merged;
        let mut review = sample(4, "ana", 0);
        review.head.value.review_requested = true;
        let mut draft = sample(6, "bo", 0);
        draft.head.value.is_draft = true;
        let mut old = sample(7, "bo", 0);
        old.head.value.created_at = Some(test_time() - TimeDelta::days(30));
        Snapshot {
            prs: vec![
                (Scope::Mine, sample(1, "me", 0)),
                (Scope::Mine, sample(2, "me", 5)),
                (Scope::Mine, merged),
                (Scope::Team, review),
                (Scope::Team, sample(5, "bo", 0)),
                (Scope::Team, draft),
                (Scope::Team, old),
            ],
            synced_at: Some(test_time()),
            ..Snapshot::default()
        }
    }

    #[test]
    fn the_tabs_come_in_order_with_their_rules() {
        let config = config("[filters]\nmax_age = \"2 weeks\"");
        let listing = listing(&snapshot(), &config, test_time(), false);
        assert_eq!(
            numbers(&listing),
            [("mine", 2), ("mine", 1), ("review", 4), ("team", 5)]
        );
    }

    #[test]
    fn all_skips_the_age_limit_and_the_ready_rule() {
        let config = config("[filters]\nmax_age = \"2 weeks\"");
        let listing = listing(&snapshot(), &config, test_time(), true);
        assert_eq!(
            numbers(&listing),
            [
                ("mine", 2),
                ("mine", 1),
                ("review", 4),
                ("team", 5),
                ("team", 6),
                ("team", 7)
            ]
        );
    }

    #[test]
    fn an_empty_cache_gives_an_empty_list() {
        let listing = listing(&Snapshot::default(), &Config::default(), test_time(), false);
        assert_eq!(
            json(&listing).unwrap(),
            r#"{"version":1,"synced_at":null,"prs":[]}"#
        );
        assert_eq!(text(&listing), "");
    }

    #[test]
    fn json_has_the_documented_fields() {
        let mut review = sample(4, "ana", 0);
        review.head.value.title = "Fix the merge".into();
        review.head.value.review_requested = true;
        review.head.value.mergeable = crate::domain::pr::Mergeable::Mergeable;
        review.head.value.merge_state = crate::domain::pr::MergeStateStatus::Clean;
        review.head.value.review_decision = Some(crate::domain::pr::ReviewDecision::Approved);
        let snapshot = Snapshot {
            prs: vec![(Scope::Team, review)],
            synced_at: Some(test_time()),
            ..Snapshot::default()
        };
        let config = config("[[repo]]\nname = \"o/r\"\npath = \"/code/r\"\n");
        let listing = listing(&snapshot, &config, test_time(), false);
        let value: serde_json::Value = serde_json::from_str(&json(&listing).unwrap()).unwrap();
        assert_eq!(
            value,
            json!({
                "version": 1,
                "synced_at": "2026-09-21T14:13:20Z",
                "prs": [{
                    "repo": "o/r",
                    "number": 4,
                    "title": "Fix the merge",
                    "author": "ana",
                    "url": "https://github.com/o/r/pull/4",
                    "tab": "review",
                    "state": "open",
                    "draft": false,
                    "review_requested": true,
                    "status": "ready · CI passed · approved",
                    "label": "#4 Fix the merge · ready · CI passed · approved",
                    "path": "/code/r",
                }],
            })
        );
    }

    #[test]
    fn a_repo_without_a_path_has_a_null_path() {
        let config = config("[[repo]]\nname = \"o/r\"\n");
        let listing = listing(&snapshot(), &config, test_time(), false);
        assert_eq!(listing.prs.len(), 5);
        assert!(listing.prs.iter().all(|pr| pr.path.is_none()));
    }

    #[test]
    fn text_has_one_line_for_each_pr() {
        let listing = listing(&snapshot(), &Config::default(), test_time(), false);
        let text = text(&listing);
        assert_eq!(text.lines().count(), listing.prs.len());
        assert_eq!(
            text.lines().next().unwrap(),
            "mine\to/r#2\tme\tchecking · CI passed\tChange 2"
        );
    }
}
