//! The sync worker: full and delta syncs of the pull request lists.
//!
//! It waits until the session has a login. Then it runs a full sync of
//! both scopes at once, and again every `refresh.full_secs` (15 min by
//! default) and when the repo set changes. A delta sync of Mine runs every
//! `refresh.mine_secs` (60 s), of Team every `refresh.team_secs` (90 s).
//! When the rate budget does not last until the reset, the intervals
//! double. After a rate limit, no timed sync runs until its end. The
//! worker also serves `SyncReq::Full(scope)`, `SyncReq::Refresh`, which
//! reads pull requests by node id, and `SyncReq::Load`, which reads one
//! pull request by number.
//!
//! In the pinned mode (`chaoshub pr`) the worker runs no list sync. It
//! serves the refresh and load requests, and every `refresh.mine_secs` it
//! refreshes the selected pull request, or loads the pinned one when it is
//! not loaded yet. It writes no pull request rows, because the list
//! process owns the rows of the cache.
//!
//! A full sync searches the open pull requests of the repo set, at most 5
//! pages. A delta searches all states, updated since `last_delta` minus
//! 2 min, so merges and closes arrive. `last_delta` advances to the start
//! of the delta only after its last page. The page size is the last result count plus 10, between 20 and
//! 100. Only a sync that read the last page sends `gone` candidates: the
//! open pull requests that the worker saw before and that the result does
//! not hold. The worker sends the `Msg` first and writes the store second.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use futures::FutureExt;
use tokio::time::Instant;

use crate::bus::{Msg, PrRef, RateInfo, Scope, SyncError, SyncReq};
use crate::domain::keys::PrKey;
use crate::domain::pr::{PrState, PullRequest};
use crate::domain::repo::Repo;
use crate::github::GithubError;
use crate::github::budget::{RateBudget, page_size};
use crate::github::parse::Me;
use crate::workers::ctx::{WorkerCtx, WorkerMode};
use crate::workers::supervisor::Worker;
use crate::workers::{sync_error, wait_for_login};

/// The sync worker.
pub(crate) const WORKER: Worker = Worker {
    name: "sync",
    run: |ctx| run(ctx).boxed(),
};

/// The most pages of one search.
const MAX_PAGES: usize = 5;

/// The overlap of a delta with the previous one, because the search index
/// lags.
const DELTA_OVERLAP: TimeDelta = TimeDelta::minutes(2);

/// Waits for the login, then serves the schedule and the requests until
/// `app` is gone.
async fn run(ctx: WorkerCtx) {
    let mut session = ctx.session.clone();
    if wait_for_login(&mut session).await.is_none() {
        return;
    }
    let sync_rx = Arc::clone(&ctx.sync_rx);
    let mut requests = sync_rx.lock().await;
    let refresh = ctx.config.refresh.clone();
    if let WorkerMode::Pinned(key) = ctx.mode.clone() {
        run_pinned(Syncer::new(ctx), &mut requests, &key, refresh.mine_secs).await;
        return;
    }
    let mut syncer = Syncer::new(ctx);
    let mut repos = syncer.target().map(|(_, repos)| repos);
    if !syncer.full_both().await {
        return;
    }
    let now = Instant::now();
    let mut next_full = now + syncer.interval(refresh.full_secs);
    let mut next_mine = now + syncer.interval(refresh.mine_secs);
    let mut next_team = now + syncer.interval(refresh.team_secs);
    loop {
        let mut due = next_full.min(next_mine).min(next_team);
        if let Some(paused) = syncer.paused_until {
            due = due.max(paused);
        }
        tokio::select! {
            request = requests.recv() => {
                let alive = match request {
                    Some(SyncReq::Full(scope)) => syncer.full(scope).await,
                    Some(SyncReq::Refresh(refs)) => syncer.refresh(&refs).await,
                    Some(SyncReq::Load(key)) => syncer.load(&key).await,
                    None => false,
                };
                if !alive {
                    return;
                }
            }
            changed = session.changed() => {
                if changed.is_err() {
                    return;
                }
                let current = syncer.target().map(|(_, repos)| repos);
                if current != repos {
                    repos = current;
                    if !syncer.full_both().await {
                        return;
                    }
                    next_full = Instant::now() + syncer.interval(refresh.full_secs);
                }
            }
            () = tokio::time::sleep_until(due) => {
                let now = Instant::now();
                if next_full <= now {
                    if !syncer.full_both().await {
                        return;
                    }
                    next_full = Instant::now() + syncer.interval(refresh.full_secs);
                }
                if next_mine <= now {
                    if !syncer.delta(Scope::Mine).await {
                        return;
                    }
                    next_mine = Instant::now() + syncer.interval(refresh.mine_secs);
                }
                if next_team <= now {
                    if !syncer.delta(Scope::Team).await {
                        return;
                    }
                    next_team = Instant::now() + syncer.interval(refresh.team_secs);
                }
            }
        }
    }
}

