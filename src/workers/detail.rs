//! The detail worker: loads the conversation and the changed files of the
//! selected and the visible pull requests.
//!
//! It reacts to the first `Interest` and to its changes, after 250 ms
//! without a change. A new `selected` loads at once. The `visible` pull
//! requests load in the background, at most 2 at a time. A load reads the
//! conversation and the files at the same time.
//!
//! The conversation load reads the `pr_details` cache first: a row with
//! the same head oid as the `PrRef` and an age under 5 min is sent as it
//! is. Else the worker sends `DetailLoading`, runs `pr_detail`, sends
//! `DetailLoaded` and then writes the row. A failure sends
//! `DetailLoaded(key, Err(..))`.
//!
//! The files load does the same with the `pr_files` cache, an age under
//! 30 min, `FilesLoading`, `pr_files` and `FilesLoaded`. The diff changes
//! only with the head or with a move of the base, so the longer age is
//! safe. A load is done only when both parts succeed, so a failed part
//! loads again at the next `Interest` change.
//!
//! The worker also serves `DetailReq::Reload`. A reload reads the
//! conversation from GitHub at once, without the cache and without the
//! debounce, and replaces the cache row. It does not read the files. A
//! reload of a pull request whose reload runs now does nothing: the
//! running reload reads GitHub after the request was made.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use futures::FutureExt;
use tokio::task::{self, JoinError, JoinSet};

use crate::bus::{DetailReq, Msg, PrRef, SyncError};
use crate::domain::detail::PrDetail;
use crate::domain::files::PrFiles;
use crate::domain::keys::PrKey;
use crate::domain::observed::Observed;
use crate::workers::ctx::WorkerCtx;
use crate::workers::supervisor::Worker;
use crate::workers::{settle, sync_error};

/// The detail worker.
pub(crate) const WORKER: Worker = Worker {
    name: "detail",
    run: |ctx| run(ctx).boxed(),
};

/// The quiet time after an `Interest` change before the worker acts.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// The most background loads at one time.
const MAX_CONCURRENT: usize = 2;

/// The age under which a cached detail with the same head oid is used.
const MAX_AGE: TimeDelta = TimeDelta::minutes(5);

/// The age under which cached files with the same head oid are used.
const FILES_MAX_AGE: TimeDelta = TimeDelta::minutes(30);

/// The key and head oid of a successful load, or `None`.
type Loaded = Option<(PrKey, String)>;

/// Serves `Interest` changes and reload requests until `app` is gone.
async fn run(ctx: WorkerCtx) {
    let detail_rx = Arc::clone(&ctx.detail_rx);
    let mut requests = detail_rx.lock().await;
    let mut interest = ctx.interest.clone();
    // The first interest is the value of the watch, not a change. The
    // pinned mode can keep it for its whole life, so it loads too.
    interest.mark_changed();
    // The head oid of the last successful load of each pull request.
    let mut loaded: HashMap<PrKey, String> = HashMap::new();
    let mut queue: VecDeque<PrRef> = VecDeque::new();
    let mut background: JoinSet<Loaded> = JoinSet::new();
    let mut selected_loads: JoinSet<Loaded> = JoinSet::new();
    let mut last_selected: Option<PrRef> = None;
    let mut reloads: JoinSet<()> = JoinSet::new();
    // The pull request of each running reload, by task id.
    let mut reloading: HashMap<task::Id, PrKey> = HashMap::new();
    loop {
        while background.len() < MAX_CONCURRENT {
            let Some(pr) = queue.pop_front() else {
                break;
            };
            if loaded.get(&pr.key) != Some(&pr.head_oid) {
                background.spawn(load(ctx.clone(), pr));
            }
        }
        tokio::select! {
            changed = interest.changed() => {
                if changed.is_err() || !settle(&mut interest, DEBOUNCE).await {
                    return;
                }
                let current = interest.borrow_and_update().clone();
                if current.selected != last_selected {
                    if let Some(pr) = &current.selected {
                        selected_loads.spawn(load(ctx.clone(), pr.clone()));
                    }
                    last_selected = current.selected;
                }
                queue = current
                    .visible
                    .into_iter()
                    .filter(|pr| {
                        Some(pr) != last_selected.as_ref()
                            && loaded.get(&pr.key) != Some(&pr.head_oid)
                    })
                    .collect();
            }
            request = requests.recv() => {
                let Some(DetailReq::Reload(refs)) = request else {
                    return;
                };
                for pr in refs {
                    if reloading.values().any(|key| *key == pr.key) {
                        continue;
                    }
                    let key = pr.key.clone();
                    let reload = fetch_conversation(ctx.clone(), pr).map(drop);
                    reloading.insert(reloads.spawn(reload).id(), key);
                }
            }
            Some(done) = background.join_next() => record(&mut loaded, done),
            Some(done) = selected_loads.join_next() => record(&mut loaded, done),
            Some(done) = reloads.join_next_with_id() => {
                let id = match done {
                    Ok((id, ())) => id,
                    Err(err) => {
                        tracing::warn!(%err, "detail reload task failed");
                        err.id()
                    }
                };
                reloading.remove(&id);
            }
        }
    }
}

