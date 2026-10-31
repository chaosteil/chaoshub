//! Typed GraphQL queries, one per file in `queries/`.
//!
//! Each struct derives `GraphQLQuery` from one `.graphql` file and the
//! vendored schema. The derive checks every field against the schema at
//! compile time. Every query selects `rateLimit { cost remaining resetAt }`.

use chrono::Utc;
use graphql_client::GraphQLQuery;

use crate::bus::RateInfo;
use crate::domain::observed::Observed;
use crate::domain::pr::PullRequest;
use crate::domain::stack::Stack;
use crate::github::parse::{self, Me};
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