/// The loop of the pinned mode: the requests, and one tick every
/// `mine_secs`. The tick refreshes the selected pull request of the
/// interest, or loads the pinned pull request `key` while no pull request
/// is selected. It returns when `app` is gone.
async fn run_pinned(
    mut syncer: Syncer,
    requests: &mut tokio::sync::mpsc::Receiver<SyncReq>,
    key: &PrKey,
    mine_secs: u64,
) {
    let mut next = Instant::now() + syncer.interval(mine_secs);
    loop {
        let mut due = next;
        if let Some(paused) = syncer.paused_until {
            due = due.max(paused);
        }
        tokio::select! {
            request = requests.recv() => {
                let alive = match request {
                    Some(SyncReq::Full(scope)) => {
                        tracing::debug!(?scope, "full sync ignored in the pinned mode");
                        true
                    }
                    Some(SyncReq::Refresh(refs)) => syncer.refresh(&refs).await,
                    Some(SyncReq::Load(key)) => syncer.load(&key).await,
                    None => false,
                };
                if !alive {
                    return;
                }
            }
            () = tokio::time::sleep_until(due) => {
                let selected = syncer.ctx.interest.borrow().selected.clone();
                let alive = match selected {
                    Some(selected) => syncer.refresh(&[selected]).await,
                    None => syncer.load(key).await,
                };
                if !alive {
                    return;
                }
                next = Instant::now() + syncer.interval(mine_secs);
            }
        }
    }
}

/// What the worker remembers of one scope.
#[derive(Debug, Default)]
struct ScopeState {
    /// The open pull requests that the worker saw in this scope.
    known: HashSet<PrKey>,
    /// The result count of the last full sync.
    full_count: usize,
    /// The result count of the last delta sync.
    delta_count: usize,
}

impl ScopeState {
    /// Records the pull requests of a delta or a refresh: open ones are
    /// known, others are not.
    fn note(&mut self, prs: &[PullRequest]) {
        for pr in prs {
            if pr.head.value.state == PrState::Open {
                self.known.insert(pr.key.clone());
            } else {
                self.known.remove(&pr.key);
            }
        }
    }
}

/// The state of the sync worker.
struct Syncer {
    ctx: WorkerCtx,
    mine: ScopeState,
    team: ScopeState,
    budget: RateBudget,
    /// The end of a rate limit. No timed sync runs before it.
    paused_until: Option<Instant>,
}

impl Syncer {
    fn new(ctx: WorkerCtx) -> Self {
        Self {
            ctx,
            mine: ScopeState::default(),
            team: ScopeState::default(),
            budget: RateBudget::default(),
            paused_until: None,
        }
    }

    /// The interval for a base of `secs` seconds, doubled while the rate
    /// budget does not last until the reset.
    fn interval(&self, secs: u64) -> Duration {
        let base = Duration::from_secs(secs.max(1));
        let interval = self.budget.interval(base);
        if interval > base {
            tracing::info!(?base, ?interval, "slowing: rate budget");
        }
        interval
    }

