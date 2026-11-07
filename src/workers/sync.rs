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
//! A full sync searches the open pull requests of the repo set. A delta
//! searches all states, updated since `last_delta` minus 2 min, so merges
//! and closes arrive. `last_delta` advances to the start of the delta only
//! after its last page.
//!
//! A sync has two phases. Phase 1 reads the stubs of the search: the
//! light data of each pull request (`id`, `number`, repo, `state`,
//! `updatedAt`, `headRefOid`), at most 100 per page, until no page is
//! left or the stubs reach 500. The worker compares each stub with the
//! version that it knows of the pull request (`updated_at`, `head_oid`,
//! `observed_at`). Phase 2 reads the full fields of the new and the
//! changed pull requests by node id, at most 10 per request. A full sync also reads again at most 40
//! pull requests whose known version is older than 1 h, oldest first, so
//! the merge state and old cache rows stay up to date. At its first sync
//! of a scope, the worker loads the known versions from the cache.
//!
//! The worker sends `SyncStarted` before the first page of a sync. It
//! sends each phase 2 batch in a `PrsSynced` when the batch arrives, and
//! one more `PrsSynced` with `last` at the end of the sync. Only a full
//! sync that read the last page sends `seen`: the keys of every stub. It
//! also deletes the `gone` rows from the cache: the open pull requests
//! that the worker knows and that the result does not hold. The worker
//! sends the `Msg` first and writes the store second.
//!
//! GitHub stops a GraphQL request after about 10 s, with a 502 or a 504.
//! After such a timeout, the worker halves the page size of phase 1 (to
//! at least 10) or the batch size of phase 2 (to at least 5), and sends
//! the same request again. A timeout at the smallest size stops the sync
//! with that error. The smaller size stays for the next syncs of the
//! scope until the worker restarts.
//!
//! When the first page fails, the worker sends only `SyncFailed`. When a
//! later request fails, the sync is incomplete: the worker sends the end
//! of the sync, writes the sync state, and then sends `SyncFailed`. A full
//! sync sets `last_delta` when it has none, also when the sync is
//! incomplete. Only a complete full sync sets `last_full`.

use std::collections::{HashMap, HashSet};
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
use crate::github::PrStubPage;
use crate::github::budget::RateBudget;
use crate::github::parse::Me;
use crate::store::PrVersion;
use crate::workers::ctx::{WorkerCtx, WorkerMode};
use crate::workers::supervisor::Worker;
use crate::workers::{sync_error, wait_for_login};

/// The sync worker.
pub(crate) const WORKER: Worker = Worker {
    name: "sync",
    run: |ctx| run(ctx).boxed(),
};

/// The most stubs of one search.
const MAX_RESULTS: usize = 500;

/// The stubs per page of phase 1 at the start.
const PAGE_CAP: u32 = 100;

/// The smallest page size of phase 1 after timeouts.
const MIN_PAGE_CAP: u32 = 10;

/// The pull requests per request of phase 2 at the start. 20 took more
/// than 5 s for `rust-lang/rust`; 10 took less than 5 s.
const BATCH_CAP: usize = 10;

/// The smallest batch size of phase 2 after timeouts.
const MIN_BATCH_CAP: usize = 5;

/// The age of a known version after which a full sync reads the pull
/// request again, so the merge state and old cache rows stay up to date.
const REFETCH_AFTER: TimeDelta = TimeDelta::hours(1);

/// The most pull requests that one full sync reads again because of their
/// age.
const REFETCH_PER_SYNC: usize = 40;

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
#[derive(Debug)]
struct ScopeState {
    /// The versions of the open pull requests that the worker knows in
    /// this scope.
    known: HashMap<PrKey, PrVersion>,
    /// True after the worker loaded the cached versions into `known`.
    seeded: bool,
    /// The page size of phase 1. A timeout halves it.
    page_cap: u32,
    /// The batch size of phase 2. A timeout halves it.
    batch_cap: usize,
}

impl Default for ScopeState {
    fn default() -> Self {
        Self {
            known: HashMap::new(),
            seeded: false,
            page_cap: PAGE_CAP,
            batch_cap: BATCH_CAP,
        }
    }
}