/// Records the result of a finished load task.
fn record(loaded: &mut HashMap<PrKey, String>, done: Result<Loaded, JoinError>) {
    match done {
        Ok(Some((key, head_oid))) => {
            loaded.insert(key, head_oid);
        }
        Ok(None) => {}
        Err(err) => tracing::warn!(%err, "detail load task failed"),
    }
}

/// Loads the conversation and the files of `pr` at the same time.
/// Returns the key and head oid only when both succeed.
async fn load(ctx: WorkerCtx, pr: PrRef) -> Loaded {
    let (conversation, files) = tokio::join!(
        load_conversation(ctx.clone(), pr.clone()),
        load_files(ctx, pr)
    );
    conversation.and(files)
}

/// Loads the detail of `pr` from the cache or from GitHub and sends
/// `DetailLoaded`. Returns the key and head oid on success.
async fn load_conversation(ctx: WorkerCtx, pr: PrRef) -> Loaded {
    if let Some(detail) = cached(&ctx, &pr).await {
        let _ = ctx
            .tx
            .send(Msg::DetailLoaded(pr.key.clone(), Ok(detail)))
            .await;
        return Some((pr.key, pr.head_oid));
    }
    fetch_conversation(ctx, pr).await
}

/// Reads the detail of `pr` from GitHub, sends `DetailLoading` and
/// `DetailLoaded`, and writes the cache row. Returns the key and head oid
/// on success.
async fn fetch_conversation(ctx: WorkerCtx, pr: PrRef) -> Loaded {
    if ctx
        .tx
        .send(Msg::DetailLoading(pr.key.clone()))
        .await
        .is_err()
    {
        return None;
    }
    let (result, rate) = match ctx.client.pr_detail(&pr.key).await {
        Ok((Some(detail), rate)) => (Ok(detail), Some(rate)),
        Ok((None, rate)) => (
            Err(SyncError::Other("pull request not found".to_owned())),
            Some(rate),
        ),
        Err(err) => {
            tracing::warn!(key = ?pr.key, %err, "pr detail failed");
            (Err(sync_error(&err)), None)
        }
    };
    let ok = result.is_ok();
    let _ = ctx
        .tx
        .send(Msg::DetailLoaded(pr.key.clone(), result.clone()))
        .await;
    if let Some(rate) = rate {
        let _ = ctx.tx.send(Msg::Rate(rate)).await;
    }
    if let Ok(detail) = result
        && let Err(err) = ctx.store.put_detail(pr.key.clone(), detail).await
    {
        tracing::warn!(key = ?pr.key, %err, "cannot cache the pr detail");
    }
    ok.then_some((pr.key, pr.head_oid))
}

/// The cached detail of `pr` when its head oid equals the head oid of
/// `pr` and it is younger than `MAX_AGE`.
async fn cached(ctx: &WorkerCtx, pr: &PrRef) -> Option<Observed<PrDetail>> {
    let detail = match ctx.store.detail(pr.key.clone()).await {
        Ok(detail) => detail?,
        Err(err) => {
            tracing::warn!(key = ?pr.key, %err, "cannot read the pr detail cache");
            return None;
        }
    };
    let fresh = Utc::now() - detail.observed_at < MAX_AGE;
    (fresh && detail.value.head_oid == pr.head_oid).then_some(detail)
}