    /// Runs a full sync of both scopes. False when `app` is gone.
    async fn full_both(&mut self) -> bool {
        self.full(Scope::Mine).await && self.full(Scope::Team).await
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

    /// Runs a delta sync of `scope` from `last_delta` of the sync state.
    /// Without a `last_delta` it waits for a full sync. False when `app` is
    /// gone.
    async fn delta(&mut self, scope: Scope) -> bool {
        let Some((login, repos)) = self.target() else {
            return true;
        };
        let repo_set = repo_set_hash(&repos);
        let mut state = match self.ctx.store.sync_state(scope, repo_set.clone()).await {
            Ok(state) => state.unwrap_or_default(),
            Err(err) => {
                tracing::warn!(?scope, %err, "cannot read the sync state");
                return true;
            }
        };
        let Some(last_delta) = state.last_delta else {
            tracing::debug!(?scope, "delta skipped: no full sync yet");
            return true;
        };
        let query = search(scope, &login, &repos, Some(last_delta - DELTA_OVERLAP));
        let first = page_size(self.state(scope).delta_count);
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
        tracing::debug!(
            ?scope,
            count = pages.prs.len(),
            complete = pages.complete,
            "delta sync"
        );
        if !self.rate(pages.rate.clone()).await {
            return false;
        }
        let scope_state = self.state(scope);
        scope_state.delta_count = pages.prs.len();
        scope_state.note(&pages.prs);
        let msg = Msg::PrsSynced {
            scope,
            prs: pages.prs.clone(),
            gone: None,
            started_at: pages.started_at,
        };
        if self.ctx.tx.send(msg).await.is_err() {
            return false;
        }
        if let Err(err) = self.ctx.store.put_prs(scope, pages.prs).await {
            tracing::warn!(?scope, %err, "cannot cache the pull requests");
        }
        if pages.complete {
            state.last_delta = Some(pages.started_at);
            state.last_error = None;
            if let Err(err) = self.ctx.store.put_sync_state(scope, repo_set, state).await {
                tracing::warn!(?scope, %err, "cannot write the sync state");
            }
        }
        true
    }

    /// Reads the pull requests of `refs` by node id and sends one
    /// `PrRefreshed` per pull request. False when `app` is gone.
    async fn refresh(&mut self, refs: &[PrRef]) -> bool {
        let Some(login) = self.ctx.session.borrow().login.clone() else {
            return true;
        };
        let ids: Vec<String> = refs.iter().map(|pr| pr.node_id.clone()).collect();
        if ids.is_empty() {
            return true;
        }
        let teams = self.ctx.config.teams.clone();
        let me = Me {
            login: &login,
            teams: &teams,
        };
        let (prs, rate) = match self.ctx.client.pr_by_ids(&ids, &me).await {
            Ok(ok) => ok,
            Err(err) => return self.refresh_failed(&err).await,
        };
        if !self.rate(rate).await {
            return false;
        }
        self.refreshed(&login, prs).await
    }

    /// Reads the pull request `key` by number and sends `PrRefreshed`, or
    /// `SyncFailed` when the repo or the pull request does not exist.
    /// False when `app` is gone.
    async fn load(&mut self, key: &PrKey) -> bool {
        let Some(login) = self.ctx.session.borrow().login.clone() else {
            return true;
        };
        let teams = self.ctx.config.teams.clone();
        let me = Me {
            login: &login,
            teams: &teams,
        };
        let (pr, rate) = match self.ctx.client.pr_by_number(key, &me).await {
            Ok(ok) => ok,
            Err(err) => return self.refresh_failed(&err).await,
        };
        if !self.rate(rate).await {
            return false;
        }
        let Some(pr) = pr else {
            let msg = Msg::SyncFailed {
                source: "sync",
                scope: None,
                err: SyncError::Other(format!(
                    "#{} is not in {}/{}",
                    key.number, key.repo.owner, key.repo.name
                )),
            };
            return self.ctx.tx.send(msg).await.is_ok();
        };
        self.refreshed(&login, vec![pr]).await
    }

    /// Sends `SyncFailed` for a failed refresh or load. False when `app`
    /// is gone.
    async fn refresh_failed(&mut self, err: &GithubError) -> bool {
        tracing::warn!(%err, "refresh failed");
        let msg = Msg::SyncFailed {
            source: "sync",
            scope: None,
            err: sync_error(err),
        };
        self.pause_on_limit(err);
        self.ctx.tx.send(msg).await.is_ok()
    }

    /// Notes the refreshed pull requests `prs` in the scope of their
    /// author, sends one `PrRefreshed` per pull request, and caches them in
    /// the `Lists` mode. False when `app` is gone.
    async fn refreshed(&mut self, login: &str, prs: Vec<PullRequest>) -> bool {
        let (mine, team): (Vec<PullRequest>, Vec<PullRequest>) = prs
            .into_iter()
            .partition(|pr| pr.head.value.author == login);
        for (scope, prs) in [(Scope::Mine, mine), (Scope::Team, team)] {
            if prs.is_empty() {
                continue;
            }
            self.state(scope).note(&prs);
            for pr in &prs {
                if self
                    .ctx
                    .tx
                    .send(Msg::PrRefreshed(Box::new(pr.clone())))
                    .await
                    .is_err()
                {
                    return false;
                }
            }
            if self.ctx.mode == WorkerMode::Lists
                && let Err(err) = self.ctx.store.put_prs(scope, prs).await
            {
                tracing::warn!(?scope, %err, "cannot cache the pull requests");
            }
        }
        true
    }

    /// Stops the timed syncs until the end of a rate limit.
    fn pause_on_limit(&mut self, err: &GithubError) {
        if let GithubError::RateLimited { until } = err {
            let wait = (*until - Utc::now()).to_std().unwrap_or_default();
            self.paused_until = Some(Instant::now() + wait);
        }
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
        self.pause_on_limit(err);
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

    /// The search variables of the search requests to `server`.
    async fn searches(server: &MockServer) -> Vec<(String, u64)> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter_map(|request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let vars = &body["variables"];
                Some((vars["search"].as_str()?.to_owned(), vars["first"].as_u64()?))
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

    const BY_IDS: &str = include_str!("../github/testdata/pr_by_ids.json");

    #[tokio::test]
    async fn a_delta_searches_since_last_delta_and_advances_it() {
        let server = MockServer::start().await;
        serve_pages(&server, true).await;
        let (ctx, mut harness) = logged_in(&server);
        let store = ctx.store.clone();
        let repo_set = repo_set_hash(&["acme/deploy".to_owned()]);
        let last = "2026-10-30T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        store
            .put_sync_state(
                Scope::Mine,
                repo_set.clone(),
                crate::store::SyncState {
                    last_full: Some(last),
                    last_delta: Some(last),
                    last_error: None,
                },
            )
            .await
            .unwrap();
        let mut syncer = Syncer::new(ctx);
        syncer.mine.known.insert(key(1589));

        assert!(syncer.delta(Scope::Mine).await);
        assert_eq!(
            synced(&mut harness).await,
            (Scope::Mine, vec![1589, 1587, 1586, 1574, 1573, 1572], None)
        );
        assert_eq!(
            searches(&server).await[0],
            (
                "is:pr author:chaosteil repo:acme/deploy updated:>=2026-10-30T11:58:00Z".to_owned(),
                20
            )
        );
        assert!(
            syncer.mine.known.is_empty(),
            "merged pull requests are not known"
        );
        assert_eq!(syncer.mine.delta_count, 6);
        let state = store
            .sync_state(Scope::Mine, repo_set)
            .await
            .unwrap()
            .unwrap();
        assert!(state.last_delta.unwrap() > last);
        assert_eq!(state.last_full, Some(last));
    }

    #[tokio::test]
    async fn a_delta_cut_at_five_pages_keeps_last_delta() {
        let server = MockServer::start().await;
        serve_pages(&server, false).await;
        let (ctx, mut harness) = logged_in(&server);
        let store = ctx.store.clone();
        let repo_set = repo_set_hash(&["acme/deploy".to_owned()]);
        let last = "2026-10-30T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let state = crate::store::SyncState {
            last_full: Some(last),
            last_delta: Some(last),
            last_error: None,
        };
        store
            .put_sync_state(Scope::Team, repo_set.clone(), state.clone())
            .await
            .unwrap();
        let mut syncer = Syncer::new(ctx);
        assert!(syncer.delta(Scope::Team).await);
        assert_eq!(synced(&mut harness).await.2, None);
        assert_eq!(
            store.sync_state(Scope::Team, repo_set).await.unwrap(),
            Some(state)
        );
    }

    #[tokio::test]
    async fn a_delta_without_last_delta_sends_no_request() {
        let server = MockServer::start().await;
        let (ctx, mut harness) = logged_in(&server);
        let mut syncer = Syncer::new(ctx);
        assert!(syncer.delta(Scope::Mine).await);
        assert!(harness.quiet_for(Duration::from_millis(50)).await);
        assert_eq!(server.received_requests().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn the_worker_serves_refresh_requests_and_runs_deltas() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("query PrByIds"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(BY_IDS.as_bytes(), "application/json"),
            )
            .mount(&server)
            .await;
        serve_pages(&server, true).await;
        let mut config = crate::config::Config::default();
        config.refresh.mine_secs = 1;
        let (ctx, mut harness) = testkit::ctx_with(&server.uri(), config, testkit::memory_store());
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
        let worker = tokio::spawn(run(ctx));
        assert_eq!(synced(&mut harness).await.0, Scope::Mine);
        assert_eq!(synced(&mut harness).await.0, Scope::Team);

        let refs = vec![
            testkit::pr_ref(1550, "PR_1", "a"),
            testkit::pr_ref(1549, "PR_2", "b"),
        ];
        harness.sync.send(SyncReq::Refresh(refs)).await.unwrap();
        assert!(matches!(harness.recv().await, Msg::Rate(_)));
        let mut refreshed = Vec::new();
        for _ in 0..2 {
            match harness.recv().await {
                Msg::PrRefreshed(pr) => refreshed.push(pr.key.number),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(refreshed, [1550, 1549]);

        let (scope, _, gone) = synced(&mut harness).await;
        assert_eq!((scope, gone), (Scope::Mine, None));
        let delta = searches(&server)
            .await
            .into_iter()
            .any(|(search, _)| search.contains(" updated:>="));
        assert!(delta, "a delta search ran");
        worker.abort();
    }

    #[tokio::test]
    async fn a_full_request_runs_a_full_sync() {
        let server = MockServer::start().await;
        serve_pages(&server, true).await;
        let (ctx, mut harness) = logged_in(&server);
        let worker = tokio::spawn(run(ctx));
        synced(&mut harness).await;
        synced(&mut harness).await;
        harness.sync.send(SyncReq::Full(Scope::Team)).await.unwrap();
        let (scope, _, gone) = synced(&mut harness).await;
        assert_eq!(scope, Scope::Team);
        assert_eq!(gone, Some(Vec::new()));
        worker.abort();
    }

    const BY_NUMBER: &str = include_str!("../github/testdata/pr_by_number.json");

    /// Serves `body` to the `PrByNumber` queries.
    async fn serve_by_number(server: &MockServer, body: &str) {
        Mock::given(method("POST"))
            .and(body_string_contains("query PrByNumber"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(body.as_bytes().to_vec(), "application/json"),
            )
            .mount(server)
            .await;
    }

    /// The numbers of the pull requests in the cache at `path`.
    fn cached(path: &std::path::Path) -> Vec<u64> {
        let (snapshot, _) = crate::store::Store::open(path).unwrap();
        snapshot.prs.iter().map(|(_, pr)| pr.key.number).collect()
    }

    #[tokio::test]
    async fn a_load_sends_the_pull_request_and_caches_it() {
        let server = MockServer::start().await;
        serve_by_number(&server, BY_NUMBER).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let store = crate::store::Store::open(&path).unwrap().1;
        let (ctx, mut harness) =
            testkit::ctx_with(&server.uri(), crate::config::Config::default(), store);
        harness
            .session
            .send(Session {
                login: Some("chaosteil".into()),
                repos: Vec::new(),
            })
            .unwrap();
        let mut syncer = Syncer::new(ctx);

        assert!(syncer.load(&key(1550)).await);
        assert!(matches!(harness.recv().await, Msg::Rate(_)));
        match harness.recv().await {
            Msg::PrRefreshed(pr) => assert_eq!(pr.key, key(1550)),
            other => panic!("{other:?}"),
        }
        assert!(syncer.mine.known.contains(&key(1550)));
        assert_eq!(cached(&path), [1550]);
    }

    #[tokio::test]
    async fn a_load_of_a_null_pull_request_is_a_sync_failure() {
        let server = MockServer::start().await;
        let body = json!({
            "data": {
                "repository": { "pullRequest": null },
                "rateLimit": { "cost": 1, "remaining": 4990, "resetAt": "2026-10-31T05:50:43Z" }
            }
        });
        serve_by_number(&server, &body.to_string()).await;
        let (ctx, mut harness) = logged_in(&server);
        let mut syncer = Syncer::new(ctx);

        assert!(syncer.load(&key(7)).await);
        assert!(matches!(harness.recv().await, Msg::Rate(_)));
        assert_eq!(
            harness.recv().await,
            Msg::SyncFailed {
                source: "sync",
                scope: None,
                err: SyncError::Other("#7 is not in acme/deploy".into()),
            }
        );
    }

    /// The operation names of the requests to `server`.
    async fn operations(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                body["operationName"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
    }

    #[tokio::test]
    async fn the_pinned_worker_loads_then_refreshes_and_writes_no_rows() {
        let server = MockServer::start().await;
        serve_by_number(&server, BY_NUMBER).await;
        Mock::given(method("POST"))
            .and(body_string_contains("query PrByIds"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(BY_IDS.as_bytes(), "application/json"),
            )
            .mount(&server)
            .await;
        serve_pages(&server, true).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let store = crate::store::Store::open(&path).unwrap().1;
        let mut config = crate::config::Config::default();
        config.refresh.mine_secs = 1;
        let (mut ctx, mut harness) = testkit::ctx_with(&server.uri(), config, store);
        ctx.mode = WorkerMode::Pinned(key(1550));
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
        let worker = tokio::spawn(run(ctx));

        assert!(matches!(harness.recv().await, Msg::Rate(_)));
        match harness.recv().await {
            Msg::PrRefreshed(pr) => assert_eq!(pr.key, key(1550)),
            other => panic!("{other:?}"),
        }
        assert_eq!(operations(&server).await, ["PrByNumber"]);

        harness
            .interest
            .send(testkit::interest(
                Some(testkit::pr_ref(1550, "PR_1", "a")),
                Vec::new(),
            ))
            .unwrap();
        assert!(matches!(harness.recv().await, Msg::Rate(_)));
        assert!(matches!(harness.recv().await, Msg::PrRefreshed(_)));
        assert!(matches!(harness.recv().await, Msg::PrRefreshed(_)));
        assert_eq!(operations(&server).await, ["PrByNumber", "PrByIds"]);

        harness.sync.send(SyncReq::Full(Scope::Mine)).await.unwrap();
        assert!(harness.quiet_for(Duration::from_millis(300)).await);
        assert!(searches(&server).await.is_empty(), "no list sync");
        worker.abort();
        assert_eq!(cached(&path), Vec::<u64>::new(), "no pull request rows");
    }

    #[test]
    fn intervals_double_while_the_budget_is_tight() {
        let (ctx, _harness) = testkit::ctx("http://127.0.0.1:1");
        let mut syncer = Syncer::new(ctx);
        assert_eq!(syncer.interval(60), Duration::from_secs(60));
        syncer.budget.observe(RateInfo {
            remaining: 30,
            reset_at: Utc::now() + TimeDelta::seconds(3600),
            last_cost: 1,
        });
        assert_eq!(syncer.interval(60), Duration::from_secs(120));
    }

    #[test]
    fn a_rate_limit_pauses_the_timed_syncs() {
        let (ctx, _harness) = testkit::ctx("http://127.0.0.1:1");
        let mut syncer = Syncer::new(ctx);
        syncer.pause_on_limit(&GithubError::Auth);
        assert_eq!(syncer.paused_until, None);
        syncer.pause_on_limit(&GithubError::RateLimited {
            until: Utc::now() + TimeDelta::seconds(60),
        });
        let wait = syncer.paused_until.unwrap() - Instant::now();
        assert!(wait > Duration::from_secs(55), "{wait:?}");
    }

    #[test]
    fn delta_search_strings() {
        let repos = ["a/b".to_owned()];
        let since = "2026-10-30T11:58:00Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(
            search(Scope::Mine, "me", &repos, Some(since)),
            "is:pr author:me repo:a/b updated:>=2026-10-30T11:58:00Z"
        );
        assert_eq!(
            search(Scope::Team, "me", &repos, Some(since)),
            "is:pr -author:me repo:a/b updated:>=2026-10-30T11:58:00Z"
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
