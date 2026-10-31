//! The sync worker: full and delta syncs of the pull request lists.
//!
//! It waits until the session has a login. Then it runs a full sync of
//! both scopes, and again every `refresh.full_secs` (15 min by default).
//! A full sync searches the open pull requests of the repo set, at most 5
//! pages. The page size is the last result count plus 10, between 20 and
//! 100. Only a sync that read the last page sends `gone` candidates: the
//! open pull requests that the worker saw before and that the result does
//! not hold. The worker sends the `Msg` first and writes the store second.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::FutureExt;

use crate::bus::{Msg, RateInfo, Scope};
use crate::domain::keys::PrKey;
use crate::domain::pr::PullRequest;
use crate::domain::repo::Repo;
use crate::github::GithubError;
use crate::github::budget::{RateBudget, page_size};
use crate::github::parse::Me;
use crate::workers::ctx::WorkerCtx;
use crate::workers::supervisor::Worker;
use crate::workers::{sync_error, wait_for_login};

/// The sync worker.
pub(crate) const WORKER: Worker = Worker {
    name: "sync",
    run: |ctx| run(ctx).boxed(),
};

/// The most pages of one search.
const MAX_PAGES: usize = 5;

/// Waits for the login, then runs the full syncs until `app` is gone.
async fn run(ctx: WorkerCtx) {
    let mut session = ctx.session.clone();
    if wait_for_login(&mut session).await.is_none() {
        return;
    }
    let mut syncer = Syncer::new(ctx);
    let period = Duration::from_secs(syncer.ctx.config.refresh.full_secs.max(1));
    loop {
        for scope in [Scope::Mine, Scope::Team] {
            if !syncer.full(scope).await {
                return;
            }
        }
        tokio::time::sleep(period).await;
    }
}

/// What the worker remembers of one scope.
#[derive(Debug, Default)]
struct ScopeState {
    /// The open pull requests that the worker saw in this scope.
    known: HashSet<PrKey>,
    /// The result count of the last full sync.
    full_count: usize,
}

/// The state of the sync worker.
struct Syncer {
    ctx: WorkerCtx,
    mine: ScopeState,
    team: ScopeState,
    budget: RateBudget,
}

impl Syncer {
    fn new(ctx: WorkerCtx) -> Self {
        Self {
            ctx,
            mine: ScopeState::default(),
            team: ScopeState::default(),
            budget: RateBudget::default(),
        }
    }

    fn state(&mut self, scope: Scope) -> &mut ScopeState {
        match scope {
            Scope::Mine => &mut self.mine,
            Scope::Team => &mut self.team,
        }
    }

    /// The login and the sorted repo names of the session. `None` without
    /// a login or without repos.
    fn target(&self) -> Option<(String, Vec<String>)> {
        let session = self.ctx.session.borrow();
        let login = session.login.clone()?;
        let repos = repo_names(&session.repos);
        (!repos.is_empty()).then_some((login, repos))
    }

    /// Runs a full sync of `scope`. False when `app` is gone.
    async fn full(&mut self, scope: Scope) -> bool {
        let Some((login, repos)) = self.target() else {
            tracing::debug!(?scope, "full sync skipped: no login or no repos");
            return true;
        };
        let query = search(scope, &login, &repos, None);
        let first = page_size(self.state(scope).full_count);
        let teams = self.ctx.config.teams.clone();
        let me = Me {
            login: &login,
            teams: &teams,
        };
        let result = self
            .ctx
            .client
            .list_prs_pages(&query, first, MAX_PAGES, &me)
            .await;
        let pages = match result {
            Ok(pages) => pages,
            Err(err) => return self.failed(scope, &repos, &err).await,
        };
        tracing::info!(
            ?scope,
            count = pages.prs.len(),
            complete = pages.complete,
            "full sync"
        );
        if !self.rate(pages.rate.clone()).await {
            return false;
        }
        let keys: HashSet<PrKey> = pages.prs.iter().map(|pr| pr.key.clone()).collect();
        let state = self.state(scope);
        state.full_count = pages.prs.len();
        let gone = pages.complete.then(|| {
            let mut gone: Vec<PrKey> = state.known.difference(&keys).cloned().collect();
            gone.sort();
            gone
        });
        if pages.complete {
            state.known = keys;
        } else {
            state.known.extend(keys);
        }
        let started_at = pages.started_at;
        let msg = Msg::PrsSynced {
            scope,
            prs: pages.prs.clone(),
            gone: gone.clone(),
            started_at,
        };
        if self.ctx.tx.send(msg).await.is_err() {
            return false;
        }
        self.persist_full(scope, &repos, pages.prs, gone, started_at)
            .await;
        true
    }