impl ScopeState {
    /// Records the pull requests of a sync or a refresh: the versions of
    /// open ones are known, others are not.
    fn note(&mut self, prs: &[PullRequest]) {
        for pr in prs {
            if pr.head.value.state == PrState::Open {
                let version = PrVersion {
                    updated_at: pr.head.value.updated_at,
                    head_oid: pr.head.value.head_oid.clone(),
                    observed_at: pr.head.observed_at,
                };
                self.known.insert(pr.key.clone(), version);
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

    /// Loads the cached versions of `scope` into `known` before the first
    /// sync of the scope. A version that the worker knows already stays.
    async fn seed(&mut self, scope: Scope) {
        if self.state(scope).seeded {
            return;
        }
        match self.ctx.store.pr_versions(scope).await {
            Ok(versions) => {
                let state = self.state(scope);
                for (key, version) in versions {
                    state.known.entry(key).or_insert(version);
                }
                state.seeded = true;
            }
            Err(err) => tracing::warn!(?scope, %err, "cannot read the cached versions"),
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
        self.seed(scope).await;
        let query = search(scope, &login, &repos, None);
        let mut pages = match self.pages(scope, &query, &login, true).await {
            Ok(Some(pages)) => pages,
            Ok(None) => return false,
            Err(err) => return self.failed(scope, &repos, &err).await,
        };
        tracing::info!(
            ?scope,
            count = pages.keys.len(),
            fetched = pages.prs.len(),
            complete = pages.complete,
            "full sync"
        );
        let keys: HashSet<PrKey> = pages.keys.iter().cloned().collect();
        let state = self.state(scope);
        let gone = pages.complete.then(|| {
            let mut gone: Vec<PrKey> = state
                .known
                .keys()
                .filter(|key| !keys.contains(*key))
                .cloned()
                .collect();
            gone.sort();
            gone
        });
        let seen = pages.complete.then(|| {
            let mut seen: Vec<PrKey> = keys.iter().cloned().collect();
            seen.sort();
            seen
        });
        if pages.complete {
            state.known.retain(|key, _| keys.contains(key));
        }
        let started_at = pages.started_at;
        if !self.end(scope, seen, started_at).await {
            return false;
        }
        self.persist_full(scope, &repos, gone, started_at).await;
        match pages.failure.take() {
            Some(err) => self.failed(scope, &repos, &err).await,
            None => true,
        }
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
        self.seed(scope).await;
        let query = search(scope, &login, &repos, Some(last_delta - DELTA_OVERLAP));
        let mut pages = match self.pages(scope, &query, &login, false).await {
            Ok(Some(pages)) => pages,
            Ok(None) => return false,
            Err(err) => return self.failed(scope, &repos, &err).await,
        };
        tracing::debug!(
            ?scope,
            count = pages.keys.len(),
            fetched = pages.prs.len(),
            complete = pages.complete,
            "delta sync"
        );
        if !self.end(scope, None, pages.started_at).await {
            return false;
        }
        if let Some(err) = pages.failure.take() {
            return self.failed(scope, &repos, &err).await;
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

    /// Runs the two phases of the search `query`. Phase 1 reads the stubs,
    /// `page_cap` per page, until no page is left or the stubs reach
    /// `MAX_RESULTS`. After each page, phase 2 reads the new and the
    /// changed pull requests of the page. In a full sync (`refetch`),
    /// phase 2 at the end also reads the unchanged pull requests whose
    /// known version is older than `REFETCH_AFTER`: at most
    /// `REFETCH_PER_SYNC`, oldest first.
    ///
    /// It sends `SyncStarted` first. `Ok(None)` when `app` is gone. An
    /// error of the first page is `Err`. A later error stops the search:
    /// the result is incomplete and holds the error in `failure`.
    async fn pages(
        &mut self,
        scope: Scope,
        query: &str,
        login: &str,
        refetch: bool,
    ) -> Result<Option<Pages>, GithubError> {
        let teams = self.ctx.config.teams.clone();
        let me = Me {
            login,
            teams: &teams,
        };
        if self.ctx.tx.send(Msg::SyncStarted { scope }).await.is_err() {
            return Ok(None);
        }
        let mut page = self.list_page(scope, query, None).await?;
        let started_at = page.started_at;
        let mut pages = Pages {
            prs: Vec::new(),
            keys: Vec::new(),
            complete: true,
            started_at,
            failure: None,
        };
        let stale_before = started_at - REFETCH_AFTER;
        let mut stale: Vec<(DateTime<Utc>, String)> = Vec::new();
        loop {
            if !self.rate(page.rate.clone()).await {
                return Ok(None);
            }
            let known = &self.state(scope).known;
            let mut changed = Vec::new();
            for stub in &page.stubs {
                match known.get(&stub.key) {
                    Some(version)
                        if version.updated_at == stub.updated_at
                            && version.head_oid == stub.head_oid =>
                    {
                        if refetch && version.observed_at < stale_before {
                            stale.push((version.observed_at, stub.node_id.clone()));
                        }
                    }
                    _ => changed.push(stub.node_id.clone()),
                }
            }
            pages
                .keys
                .extend(page.stubs.iter().map(|stub| stub.key.clone()));
            match self.fetch(scope, &changed, &me, &mut pages).await {
                Ok(true) => {}
                Ok(false) => return Ok(None),
                Err(err) => return Ok(Some(pages.failed(err))),
            }
            let Some(after) = page.next.take() else {
                break;
            };
            if pages.keys.len() >= MAX_RESULTS {
                pages.complete = false;
                break;
            }
            page = match self.list_page(scope, query, Some(after.as_str())).await {
                Ok(page) => page,
                Err(err) => return Ok(Some(pages.failed(err))),
            };
        }
        stale.sort();
        let stale: Vec<String> = stale
            .into_iter()
            .take(REFETCH_PER_SYNC)
            .map(|(_, id)| id)
            .collect();
        match self.fetch(scope, &stale, &me, &mut pages).await {
            Ok(true) => Ok(Some(pages)),
            Ok(false) => Ok(None),
            Err(err) => Ok(Some(pages.failed(err))),
        }
    }

    /// Phase 1: reads one page of the search `query` after the cursor
    /// `after`, `page_cap` stubs per page. After a timeout, it halves
    /// `page_cap` and sends the same request again. A timeout at
    /// `MIN_PAGE_CAP` is an error.
    async fn list_page(
        &mut self,
        scope: Scope,
        query: &str,
        after: Option<&str>,
    ) -> Result<PrStubPage, GithubError> {
        loop {
            let cap = self.state(scope).page_cap;
            match self.ctx.client.list_prs(query, cap, after).await {
                Err(err) if err.is_timeout() && cap > MIN_PAGE_CAP => {
                    let cap = (cap / 2).max(MIN_PAGE_CAP);
                    self.state(scope).page_cap = cap;
                    tracing::warn!(?scope, page_cap = cap, %err, "GraphQL timeout, smaller pages");
                }
                result => return result,
            }
        }
    }

    /// Phase 2: reads the pull requests `ids` by node id, `batch_cap` per
    /// request. After a timeout, it halves `batch_cap` and sends the same
    /// ids again. A timeout at `MIN_BATCH_CAP` is an error. After each
    /// batch, it sends `Rate` and a `PrsSynced` with the pull requests,
    /// writes them into the cache, records their versions, and adds them
    /// to `pages`. `Ok(false)` when `app` is gone.
    async fn fetch(
        &mut self,
        scope: Scope,
        ids: &[String],
        me: &Me<'_>,
        pages: &mut Pages,
    ) -> Result<bool, GithubError> {
        let mut rest = ids;
        while !rest.is_empty() {
            let cap = self.state(scope).batch_cap;
            let (batch, after) = rest.split_at(cap.min(rest.len()));
            let (prs, rate) = match self.ctx.client.pr_by_ids(batch, me).await {
                Ok(ok) => ok,
                Err(err) if err.is_timeout() && cap > MIN_BATCH_CAP => {
                    let cap = (cap / 2).max(MIN_BATCH_CAP);
                    self.state(scope).batch_cap = cap;
                    tracing::warn!(?scope, batch_cap = cap, %err, "GraphQL timeout, smaller batches");
                    continue;
                }
                Err(err) => return Err(err),
            };
            rest = after;
            if !self.rate(rate).await {
                return Ok(false);
            }
            let msg = Msg::PrsSynced {
                scope,
                prs: prs.clone(),
                seen: None,
                started_at: pages.started_at,
                last: false,
            };
            if self.ctx.tx.send(msg).await.is_err() {
                return Ok(false);
            }
            if let Err(err) = self.ctx.store.put_prs(scope, prs.clone()).await {
                tracing::warn!(?scope, %err, "cannot cache the pull requests");
            }
            self.state(scope).note(&prs);
            pages.prs.extend(prs);
        }
        Ok(true)
    }

    /// Sends the `PrsSynced` that ends the sync of `scope`. False when
    /// `app` is gone.
    async fn end(&self, scope: Scope, seen: Option<Vec<PrKey>>, started_at: DateTime<Utc>) -> bool {
        let msg = Msg::PrsSynced {
            scope,
            prs: Vec::new(),
            seen,
            started_at,
            last: true,
        };
        self.ctx.tx.send(msg).await.is_ok()
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

    /// Writes the result of a full sync: the sync state, and after a
    /// complete sync (`gone` is set) the deleted gone candidates. The pull
    /// requests are in the cache already.
    async fn persist_full(
        &self,
        scope: Scope,
        repos: &[String],
        gone: Option<Vec<PrKey>>,
        started_at: DateTime<Utc>,
    ) {
        let store = &self.ctx.store;
        let repo_set = repo_set_hash(repos);
        let mut state = match store.sync_state(scope, repo_set.clone()).await {
            Ok(state) => state.unwrap_or_default(),
            Err(err) => {
                tracing::warn!(?scope, %err, "cannot read the sync state");
                return;
            }
        };
        // The next delta starts at the first full sync, also when that
        // sync is incomplete.
        state.last_delta = state.last_delta.or(Some(started_at));
        if let Some(gone) = gone {
            if let Err(err) = store.drop_gone(gone, started_at).await {
                tracing::warn!(?scope, %err, "cannot drop the gone pull requests");
            }
            state.last_full = Some(started_at);
            state.last_error = None;
        }
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

/// The result of one search.
#[derive(Debug)]
struct Pages {
    /// The pull requests that phase 2 read.
    prs: Vec<PullRequest>,
    /// The keys of every stub of the search.
    keys: Vec<PrKey>,
    /// True when the worker read the last page, false when `MAX_RESULTS`
    /// or a failure stopped it.
    complete: bool,
    /// The start time of the first page.
    started_at: DateTime<Utc>,
    /// The error of the request that stopped the search after one or more
    /// pages.
    failure: Option<GithubError>,
}

impl Pages {
    /// The incomplete result after the error `err`.
    fn failed(mut self, err: GithubError) -> Self {
        self.complete = false;
        self.failure = Some(err);
        self
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
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;
    use crate::bus::{Session, SyncError};
    use crate::domain::keys::RepoKey;
    use crate::workers::testkit::{self, Harness};

    const PAGE1: &str = include_str!("../github/testdata/list_prs_page1.json");
    const PAGE2: &str = include_str!("../github/testdata/list_prs_page2.json");

    /// The node ids, the `updatedAt` and the `headRefOid` of the stubs of
    /// page 1.
    const ID_1589: &str = "PR_kwDOAAAABc8AAAAAAAAA08";
    const ID_1587: &str = "PR_kwDOAAAABc8AAAAAAAAA07";
    const ID_1586: &str = "PR_kwDOAAAABc8AAAAAAAAA06";
    const UPDATED_1589: &str = "2026-10-31T02:20:36Z";
    const OID_1589: &str = "00000000000000000000000000000000c0de0006";
    const UPDATED_1587: &str = "2026-10-30T23:09:37Z";

    /// A `rateLimit` value for the test responses.
    fn rate_json() -> Value {
        json!({ "cost": 1, "remaining": 4990, "resetAt": "2026-10-31T05:50:43Z" })
    }

    /// Answers a `PrByIds` request: the nodes whose `id` is in the `ids`
    /// variable, in the order of the ids. An unknown id gets a null node.
    struct ByIds(Vec<Value>);

    impl Respond for ByIds {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let nodes: Vec<Value> = body["variables"]["ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| {
                    self.0
                        .iter()
                        .find(|node| node["id"] == *id)
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect();
            ResponseTemplate::new(200)
                .set_body_json(json!({ "data": { "nodes": nodes, "rateLimit": rate_json() } }))
        }
    }

    /// The nodes of the page fixtures.
    fn page_nodes() -> Vec<Value> {
        [PAGE1, PAGE2]
            .iter()
            .flat_map(|page| {
                let page: Value = serde_json::from_str(page).unwrap();
                page["data"]["search"]["nodes"].as_array().unwrap().clone()
            })
            .collect()
    }

    /// Answers the `PrByIds` requests with `nodes`.
    async fn serve_by_ids(server: &MockServer, nodes: Vec<Value>) {
        Mock::given(method("POST"))
            .and(body_string_contains("query PrByIds"))
            .respond_with(ByIds(nodes))
            .mount(server)
            .await;
    }

    /// Serves `first` for the first search request and `next` for every
    /// cursor.
    async fn serve_searches(server: &MockServer, first: Value, next: Value) {
        Mock::given(method("POST"))
            .and(body_string_contains("\"after\":\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(next))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("\"after\":null"))
            .respond_with(ResponseTemplate::new(200).set_body_json(first))
            .mount(server)
            .await;
    }

    /// Serves page 1 for the first request and page 2 for every cursor,
    /// and the nodes of both pages to `PrByIds`. `last` makes page 2 the
    /// last page.
    async fn serve_pages(server: &MockServer, last: bool) {
        let mut page2: Value = serde_json::from_str(PAGE2).unwrap();
        page2["data"]["search"]["pageInfo"]["hasNextPage"] = json!(!last);
        serve_searches(server, serde_json::from_str(PAGE1).unwrap(), page2).await;
        serve_by_ids(server, page_nodes()).await;
    }

    /// Serves page 1 as the only page, and its nodes to `PrByIds`.
    async fn serve_page1(server: &MockServer) {
        let mut page1: Value = serde_json::from_str(PAGE1).unwrap();
        page1["data"]["search"]["pageInfo"]["hasNextPage"] = json!(false);
        serve_searches(server, page1.clone(), page1).await;
        serve_by_ids(server, page_nodes()).await;
    }

    /// The `updatedAt` and the `headRefOid` of the stubs of `stub_page`.
    const STUB_UPDATED: &str = "2026-10-30T00:00:00Z";
    const STUB_OID: &str = "oid";

    /// A search page with `count` open stubs of `acme/deploy`, numbers
    /// 1000 and up. `next` gives it a next page.
    fn stub_page(count: u64, next: bool) -> Value {
        let nodes: Vec<Value> = (1000..1000 + count)
            .map(|number| {
                json!({
                    "__typename": "PullRequest",
                    "id": format!("PR_{number}"),
                    "number": number,
                    "repository": { "nameWithOwner": "acme/deploy" },
                    "state": "OPEN",
                    "updatedAt": STUB_UPDATED,
                    "headRefOid": STUB_OID,
                })
            })
            .collect();
        json!({
            "data": {
                "search": {
                    "pageInfo": {
                        "hasNextPage": next,
                        "endCursor": if next { json!("next") } else { Value::Null },
                    },
                    "nodes": nodes,
                },
                "rateLimit": rate_json(),
            }
        })
    }

    /// Records the versions of the stubs of `stub_page(count, _)` in
    /// `state`, observed now, so phase 2 reads none of them.
    fn know_stubs(state: &mut ScopeState, count: u64) {
        for number in 1000..1000 + count {
            state
                .known
                .insert(key(number), version(STUB_UPDATED, STUB_OID, Utc::now()));
        }
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

    /// The messages of the next sync until its end: `SyncStarted`, the
    /// `Rate` of each response, the numbers of each phase 2 batch, then
    /// the scope and the `seen` numbers of the end.
    async fn sync_pages(harness: &mut Harness) -> (Scope, Vec<Vec<u64>>, Option<Vec<u64>>) {
        let started = match harness.recv().await {
            Msg::SyncStarted { scope } => scope,
            other => panic!("{other:?}"),
        };
        let mut batches = Vec::new();
        loop {
            match harness.recv().await {
                Msg::Rate(_) => {}
                Msg::PrsSynced {
                    prs, last: false, ..
                } => batches.push(prs.iter().map(|pr| pr.key.number).collect()),
                Msg::PrsSynced {
                    scope,
                    prs,
                    seen,
                    last: true,
                    ..
                } => {
                    assert_eq!(prs, Vec::new(), "the end has no pull requests");
                    assert_eq!(scope, started);
                    let seen = seen.map(|keys| keys.iter().map(|key| key.number).collect());
                    return (scope, batches, seen);
                }
                other => panic!("{other:?}"),
            }
        }
    }

    /// The scope, the numbers of every batch and the `seen` numbers of the
    /// next sync.
    async fn synced(harness: &mut Harness) -> (Scope, Vec<u64>, Option<Vec<u64>>) {
        let (scope, pages, seen) = sync_pages(harness).await;
        (scope, pages.concat(), seen)
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

    /// The `ids` variable of each `PrByIds` request to `server`.
    async fn by_ids(server: &MockServer) -> Vec<Vec<String>> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter_map(|request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let ids = body["variables"]["ids"].as_array()?;
                Some(
                    ids.iter()
                        .map(|id| id.as_str().unwrap().to_owned())
                        .collect(),
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

    fn version(updated_at: &str, head_oid: &str, observed_at: DateTime<Utc>) -> PrVersion {
        PrVersion {
            updated_at: updated_at.parse().unwrap(),
            head_oid: head_oid.to_owned(),
            observed_at,
        }
    }

    /// The sync state of `scope` for the repo `acme/deploy`.
    async fn sync_state(
        store: &crate::store::handle::StoreHandle,
        scope: Scope,
    ) -> crate::store::SyncState {
        store
            .sync_state(scope, repo_set_hash(&["acme/deploy".to_owned()]))
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn a_full_sync_sends_each_batch_and_then_the_seen_keys() {
        let server = MockServer::start().await;
        serve_pages(&server, true).await;
        let (ctx, mut harness) = logged_in(&server);
        let store = ctx.store.clone();
        let mut syncer = Syncer::new(ctx);
        syncer
            .mine
            .known
            .insert(key(1), version(UPDATED_1589, OID_1589, Utc::now()));
        syncer
            .mine
            .known
            .insert(key(1589), version(UPDATED_1589, "old", Utc::now()));

        assert!(syncer.full(Scope::Mine).await);
        assert_eq!(
            sync_pages(&mut harness).await,
            (
                Scope::Mine,
                vec![vec![1589, 1587, 1586], vec![1574, 1573, 1572]],
                Some(vec![1572, 1573, 1574, 1586, 1587, 1589])
            )
        );
        assert!(
            syncer.mine.known.is_empty(),
            "#1 is gone, and merged pull requests are not known"
        );
        assert_eq!(
            searches(&server).await,
            [
                (
                    "is:pr is:open author:chaosteil repo:acme/deploy".to_owned(),
                    100
                ),
                (
                    "is:pr is:open author:chaosteil repo:acme/deploy".to_owned(),
                    100
                ),
            ]
        );
        assert_eq!(by_ids(&server).await.len(), 2);

        let state = sync_state(&store, Scope::Mine).await;
        assert!(state.last_full.is_some());
        assert_eq!(state.last_delta, state.last_full);
        assert_eq!(state.last_error, None);
    }

    #[tokio::test]
    async fn a_full_sync_cut_at_max_results_sets_only_last_delta() {
        let server = MockServer::start().await;
        serve_searches(&server, stub_page(100, true), stub_page(100, true)).await;
        serve_by_ids(&server, Vec::new()).await;
        let (ctx, mut harness) = logged_in(&server);
        let store = ctx.store.clone();
        let mut syncer = Syncer::new(ctx);
        know_stubs(&mut syncer.team, 100);
        syncer
            .team
            .known
            .insert(key(1), version(STUB_UPDATED, STUB_OID, Utc::now()));

        assert!(syncer.full(Scope::Team).await);
        let (scope, batches, seen) = sync_pages(&mut harness).await;
        assert_eq!(scope, Scope::Team);
        assert_eq!(batches, Vec::<Vec<u64>>::new());
        assert_eq!(seen, None);
        assert_eq!(searches(&server).await.len(), MAX_RESULTS / 100);
        assert_eq!(by_ids(&server).await, Vec::<Vec<String>>::new());
        assert!(syncer.team.known.contains_key(&key(1)), "#1 is not dropped");
        assert!(searches(&server).await[0].0.contains(" -author:chaosteil "));

        let state = sync_state(&store, Scope::Team).await;
        assert!(state.last_delta.is_some());
        assert_eq!(state.last_full, None);
    }

    #[tokio::test]
    async fn a_failed_second_page_ends_the_sync_and_then_fails() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("\"after\":\""))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("\"after\":null"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(PAGE1.as_bytes(), "application/json"),
            )
            .mount(&server)
            .await;
        serve_by_ids(&server, page_nodes()).await;
        let (ctx, mut harness) = logged_in(&server);
        let store = ctx.store.clone();
        let mut syncer = Syncer::new(ctx);

        assert!(syncer.full(Scope::Team).await);
        assert_eq!(
            sync_pages(&mut harness).await,
            (Scope::Team, vec![vec![1589, 1587, 1586]], None)
        );
        match harness.recv().await {
            Msg::SyncFailed {
                scope: Some(Scope::Team),
                ..
            } => {}
            other => panic!("{other:?}"),
        }
        let state = sync_state(&store, Scope::Team).await;
        assert!(state.last_delta.is_some());
        assert_eq!(state.last_full, None);
        assert!(state.last_error.is_some());
    }

    #[tokio::test]
    async fn phase_2_reads_only_the_new_and_the_changed_pull_requests() {
        let server = MockServer::start().await;
        serve_page1(&server).await;
        let (ctx, mut harness) = logged_in(&server);
        let mut syncer = Syncer::new(ctx);
        let unchanged = version(UPDATED_1589, OID_1589, Utc::now());
        syncer.mine.known.insert(key(1589), unchanged.clone());
        syncer
            .mine
            .known
            .insert(key(1587), version(UPDATED_1587, "old", Utc::now()));

        assert!(syncer.full(Scope::Mine).await);
        assert_eq!(
            sync_pages(&mut harness).await,
            (
                Scope::Mine,
                vec![vec![1587, 1586]],
                Some(vec![1586, 1587, 1589])
            )
        );
        assert_eq!(by_ids(&server).await, [[ID_1587, ID_1586]]);
        assert_eq!(syncer.mine.known.get(&key(1589)), Some(&unchanged));
    }

    #[tokio::test]
    async fn a_full_sync_reads_an_old_version_again_and_a_delta_does_not() {
        let old = Utc::now() - REFETCH_AFTER - TimeDelta::minutes(5);
        let server = MockServer::start().await;
        serve_page1(&server).await;
        let (ctx, mut harness) = logged_in(&server);
        let mut syncer = Syncer::new(ctx);
        syncer
            .mine
            .known
            .insert(key(1589), version(UPDATED_1589, OID_1589, old));
        assert!(syncer.full(Scope::Mine).await);
        assert_eq!(
            synced(&mut harness).await.1,
            [1587, 1586, 1589],
            "the old version comes last"
        );
        assert_eq!(
            by_ids(&server).await,
            [vec![ID_1587, ID_1586], vec![ID_1589]]
        );

        let server = MockServer::start().await;
        serve_page1(&server).await;
        let (ctx, mut harness) = logged_in(&server);
        let last = "2026-10-30T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        ctx.store
            .put_sync_state(
                Scope::Mine,
                repo_set_hash(&["acme/deploy".to_owned()]),
                crate::store::SyncState {
                    last_full: Some(last),
                    last_delta: Some(last),
                    last_error: None,
                },
            )
            .await
            .unwrap();
        let mut syncer = Syncer::new(ctx);
        syncer
            .mine
            .known
            .insert(key(1589), version(UPDATED_1589, OID_1589, old));
        assert!(syncer.delta(Scope::Mine).await);
        assert_eq!(synced(&mut harness).await.1, [1587, 1586]);
        assert_eq!(by_ids(&server).await, [[ID_1587, ID_1586]]);
    }

    #[tokio::test]
    async fn the_first_sync_loads_the_known_versions_from_the_cache() {
        let server = MockServer::start().await;
        serve_page1(&server).await;
        let (ctx, mut harness) = logged_in(&server);
        let me = Me {
            login: "chaosteil",
            teams: &[],
        };
        let mut cached = crate::github::parse::pr_by_ids::prs(
            crate::github::parse::fixture(include_str!("../github/testdata/pr_by_ids_lists.json")),
            &me,
            Utc::now(),
        )
        .unwrap()
        .remove(0);
        assert_eq!(cached.key, key(1589));
        cached.head.value.state = PrState::Open;
        ctx.store.put_prs(Scope::Mine, vec![cached]).await.unwrap();
        let mut syncer = Syncer::new(ctx);

        assert!(syncer.full(Scope::Mine).await);
        assert_eq!(synced(&mut harness).await.1, [1587, 1586]);
        assert!(syncer.mine.seeded);
        assert!(syncer.mine.known.contains_key(&key(1589)));
        assert_eq!(by_ids(&server).await, [[ID_1587, ID_1586]]);
    }

    #[tokio::test]
    async fn a_timeout_halves_the_page_size_for_this_and_the_next_syncs() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("\"after\":null"))
            .respond_with(ResponseTemplate::new(502))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        serve_page1(&server).await;
        let (ctx, mut harness) = logged_in(&server);
        let mut syncer = Syncer::new(ctx);

        assert!(syncer.full(Scope::Mine).await);
        assert_eq!(synced(&mut harness).await.1, [1589, 1587, 1586]);
        assert!(syncer.full(Scope::Mine).await);
        synced(&mut harness).await;
        let firsts: Vec<u64> = searches(&server)
            .await
            .into_iter()
            .map(|(_, first)| first)
            .collect();
        assert_eq!(firsts, [100, 50, 50]);
        assert_eq!(syncer.mine.page_cap, 50);
    }

    #[tokio::test]
    async fn a_timeout_of_a_batch_sends_the_same_ids_again_in_two_halves() {
        let server = MockServer::start().await;
        serve_searches(&server, stub_page(10, false), stub_page(10, false)).await;
        let page1: Value = serde_json::from_str(PAGE1).unwrap();
        let template = &page1["data"]["search"]["nodes"][0];
        let nodes: Vec<Value> = (1000..1010)
            .map(|number| {
                let mut node = template.clone();
                node["id"] = json!(format!("PR_{number}"));
                node["number"] = json!(number);
                node["state"] = json!("OPEN");
                node
            })
            .collect();
        Mock::given(method("POST"))
            .and(body_string_contains("query PrByIds"))
            .respond_with(ResponseTemplate::new(504))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        serve_by_ids(&server, nodes).await;
        let (ctx, mut harness) = logged_in(&server);
        let mut syncer = Syncer::new(ctx);

        assert!(syncer.full(Scope::Team).await);
        assert_eq!(
            sync_pages(&mut harness).await.1,
            [
                vec![1000, 1001, 1002, 1003, 1004],
                vec![1005, 1006, 1007, 1008, 1009]
            ]
        );
        let ids: Vec<String> = (1000..1010).map(|number| format!("PR_{number}")).collect();
        assert_eq!(
            by_ids(&server).await,
            [ids.clone(), ids[..5].to_vec(), ids[5..].to_vec()]
        );
        assert_eq!(syncer.team.batch_cap, MIN_BATCH_CAP);
    }

    #[tokio::test]
    async fn a_timeout_at_the_smallest_page_size_fails_the_sync() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(502))
            .mount(&server)
            .await;
        let (ctx, mut harness) = logged_in(&server);
        let mut syncer = Syncer::new(ctx);
        syncer.mine.page_cap = MIN_PAGE_CAP;

        assert!(syncer.full(Scope::Mine).await);
        assert_eq!(
            harness.recv().await,
            Msg::SyncStarted { scope: Scope::Mine }
        );
        match harness.recv().await {
            Msg::SyncFailed {
                scope: Some(Scope::Mine),
                ..
            } => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(
            searches(&server).await,
            [(
                "is:pr is:open author:chaosteil repo:acme/deploy".to_owned(),
                u64::from(MIN_PAGE_CAP)
            )]
        );
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
            Msg::SyncStarted { scope: Scope::Mine }
        );
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

    /// Serves the `pr_by_ids.json` fixture to the `PrByIds` requests for
    /// the ids `PR_1` and `PR_2` of the refresh tests.
    async fn serve_refresh(server: &MockServer) {
        Mock::given(method("POST"))
            .and(body_string_contains("query PrByIds"))
            .and(body_string_contains("\"PR_1\""))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(BY_IDS.as_bytes(), "application/json"),
            )
            .with_priority(1)
            .mount(server)
            .await;
    }

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
        syncer
            .mine
            .known
            .insert(key(1589), version(UPDATED_1589, "old", Utc::now()));

        assert!(syncer.delta(Scope::Mine).await);
        assert_eq!(
            synced(&mut harness).await,
            (Scope::Mine, vec![1589, 1587, 1586, 1574, 1573, 1572], None)
        );
        assert_eq!(
            searches(&server).await[0],
            (
                "is:pr author:chaosteil repo:acme/deploy updated:>=2026-10-30T11:58:00Z".to_owned(),
                100
            )
        );
        assert!(
            syncer.mine.known.is_empty(),
            "merged pull requests are not known"
        );
        let state = store
            .sync_state(Scope::Mine, repo_set)
            .await
            .unwrap()
            .unwrap();
        assert!(state.last_delta.unwrap() > last);
        assert_eq!(state.last_full, Some(last));
    }

    #[tokio::test]
    async fn a_delta_cut_at_max_results_keeps_last_delta() {
        let server = MockServer::start().await;
        serve_searches(&server, stub_page(100, true), stub_page(100, true)).await;
        serve_by_ids(&server, Vec::new()).await;
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
        know_stubs(&mut syncer.team, 100);
        assert!(syncer.delta(Scope::Team).await);
        assert_eq!(synced(&mut harness).await.2, None);
        assert_eq!(searches(&server).await.len(), MAX_RESULTS / 100);
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
        serve_refresh(&server).await;
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

        let (scope, _, seen) = synced(&mut harness).await;
        assert_eq!((scope, seen), (Scope::Mine, None));
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
        let (scope, mut numbers, seen) = synced(&mut harness).await;
        assert_eq!(scope, Scope::Team);
        numbers.sort_unstable();
        assert_eq!(seen, Some(numbers));
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
        assert!(syncer.mine.known.contains_key(&key(1550)));
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
        serve_refresh(&server).await;
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
