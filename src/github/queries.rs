//! Typed GraphQL queries, one per file in `queries/`.
//!
//! Each struct derives `GraphQLQuery` from one `.graphql` file and the
//! vendored schema. The derive checks every field against the schema at
//! compile time. Every query selects `rateLimit { cost remaining resetAt }`.

use chrono::Utc;
use graphql_client::GraphQLQuery;

use crate::bus::RateInfo;
use crate::domain;
use crate::domain::keys::PrKey;
use crate::domain::observed::Observed;
use crate::domain::pr::PullRequest;
use crate::domain::stack::Stack;
use crate::github::parse::{self, CiPollResult, Me};
use crate::github::{GithubClient, GithubError};

/// The GraphQL `DateTime` scalar: an ISO 8601 time.
type DateTime = chrono::DateTime<chrono::Utc>;
/// The GraphQL `URI` scalar.
#[allow(
    clippy::upper_case_acronyms,
    reason = "the derive needs the GraphQL scalar name"
)]
type URI = String;
/// The GraphQL `GitObjectID` scalar: a commit SHA.
type GitObjectID = String;

/// `viewer { login }`: the login of the token owner.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github/schema.docs.graphql",
    query_path = "src/github/queries/viewer.graphql",
    response_derives = "Debug"
)]
pub(crate) struct Viewer;

/// One page of a pull request search.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github/schema.docs.graphql",
    query_path = "src/github/queries/list_prs.graphql",
    response_derives = "Debug"
)]
pub(crate) struct ListPrs;

/// Pull requests by GraphQL node id.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github/schema.docs.graphql",
    query_path = "src/github/queries/pr_by_ids.graphql",
    response_derives = "Debug"
)]
pub(crate) struct PrByIds;

/// The entries of a stack.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github/schema.docs.graphql",
    query_path = "src/github/queries/stack_entries.graphql",
    response_derives = "Debug"
)]
pub(crate) struct StackEntries;

/// The CI state of visible pull requests and the checks of the selected
/// one, in one request.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github/schema.docs.graphql",
    query_path = "src/github/queries/ci_poll.graphql",
    response_derives = "Debug"
)]
pub(crate) struct CiPoll;

/// The conversation of one pull request.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github/schema.docs.graphql",
    query_path = "src/github/queries/pr_detail.graphql",
    response_derives = "Debug"
)]
pub(crate) struct PrDetail;

/// `addPullRequestReview`: approve or request changes.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github/schema.docs.graphql",
    query_path = "src/github/queries/add_review.graphql",
    response_derives = "Debug"
)]
pub(crate) struct AddReview;

/// `markPullRequestReadyForReview`.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github/schema.docs.graphql",
    query_path = "src/github/queries/mark_ready.graphql",
    response_derives = "Debug"
)]
pub(crate) struct MarkReady;

/// `convertPullRequestToDraft`.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github/schema.docs.graphql",
    query_path = "src/github/queries/convert_to_draft.graphql",
    response_derives = "Debug"
)]
pub(crate) struct ConvertToDraft;

/// `closePullRequest`.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github/schema.docs.graphql",
    query_path = "src/github/queries/close_pr.graphql",
    response_derives = "Debug"
)]
pub(crate) struct ClosePr;

/// The event of a review that chaoshub submits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewEvent {
    Approve,
    RequestChanges,
}

/// The error of a mutation whose payload is null.
fn no_payload(mutation: &str) -> GithubError {
    GithubError::Graphql(vec![format!("{mutation} returned no result")])
}

impl GithubClient {
    /// The login of the token owner.
    pub(crate) async fn viewer(&self) -> Result<(String, RateInfo), GithubError> {
        let (data, rate) = self.run::<Viewer>(viewer::Variables).await?;
        Ok((data.viewer.login, rate))
    }