    /// Writes the result of a full sync: the pull requests, the deleted
    /// gone candidates and the sync state.
    async fn persist_full(
        &self,
        scope: Scope,
        repos: &[String],
        prs: Vec<PullRequest>,
        gone: Option<Vec<PrKey>>,
        started_at: DateTime<Utc>,
    ) {
        let store = &self.ctx.store;
        if let Err(err) = store.put_prs(scope, prs).await {
            tracing::warn!(?scope, %err, "cannot cache the pull requests");
        }
        let Some(gone) = gone else {
            return;
        };
        if let Err(err) = store.drop_gone(gone, started_at).await {
            tracing::warn!(?scope, %err, "cannot drop the gone pull requests");
        }
        let repo_set = repo_set_hash(repos);
        let mut state = match store.sync_state(scope, repo_set.clone()).await {
            Ok(state) => state.unwrap_or_default(),
            Err(err) => {
                tracing::warn!(?scope, %err, "cannot read the sync state");
                return;
            }
        };
        state.last_full = Some(started_at);
        // The next delta starts at the first full sync.
        state.last_delta = state.last_delta.or(Some(started_at));
        state.last_error = None;
        if let Err(err) = store.put_sync_state(scope, repo_set, state).await {
            tracing::warn!(?scope, %err, "cannot write the sync state");
        }
    }

    /// Records the rate info and sends `Rate`. False when `app` is gone.
    async fn rate(&mut self, rate: RateInfo) -> bool {
        self.budget.observe(rate.clone());
        self.ctx.tx.send(Msg::Rate(rate)).await.is_ok()
    }

    /// Sends `SyncFailed` for `scope` and writes the error into the sync
    /// state. False when `app` is gone.
    async fn failed(&mut self, scope: Scope, repos: &[String], err: &GithubError) -> bool {
        tracing::warn!(?scope, %err, "sync failed");
        let msg = Msg::SyncFailed {
            source: "sync",
            scope: Some(scope),
            err: sync_error(err),
        };
        if self.ctx.tx.send(msg).await.is_err() {
            return false;
        }
        let store = &self.ctx.store;
        let repo_set = repo_set_hash(repos);
        if let Ok(state) = store.sync_state(scope, repo_set.clone()).await {
            let mut state = state.unwrap_or_default();
            state.last_error = Some(err.to_string());
            if let Err(err) = store.put_sync_state(scope, repo_set, state).await {
                tracing::warn!(?scope, %err, "cannot write the sync state");
            }
        }
        true
    }
}