/// Loads the changed files of `pr` from the cache or from GitHub and
/// sends `FilesLoaded`. Returns the key and head oid on success.
async fn load_files(ctx: WorkerCtx, pr: PrRef) -> Loaded {
    if let Some(files) = cached_files(&ctx, &pr).await {
        let _ = ctx
            .tx
            .send(Msg::FilesLoaded(pr.key.clone(), Ok(files)))
            .await;
        return Some((pr.key, pr.head_oid));
    }
    if ctx
        .tx
        .send(Msg::FilesLoading(pr.key.clone()))
        .await
        .is_err()
    {
        return None;
    }
    let observed_at = Utc::now();
    let result = match ctx.client.pr_files(&pr.key, &pr.head_oid).await {
        Ok(files) => Ok(Observed {
            value: files,
            observed_at,
        }),
        Err(err) => {
            tracing::warn!(key = ?pr.key, %err, "pr files failed");
            Err(sync_error(&err))
        }
    };
    let ok = result.is_ok();
    let _ = ctx
        .tx
        .send(Msg::FilesLoaded(pr.key.clone(), result.clone()))
        .await;
    if let Ok(files) = result
        && let Err(err) = ctx.store.put_files(pr.key.clone(), files).await
    {
        tracing::warn!(key = ?pr.key, %err, "cannot cache the pr files");
    }
    ok.then_some((pr.key, pr.head_oid))
}