    /// One page of the pull request search `search`, with `first` results
    /// after the cursor `after`.
    pub(crate) async fn list_prs(
        &self,
        search: &str,
        first: u32,
        after: Option<&str>,
        me: &Me<'_>,
    ) -> Result<PrPage, GithubError> {
        let started_at = Utc::now();
        let vars = list_prs::Variables {
            search: search.to_owned(),
            first: i64::from(first),
            after: after.map(str::to_owned),
        };
        let (data, rate) = self.run::<ListPrs>(vars).await?;
        let (prs, next) = parse::list_prs::page(data, me, started_at)?;
        Ok(PrPage {
            prs,
            next,
            rate,
            started_at,
        })
    }

    /// Up to `max_pages` pages of the search `search`, `first` results per
    /// page. Each pull request carries the start time of its own page.
    pub(crate) async fn list_prs_pages(
        &self,
        search: &str,
        first: u32,
        max_pages: usize,
        me: &Me<'_>,
    ) -> Result<PrPages, GithubError> {
        let mut page = self.list_prs(search, first, None, me).await?;
        let started_at = page.started_at;
        let mut prs = std::mem::take(&mut page.prs);
        let mut pages = 1;
        while let Some(after) = page.next.take() {
            if pages >= max_pages {
                page.next = Some(after);
                break;
            }
            page = self.list_prs(search, first, Some(&after), me).await?;
            prs.append(&mut page.prs);
            pages += 1;
        }
        Ok(PrPages {
            prs,
            complete: page.next.is_none(),
            rate: page.rate,
            started_at,
        })
    }

    /// The pull requests with the GraphQL node ids `ids`. Ids that do not
    /// resolve to a pull request are skipped.
    pub(crate) async fn pr_by_ids(
        &self,
        ids: &[String],
        me: &Me<'_>,
    ) -> Result<(Vec<PullRequest>, RateInfo), GithubError> {
        let started_at = Utc::now();
        let vars = pr_by_ids::Variables { ids: ids.to_vec() };
        let (data, rate) = self.run::<PrByIds>(vars).await?;
        Ok((parse::pr_by_ids::prs(data, me, started_at)?, rate))
    }

    /// The stack with the GraphQL node id `id` and its entries, position 1
    /// first. `None` when the id does not name a stack.
    pub(crate) async fn stack_entries(
        &self,
        id: &str,
    ) -> Result<(Option<Observed<Stack>>, RateInfo), GithubError> {
        let started_at = Utc::now();
        let vars = stack_entries::Variables { id: id.to_owned() };
        let (data, rate) = self.run::<StackEntries>(vars).await?;
        Ok((parse::stack_entries::stack(data, started_at)?, rate))
    }

    /// The CI state of the pull requests `visible` and, when `selected` is
    /// set, the state and the checks of that pull request. All ids are
    /// GraphQL node ids.
    pub(crate) async fn ci_poll(
        &self,
        visible: &[String],
        selected: Option<&str>,
    ) -> Result<(CiPollResult, RateInfo), GithubError> {
        let started_at = Utc::now();
        let vars = ci_poll::Variables {
            visible: visible.to_vec(),
            selected: selected.unwrap_or_default().to_owned(),
            with_selected: selected.is_some(),
        };
        let (data, rate) = self.run::<CiPoll>(vars).await?;
        Ok((parse::ci_poll::result(data, started_at), rate))
    }

    /// The comments, reviews and review threads of the pull request `key`.
    /// `None` when the repo or the pull request does not exist.
    pub(crate) async fn pr_detail(
        &self,
        key: &PrKey,
    ) -> Result<(Option<Observed<domain::detail::PrDetail>>, RateInfo), GithubError> {
        let started_at = Utc::now();
        let vars = pr_detail::Variables {
            owner: key.repo.owner.clone(),
            name: key.repo.name.clone(),
            number: i64::try_from(key.number)
                .map_err(|_| GithubError::Parse(format!("number out of range: {}", key.number)))?,
        };
        let (data, rate) = self.run::<PrDetail>(vars).await?;
        Ok((parse::pr_detail::detail(data, started_at)?, rate))
    }