/// The `owner/name` of every repo, sorted and without duplicates.
fn repo_names(repos: &[Repo]) -> Vec<String> {
    let mut names: Vec<String> = repos
        .iter()
        .map(|repo| format!("{}/{}", repo.key.owner, repo.key.name))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The `sync_state` key of a repo set: a 64-bit FNV-1a hash of the
/// sorted names, in hex. It stays the same across builds.
fn repo_set_hash(names: &[String]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in names.join(",").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

/// The search string of a sync. `since` is `None` for a full sync, which
/// adds `is:open`. A delta has no `is:open`, so merges and closes arrive,
/// and adds `updated:>=<since>`. Team excludes the pull requests of the
/// user.
fn search(scope: Scope, login: &str, repos: &[String], since: Option<DateTime<Utc>>) -> String {
    let mut query = String::from("is:pr");
    if since.is_none() {
        query.push_str(" is:open");
    }
    let author = match scope {
        Scope::Mine => "author",
        Scope::Team => "-author",
    };
    let _ = write!(query, " {author}:{login}");
    for repo in repos {
        let _ = write!(query, " repo:{repo}");
    }
    if let Some(since) = since {
        let _ = write!(query, " updated:>={}", since.format("%Y-%m-%dT%H:%M:%SZ"));
    }
    query
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::bus::{Session, SyncError};
    use crate::domain::keys::RepoKey;
    use crate::workers::testkit::{self, Harness};

    const PAGE1: &str = include_str!("../github/testdata/list_prs_page1.json");
    const PAGE2: &str = include_str!("../github/testdata/list_prs_page2.json");

    /// Serves page 1 for the first request and page 2 for every cursor.
    /// `last` makes page 2 the last page.
    async fn serve_pages(server: &MockServer, last: bool) {
        let mut page2: Value = serde_json::from_str(PAGE2).unwrap();
        page2["data"]["search"]["pageInfo"]["hasNextPage"] = json!(!last);
        Mock::given(method("POST"))
            .and(body_string_contains("\"after\":\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(page2))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("\"after\":null"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(PAGE1.as_bytes(), "application/json"),
            )
            .mount(server)
            .await;
    }

    /// A context whose session has a login and the repo `acme/deploy`.
    fn logged_in(server: &MockServer) -> (WorkerCtx, Harness) {
        let (ctx, harness) = testkit::ctx(&server.uri());
        harness
            .session
            .send(Session {
                login: Some("chaosteil".into()),
                repos: vec![Repo {
                    key: RepoKey {
                        owner: "acme".into(),
                        name: "deploy".into(),
                    },
                    path: None,
                    settings: None,
                }],
            })
            .unwrap();
        (ctx, harness)
    }

    /// The scope, the numbers and the gone numbers of the next
    /// `PrsSynced`, after the `Rate` before it.
    async fn synced(harness: &mut Harness) -> (Scope, Vec<u64>, Option<Vec<u64>>) {
        assert!(matches!(harness.recv().await, Msg::Rate(_)));
        match harness.recv().await {
            Msg::PrsSynced {
                scope, prs, gone, ..
            } => (
                scope,
                prs.iter().map(|pr| pr.key.number).collect(),
                gone.map(|keys| keys.iter().map(|key| key.number).collect()),
            ),
            other => panic!("{other:?}"),
        }
    }

    /// The search variables of the requests to `server`.
    async fn searches(server: &MockServer) -> Vec<(String, u64)> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let vars = &body["variables"];
                (
                    vars["search"].as_str().unwrap().to_owned(),
                    vars["first"].as_u64().unwrap(),
                )
            })
            .collect()
    }

    fn key(number: u64) -> PrKey {
        PrKey {
            repo: RepoKey {
                owner: "acme".into(),
                name: "deploy".into(),
            },
            number,
        }
    }

    #[tokio::test]
    async fn a_full_sync_reads_two_pages_and_sends_gone_candidates() {
        let server = MockServer::start().await;
        serve_pages(&server, true).await;
        let (ctx, mut harness) = logged_in(&server);
        let store = ctx.store.clone();
        let mut syncer = Syncer::new(ctx);
        syncer.mine.known.insert(key(1));
        syncer.mine.known.insert(key(1589));

        assert!(syncer.full(Scope::Mine).await);
        assert_eq!(
            synced(&mut harness).await,
            (
                Scope::Mine,
                vec![1589, 1587, 1586, 1574, 1573, 1572],
                Some(vec![1])
            )
        );
        assert_eq!(
            searches(&server).await,
            [
                (
                    "is:pr is:open author:chaosteil repo:acme/deploy".to_owned(),
                    20
                ),
                (
                    "is:pr is:open author:chaosteil repo:acme/deploy".to_owned(),
                    20
                ),
            ]
        );
        assert_eq!(syncer.mine.known.len(), 6);
        assert_eq!(syncer.mine.full_count, 6);

        let state = store
            .sync_state(Scope::Mine, repo_set_hash(&["acme/deploy".to_owned()]))
            .await
            .unwrap()
            .unwrap();
        assert!(state.last_full.is_some());
        assert_eq!(state.last_delta, state.last_full);
        assert_eq!(state.last_error, None);
    }

    #[tokio::test]
    async fn a_full_sync_cut_at_five_pages_sends_no_gone() {
        let server = MockServer::start().await;
        serve_pages(&server, false).await;
        let (ctx, mut harness) = logged_in(&server);
        let mut syncer = Syncer::new(ctx);
        syncer.team.known.insert(key(1));

        assert!(syncer.full(Scope::Team).await);
        let (scope, numbers, gone) = synced(&mut harness).await;
        assert_eq!(scope, Scope::Team);
        assert_eq!(numbers.len(), 15);
        assert_eq!(gone, None);
        assert_eq!(server.received_requests().await.unwrap().len(), MAX_PAGES);
        assert!(syncer.team.known.contains(&key(1)));
        assert!(searches(&server).await[0].0.contains(" -author:chaosteil "));
    }

    #[tokio::test]
    async fn the_worker_waits_for_the_login_and_syncs_both_scopes() {
        let server = MockServer::start().await;
        serve_pages(&server, true).await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        let worker = tokio::spawn(run(ctx));
        assert!(harness.quiet_for(Duration::from_millis(100)).await);
        harness
            .session
            .send(Session {
                login: Some("chaosteil".into()),
                repos: vec![Repo {
                    key: key(0).repo,
                    path: None,
                    settings: None,
                }],
            })
            .unwrap();
        assert_eq!(synced(&mut harness).await.0, Scope::Mine);
        assert_eq!(synced(&mut harness).await.0, Scope::Team);
        worker.abort();
    }

    #[tokio::test]
    async fn no_network_is_an_offline_sync_failure() {
        let (ctx, mut harness) = testkit::ctx("http://127.0.0.1:1");
        harness
            .session
            .send(Session {
                login: Some("chaosteil".into()),
                repos: vec![Repo {
                    key: key(0).repo,
                    path: None,
                    settings: None,
                }],
            })
            .unwrap();
        let mut syncer = Syncer::new(ctx);
        assert!(syncer.full(Scope::Mine).await);
        assert_eq!(
            harness.recv().await,
            Msg::SyncFailed {
                source: "sync",
                scope: Some(Scope::Mine),
                err: SyncError::Offline,
            }
        );
    }

    #[test]
    fn repo_names_are_sorted_and_unique() {
        let repo = |owner: &str, name: &str| Repo {
            key: RepoKey {
                owner: owner.into(),
                name: name.into(),
            },
            path: None,
            settings: None,
        };
        assert_eq!(
            repo_names(&[repo("c", "d"), repo("a", "b"), repo("c", "d")]),
            ["a/b", "c/d"]
        );
    }

    #[test]
    fn repo_set_hash_is_stable() {
        assert_eq!(repo_set_hash(&[]), "cbf29ce484222325");
        let one = repo_set_hash(&["a/b".to_owned(), "c/d".to_owned()]);
        assert_eq!(one.len(), 16);
        assert_ne!(one, repo_set_hash(&["a/b".to_owned()]));
    }

    #[test]
    fn full_search_strings() {
        let repos = ["a/b".to_owned(), "c/d".to_owned()];
        assert_eq!(
            search(Scope::Mine, "me", &repos, None),
            "is:pr is:open author:me repo:a/b repo:c/d"
        );
        assert_eq!(
            search(Scope::Team, "me", &repos, None),
            "is:pr is:open -author:me repo:a/b repo:c/d"
        );
    }
}
