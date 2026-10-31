//! The startup worker: reads the login and the repo of the current
//! directory, then sends `Msg::StartupDone`.
//!
//! It runs `viewer { login }` unless the configuration sets `login`. The
//! cwd discovery runs at the same time. A failed `viewer` query sends
//! `SyncFailed` and runs again after a wait that doubles up to 60 s,
//! because no sync can start without the login.

use std::time::Duration;

use futures::FutureExt;

use crate::bus::Msg;
use crate::config::discover;
use crate::domain::repo::Repo;
use crate::workers::ctx::WorkerCtx;
use crate::workers::supervisor::Worker;
use crate::workers::sync_error;

/// The startup worker.
pub(crate) const WORKER: Worker = Worker {
    name: "startup",
    run: |ctx| run(ctx).boxed(),
};

/// The wait before the first retry of the `viewer` query.
const FIRST_RETRY: Duration = Duration::from_secs(1);

/// The longest wait between two `viewer` queries.
const MAX_RETRY: Duration = Duration::from_secs(60);

/// Reads the login and the cwd repo and sends `StartupDone`.
async fn run(ctx: WorkerCtx) {
    let (login, cwd_repo) = tokio::join!(login(&ctx), cwd_repo());
    let Some(login) = login else {
        return;
    };
    tracing::info!(%login, cwd_repo = ?cwd_repo.as_ref().map(|repo| &repo.key), "startup done");
    let _ = ctx.tx.send(Msg::StartupDone { login, cwd_repo }).await;
}

/// The login from the configuration, else from the `viewer` query. It
/// retries until the query succeeds. `None` when `app` is gone.
async fn login(ctx: &WorkerCtx) -> Option<String> {
    if let Some(login) = &ctx.config.login {
        return Some(login.clone());
    }
    let mut delay = FIRST_RETRY;
    loop {
        match ctx.client.viewer().await {
            Ok((login, _)) => return Some(login),
            Err(err) => {
                tracing::warn!(%err, ?delay, "viewer query failed, retrying");
                let failed = Msg::SyncFailed {
                    source: "sync",
                    scope: None,
                    err: sync_error(&err),
                };
                ctx.tx.send(failed).await.ok()?;
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(MAX_RETRY);
            }
        }
    }
}

/// The repo of the current directory, if it has a GitHub origin.
async fn cwd_repo() -> Option<Repo> {
    match std::env::current_dir() {
        Ok(dir) => discover::cwd_repo(&dir).await,
        Err(err) => {
            tracing::warn!(%err, "cannot read the current directory");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::bus::SyncError;
    use crate::config::Config;
    use crate::workers::testkit;

    const VIEWER: &str = include_str!("../github/testdata/viewer.json");

    async fn viewer_ok(server: &MockServer) {
        Mock::given(method("POST"))
            .and(body_string_contains("query Viewer"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(VIEWER.as_bytes(), "application/json"),
            )
            .mount(server)
            .await;
    }

    fn login_of(msg: Msg) -> String {
        match msg {
            Msg::StartupDone { login, .. } => login,
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn sends_the_viewer_login() {
        let server = MockServer::start().await;
        viewer_ok(&server).await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        run(ctx).await;
        assert_eq!(login_of(harness.recv().await), "chaosteil");
    }

    #[tokio::test]
    async fn the_configured_login_skips_the_query() {
        let server = MockServer::start().await;
        let config = Config {
            login: Some("someone".into()),
            ..Config::default()
        };
        let (ctx, mut harness) = testkit::ctx_with(&server.uri(), config, testkit::memory_store());
        run(ctx).await;
        assert_eq!(login_of(harness.recv().await), "someone");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failed_query_reports_and_retries() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(502))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        viewer_ok(&server).await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        let worker = tokio::spawn(run(ctx));
        match harness.recv().await {
            Msg::SyncFailed {
                source: "sync",
                scope: None,
                err: SyncError::Other(_),
            } => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(login_of(harness.recv().await), "chaosteil");
        worker.await.unwrap();
    }
}