    /// Submits a review of the pull request with the node id `id`. A
    /// request for changes needs a `body`.
    pub(crate) async fn add_review(
        &self,
        id: &str,
        event: ReviewEvent,
        body: Option<&str>,
    ) -> Result<RateInfo, GithubError> {
        let vars = add_review::Variables {
            id: id.to_owned(),
            event: match event {
                ReviewEvent::Approve => add_review::PullRequestReviewEvent::APPROVE,
                ReviewEvent::RequestChanges => add_review::PullRequestReviewEvent::REQUEST_CHANGES,
            },
            body: body.map(str::to_owned),
        };
        let (data, rate) = self.run::<AddReview>(vars).await?;
        data.add_pull_request_review
            .and_then(|payload| payload.pull_request_review)
            .ok_or_else(|| no_payload("addPullRequestReview"))?;
        Ok(rate)
    }

    /// Marks the draft pull request with the node id `id` as ready for
    /// review.
    pub(crate) async fn mark_ready(&self, id: &str) -> Result<RateInfo, GithubError> {
        let vars = mark_ready::Variables { id: id.to_owned() };
        let (data, rate) = self.run::<MarkReady>(vars).await?;
        data.mark_pull_request_ready_for_review
            .and_then(|payload| payload.pull_request)
            .ok_or_else(|| no_payload("markPullRequestReadyForReview"))?;
        Ok(rate)
    }

    /// Converts the pull request with the node id `id` to a draft.
    pub(crate) async fn convert_to_draft(&self, id: &str) -> Result<RateInfo, GithubError> {
        let vars = convert_to_draft::Variables { id: id.to_owned() };
        let (data, rate) = self.run::<ConvertToDraft>(vars).await?;
        data.convert_pull_request_to_draft
            .and_then(|payload| payload.pull_request)
            .ok_or_else(|| no_payload("convertPullRequestToDraft"))?;
        Ok(rate)
    }

    /// Closes the pull request with the node id `id` without a merge.
    pub(crate) async fn close_pr(&self, id: &str) -> Result<RateInfo, GithubError> {
        let vars = close_pr::Variables { id: id.to_owned() };
        let (data, rate) = self.run::<ClosePr>(vars).await?;
        data.close_pull_request
            .and_then(|payload| payload.pull_request)
            .ok_or_else(|| no_payload("closePullRequest"))?;
        Ok(rate)
    }
}

/// One page of a pull request search.
#[derive(Debug)]
pub(crate) struct PrPage {
    pub prs: Vec<PullRequest>,
    /// The cursor of the next page. `None` on the last page.
    pub next: Option<String>,
    pub rate: RateInfo,
    /// The start time of the request.
    pub started_at: chrono::DateTime<Utc>,
}

/// The pages of a pull request search.
#[derive(Debug)]
pub(crate) struct PrPages {
    pub prs: Vec<PullRequest>,
    /// True when the last page was read, false when `max_pages` stopped it.
    pub complete: bool,
    /// The rate limit info of the last page.
    pub rate: RateInfo,
    /// The start time of the first page.
    pub started_at: chrono::DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::github::graphql::tests::client;