/// The cached files of `pr` when their head oid equals the head oid of
/// `pr` and they are younger than `FILES_MAX_AGE`.
async fn cached_files(ctx: &WorkerCtx, pr: &PrRef) -> Option<Observed<PrFiles>> {
    let files = match ctx.store.files(pr.key.clone()).await {
        Ok(files) => files?,
        Err(err) => {
            tracing::warn!(key = ?pr.key, %err, "cannot read the pr files cache");
            return None;
        }
    };
    let fresh = Utc::now() - files.observed_at < FILES_MAX_AGE;
    (fresh && files.value.head_oid == pr.head_oid).then_some(files)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use pretty_assertions::assert_eq;
    use serde_json::Value;
    use wiremock::matchers::{body_string_contains, method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::workers::testkit::{self, Harness};

    const PR_DETAIL: &str = include_str!("../github/testdata/pr_detail.json");
    const PR_FILES: &str = include_str!("../github/testdata/pr_files.json");
    const FIXTURE_OID: &str = "1e2defa1ebcdb1cb5232604e4ba6000c00d2d0e2";

    /// The path of the files endpoint of any pull request.
    const FILES_PATH: &str = r"^/repos/[^/]+/[^/]+/pulls/\d+/files$";

    async fn serve(server: &MockServer, delay: Duration) {
        Mock::given(method("POST"))
            .and(body_string_contains("query PrDetail"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(PR_DETAIL.as_bytes(), "application/json")
                    .set_delay(delay),
            )
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(FILES_PATH))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(PR_FILES.as_bytes(), "application/json"),
            )
            .mount(server)
            .await;
    }

    /// The number and the success of each `DetailLoaded` until no message
    /// came for `quiet`.
    async fn details(harness: &mut Harness, quiet: Duration) -> Vec<(u64, bool)> {
        let mut out = Vec::new();
        while let Ok(Some(msg)) = tokio::time::timeout(quiet, harness.rx.recv()).await {
            if let Msg::DetailLoaded(key, result) = msg {
                out.push((key.number, result.is_ok()));
            }
        }
        out
    }

    /// The pull request numbers in the GraphQL requests to `server`.
    async fn requested(server: &MockServer) -> Vec<u64> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path() == "/graphql")
            .map(|request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                body["variables"]["number"].as_u64().unwrap()
            })
            .collect()
    }

    fn detail(head_oid: &str, age: TimeDelta) -> Observed<PrDetail> {
        Observed {
            value: PrDetail {
                comments: Vec::new(),
                reviews: Vec::new(),
                threads: Vec::new(),
                head_oid: head_oid.to_owned(),
            },
            observed_at: Utc::now() - age,
        }
    }

    fn files(head_oid: &str, age: TimeDelta) -> Observed<PrFiles> {
        Observed {
            value: PrFiles {
                files: Vec::new(),
                truncated: false,
                head_oid: head_oid.to_owned(),
            },
            observed_at: Utc::now() - age,
        }
    }

    #[tokio::test]
    async fn quick_selection_changes_load_only_the_last() {
        let server = MockServer::start().await;
        serve(&server, Duration::ZERO).await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        let worker = tokio::spawn(run(ctx));
        for number in 1..=3 {
            let pr = testkit::pr_ref(number, &format!("PR_{number}"), "h");
            harness
                .interest
                .send(testkit::interest(Some(pr), Vec::new()))
                .unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            details(&mut harness, Duration::from_millis(600)).await,
            [(3, true)]
        );
        assert_eq!(requested(&server).await, [3]);
        worker.abort();
    }

    #[tokio::test]
    async fn a_fresh_cache_row_with_the_same_head_is_sent_without_a_request() {
        let server = MockServer::start().await;
        serve(&server, Duration::ZERO).await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        let hit = testkit::pr_ref(1, "PR_1", "abc");
        let moved = testkit::pr_ref(2, "PR_2", "new");
        let old = testkit::pr_ref(3, "PR_3", "abc");
        let cached_hit = detail("abc", TimeDelta::seconds(10));
        let store = &ctx.store;
        store
            .put_detail(hit.key.clone(), cached_hit.clone())
            .await
            .unwrap();
        store
            .put_detail(moved.key.clone(), detail("old", TimeDelta::seconds(10)))
            .await
            .unwrap();
        store
            .put_detail(old.key.clone(), detail("abc", TimeDelta::minutes(6)))
            .await
            .unwrap();

        assert!(load_conversation(ctx.clone(), hit.clone()).await.is_some());
        assert_eq!(
            harness.recv().await,
            Msg::DetailLoaded(hit.key.clone(), Ok(cached_hit))
        );
        assert_eq!(requested(&server).await, Vec::<u64>::new());

        assert!(load_conversation(ctx.clone(), moved).await.is_some());
        assert!(load_conversation(ctx.clone(), old.clone()).await.is_some());
        assert_eq!(requested(&server).await, [2, 3]);

        let cached = ctx.store.detail(old.key).await.unwrap().unwrap();
        assert_eq!(cached.value.head_oid, FIXTURE_OID);
        assert!(Utc::now() - cached.observed_at < TimeDelta::seconds(10));
    }

    #[tokio::test]
    async fn a_reload_skips_a_fresh_cache_row_and_replaces_it() {
        let server = MockServer::start().await;
        serve(&server, Duration::ZERO).await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        let pr = testkit::pr_ref(1, "PR_1", FIXTURE_OID);
        ctx.store
            .put_detail(pr.key.clone(), detail(FIXTURE_OID, TimeDelta::seconds(10)))
            .await
            .unwrap();
        let store = ctx.store.clone();
        let worker = tokio::spawn(run(ctx));

        harness
            .detail
            .send(DetailReq::Reload(vec![pr.clone()]))
            .await
            .unwrap();
        assert_eq!(harness.recv().await, Msg::DetailLoading(pr.key.clone()));
        let Msg::DetailLoaded(key, Ok(loaded)) = harness.recv().await else {
            panic!("no loaded detail");
        };
        assert_eq!(key, pr.key);
        assert!(Utc::now() - loaded.observed_at < TimeDelta::seconds(5));
        assert_eq!(requested(&server).await, [1]);
        // A reload reads only the conversation.
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.url.path() == "/graphql")
        );

        let cached = store.detail(pr.key).await.unwrap().unwrap();
        assert_eq!(cached, loaded);
        worker.abort();
    }

    #[tokio::test]
    async fn a_reload_of_a_running_reload_does_nothing() {
        let server = MockServer::start().await;
        serve(&server, Duration::from_millis(300)).await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        let worker = tokio::spawn(run(ctx));
        let one = testkit::pr_ref(1, "PR_1", "h");
        let two = testkit::pr_ref(2, "PR_2", "h");

        harness
            .detail
            .send(DetailReq::Reload(vec![one.clone()]))
            .await
            .unwrap();
        harness
            .detail
            .send(DetailReq::Reload(vec![one.clone(), two]))
            .await
            .unwrap();
        let mut loaded = details(&mut harness, Duration::from_millis(600)).await;
        loaded.sort_unstable();
        assert_eq!(loaded, [(1, true), (2, true)]);

        // The first reload ended, so a new request reads GitHub again.
        harness
            .detail
            .send(DetailReq::Reload(vec![one]))
            .await
            .unwrap();
        assert_eq!(
            details(&mut harness, Duration::from_millis(600)).await,
            [(1, true)]
        );
        let mut numbers = requested(&server).await;
        numbers.sort_unstable();
        assert_eq!(numbers, [1, 1, 2]);
        worker.abort();
    }

    #[tokio::test]
    async fn visible_loads_run_two_at_a_time() {
        let server = MockServer::start().await;
        serve(&server, Duration::from_millis(400)).await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        let worker = tokio::spawn(run(ctx));
        let visible = (1..=4)
            .map(|number| testkit::pr_ref(number, &format!("PR_{number}"), "h"))
            .collect();
        let start = Instant::now();
        harness
            .interest
            .send(testkit::interest(None, visible))
            .unwrap();
        tokio::time::sleep(DEBOUNCE + Duration::from_millis(200)).await;
        assert_eq!(requested(&server).await.len(), 2);
        let mut loaded = details(&mut harness, Duration::from_millis(1000)).await;
        loaded.sort_unstable();
        assert_eq!(loaded, [(1, true), (2, true), (3, true), (4, true)]);
        assert!(start.elapsed() >= DEBOUNCE + Duration::from_millis(800));
        worker.abort();
    }

    #[tokio::test]
    async fn the_first_interest_loads_without_a_change() {
        let server = MockServer::start().await;
        serve(&server, Duration::ZERO).await;
        let (mut ctx, mut harness) = testkit::ctx(&server.uri());
        let selected = testkit::pr_ref(7, "PR_7", "h");
        harness
            .interest
            .send(testkit::interest(Some(selected), Vec::new()))
            .unwrap();
        // `run` makes the watch with the first interest: it is not a
        // change.
        ctx.interest.borrow_and_update();
        let worker = tokio::spawn(run(ctx));
        assert_eq!(
            details(&mut harness, Duration::from_millis(1000)).await,
            [(7, true)]
        );
        worker.abort();
    }

    #[tokio::test]
    async fn a_failed_load_is_an_error_result() {
        let (ctx, mut harness) = testkit::ctx("http://127.0.0.1:1");
        let pr = testkit::pr_ref(1, "PR_1", "h");
        assert_eq!(load_conversation(ctx, pr.clone()).await, None);
        assert_eq!(harness.recv().await, Msg::DetailLoading(pr.key.clone()));
        assert_eq!(
            harness.recv().await,
            Msg::DetailLoaded(pr.key, Err(SyncError::Offline))
        );
    }

    #[tokio::test]
    async fn a_fresh_files_cache_row_is_sent_without_a_request() {
        let server = MockServer::start().await;
        serve(&server, Duration::ZERO).await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        let hit = testkit::pr_ref(1, "PR_1", "abc");
        let cached_hit = files("abc", TimeDelta::minutes(20));
        ctx.store
            .put_files(hit.key.clone(), cached_hit.clone())
            .await
            .unwrap();

        assert!(load_files(ctx.clone(), hit.clone()).await.is_some());
        assert_eq!(
            harness.recv().await,
            Msg::FilesLoaded(hit.key, Ok(cached_hit))
        );
        assert!(server.received_requests().await.unwrap().is_empty());

        let old = testkit::pr_ref(2, "PR_2", "abc");
        ctx.store
            .put_files(old.key.clone(), files("abc", TimeDelta::minutes(31)))
            .await
            .unwrap();
        assert!(load_files(ctx.clone(), old.clone()).await.is_some());
        let cached = ctx.store.files(old.key).await.unwrap().unwrap();
        assert_eq!(cached.value.files.len(), 5);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_files_load_does_not_stop_the_conversation() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(FILES_PATH))
            .respond_with(ResponseTemplate::new(500))
            .with_priority(1)
            .mount(&server)
            .await;
        serve(&server, Duration::ZERO).await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        let pr = testkit::pr_ref(1, "PR_1", "h");

        assert_eq!(load(ctx, pr.clone()).await, None);

        let mut conversation = None;
        let mut files = None;
        while conversation.is_none() || files.is_none() {
            match harness.recv().await {
                Msg::DetailLoaded(key, result) => conversation = Some((key, result.is_ok())),
                Msg::FilesLoaded(key, result) => files = Some((key, result.is_err())),
                _ => {}
            }
        }
        assert_eq!(conversation, Some((pr.key.clone(), true)));
        assert_eq!(files, Some((pr.key, true)));
    }
}