    const ME: Me<'static> = Me {
        login: "chaosteil",
        teams: &[],
    };

    fn fixture(json: &str) -> Value {
        serde_json::from_str(json).unwrap()
    }

    /// Serves page 1, and page 2 for the cursor of page 1. `last` makes
    /// page 2 the last page.
    async fn pages(server: &MockServer, last: bool) {
        let mut page2 = fixture(include_str!("testdata/list_prs_page2.json"));
        if last {
            page2["data"]["search"]["pageInfo"]["hasNextPage"] = json!(false);
        }
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("Y3Vyc29yOjM="))
            .respond_with(ResponseTemplate::new(200).set_body_json(page2))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("\"after\":null"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(fixture(include_str!("testdata/list_prs_page1.json"))),
            )
            .mount(server)
            .await;
    }

    fn numbers(prs: &[PullRequest]) -> Vec<u64> {
        prs.iter().map(|pr| pr.key.number).collect()
    }

    #[tokio::test]
    async fn viewer_returns_the_login() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("query Viewer"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(fixture(include_str!("testdata/viewer.json"))),
            )
            .mount(&server)
            .await;
        let (login, rate) = client(&server).viewer().await.unwrap();
        assert_eq!(login, "chaosteil");
        assert_eq!(rate.last_cost, 1);
    }

    #[tokio::test]
    async fn pages_follow_the_cursor_to_the_last_page() {
        let server = MockServer::start().await;
        pages(&server, true).await;
        let before = Utc::now();
        let result = client(&server)
            .list_prs_pages("is:pr", 3, 5, &ME)
            .await
            .unwrap();
        assert_eq!(numbers(&result.prs), [1589, 1587, 1586, 1574, 1573, 1572]);
        assert!(result.complete);
        assert!(result.started_at >= before);
        assert!(
            result
                .prs
                .iter()
                .all(|pr| pr.head.observed_at >= result.started_at)
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn pages_stop_at_the_page_limit() {
        let server = MockServer::start().await;
        pages(&server, false).await;
        let result = client(&server)
            .list_prs_pages("is:pr", 3, 2, &ME)
            .await
            .unwrap();
        assert_eq!(result.prs.len(), 6);
        assert!(!result.complete, "page 2 has a next page");

        let result = client(&server)
            .list_prs_pages("is:pr", 3, 1, &ME)
            .await
            .unwrap();
        assert_eq!(numbers(&result.prs), [1589, 1587, 1586]);
        assert!(!result.complete);
    }

    #[tokio::test]
    async fn list_prs_sends_the_variables() {
        let server = MockServer::start().await;
        pages(&server, true).await;
        let page = client(&server)
            .list_prs("is:pr author:chaosteil", 3, None, &ME)
            .await
            .unwrap();
        assert_eq!(page.next.as_deref(), Some("Y3Vyc29yOjM="));
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["operationName"], "ListPrs");
        assert_eq!(
            body["variables"],
            json!({ "search": "is:pr author:chaosteil", "first": 3, "after": null })
        );
    }

    #[tokio::test]
    async fn stack_entries_sends_the_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("query StackEntries"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(fixture(include_str!("testdata/stack_entries.json"))),
            )
            .mount(&server)
            .await;
        let before = Utc::now();
        let (stack, _) = client(&server).stack_entries("PRS_1").await.unwrap();
        let stack = stack.unwrap();
        assert!(stack.observed_at >= before);
        assert_eq!(stack.value.entries.len(), 6);
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["variables"], json!({ "id": "PRS_1" }));
    }

    #[tokio::test]
    async fn ci_poll_sends_the_selection() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("query CiPoll"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(fixture(include_str!("testdata/ci_poll.json"))),
            )
            .mount(&server)
            .await;
        let client = client(&server);
        let visible = vec!["PR_1".to_owned(), "PR_2".to_owned()];
        let (result, _) = client.ci_poll(&visible, Some("PR_2")).await.unwrap();
        assert_eq!(result.visible.len(), 2);
        assert!(result.selected.is_some());
        client.ci_poll(&visible[..1], None).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let bodies: Vec<Value> = requests
            .iter()
            .map(|request| {
                serde_json::from_slice::<Value>(&request.body).unwrap()["variables"].clone()
            })
            .collect();
        assert_eq!(
            bodies,
            [
                json!({ "visible": ["PR_1", "PR_2"], "selected": "PR_2", "withSelected": true }),
                json!({ "visible": ["PR_1"], "selected": "", "withSelected": false }),
            ]
        );
    }

    #[tokio::test]
    async fn pr_detail_sends_the_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("query PrDetail"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(fixture(include_str!("testdata/pr_detail.json"))),
            )
            .mount(&server)
            .await;
        let key = PrKey {
            repo: crate::domain::keys::RepoKey {
                owner: "acme".into(),
                name: "monorepo".into(),
            },
            number: 3311,
        };
        let before = Utc::now();
        let (detail, _) = client(&server).pr_detail(&key).await.unwrap();
        let detail = detail.unwrap();
        assert!(detail.observed_at >= before);
        assert_eq!(detail.value.threads.len(), 2);
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            body["variables"],
            json!({ "owner": "acme", "name": "monorepo", "number": 3311 })
        );
    }

    /// Serves `data` to every request, with rate limit headers, because a
    /// mutation cannot select `rateLimit`.
    async fn mutation_server(data: Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-remaining", "4990")
                    .insert_header("x-ratelimit-reset", "1790000000")
                    .set_body_json(json!({ "data": data })),
            )
            .mount(&server)
            .await;
        server
    }

    /// The operation name and the variables of the requests to `server`.
    async fn sent(server: &MockServer) -> Vec<(Value, Value)> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                (body["operationName"].clone(), body["variables"].clone())
            })
            .collect()
    }

    #[tokio::test]
    async fn add_review_sends_the_event_and_body() {
        let server = mutation_server(json!({
            "addPullRequestReview": { "pullRequestReview": { "id": "PRR_1", "state": "APPROVED" } }
        }))
        .await;
        let client = client(&server);
        let rate = client
            .add_review("PR_1", ReviewEvent::Approve, None)
            .await
            .unwrap();
        assert_eq!((rate.remaining, rate.last_cost), (4990, 0));
        client
            .add_review("PR_1", ReviewEvent::RequestChanges, Some("Please fix."))
            .await
            .unwrap();
        assert_eq!(
            sent(&server).await,
            [
                (
                    json!("AddReview"),
                    json!({ "id": "PR_1", "event": "APPROVE", "body": null })
                ),
                (
                    json!("AddReview"),
                    json!({ "id": "PR_1", "event": "REQUEST_CHANGES", "body": "Please fix." })
                ),
            ]
        );
    }

    #[tokio::test]
    async fn draft_ready_and_close_send_the_id() {
        let server = mutation_server(json!({
            "markPullRequestReadyForReview": { "pullRequest": { "id": "PR_1", "isDraft": false } },
            "convertPullRequestToDraft": { "pullRequest": { "id": "PR_1", "isDraft": true } },
            "closePullRequest": { "pullRequest": { "id": "PR_1", "state": "CLOSED" } }
        }))
        .await;
        let client = client(&server);
        client.mark_ready("PR_1").await.unwrap();
        client.convert_to_draft("PR_1").await.unwrap();
        client.close_pr("PR_1").await.unwrap();
        let id = json!({ "id": "PR_1" });
        assert_eq!(
            sent(&server).await,
            [
                (json!("MarkReady"), id.clone()),
                (json!("ConvertToDraft"), id.clone()),
                (json!("ClosePr"), id),
            ]
        );
    }

    #[tokio::test]
    async fn a_null_payload_is_an_error() {
        let server = mutation_server(json!({ "closePullRequest": null })).await;
        let err = client(&server).close_pr("PR_1").await.unwrap_err();
        assert!(
            matches!(&err, GithubError::Graphql(messages) if messages == &["closePullRequest returned no result"]),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn pr_by_ids_sends_the_ids() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("query PrByIds"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(fixture(include_str!("testdata/pr_by_ids.json"))),
            )
            .mount(&server)
            .await;
        let ids = vec!["PR_1".to_owned(), "PR_2".to_owned()];
        let (prs, _) = client(&server).pr_by_ids(&ids, &ME).await.unwrap();
        assert_eq!(numbers(&prs), [1550, 1549]);
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["variables"], json!({ "ids": ["PR_1", "PR_2"] }));
    }
}
