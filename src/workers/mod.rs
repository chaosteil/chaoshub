//! Background tasks. They call the adapters and send `bus::Msg`.
#![allow(dead_code, reason = "run spawns the workers in a later phase")]

mod ci;
pub(crate) mod ctx;
mod detail;
mod jj;
mod prefetch;
mod startup;
mod supervisor;
mod sync;
#[cfg(test)]
mod testkit;

use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::bus::{Session, SyncError};
use crate::github::GithubError;
use crate::workers::ctx::WorkerCtx;
use crate::workers::supervisor::{FIRST_BACKOFF, Worker, supervise};

/// The workers that `spawn_all` starts.
const WORKERS: &[Worker] = &[startup::WORKER];

/// Spawns every worker into one `JoinSet`, each one inside its own
/// supervisor. A drop of the set aborts the supervisors and the workers.
pub(crate) fn spawn_all(ctx: WorkerCtx) -> JoinSet<()> {
    let mut set = JoinSet::new();
    let contexts = std::iter::repeat_n(ctx, WORKERS.len());
    for (&worker, ctx) in WORKERS.iter().zip(contexts) {
        set.spawn(supervise(worker, ctx, FIRST_BACKOFF));
    }
    set
}

/// Maps a GitHub error to the error that `app` shows. A request without a
/// response means the network is down.
pub(crate) fn sync_error(err: &GithubError) -> SyncError {
    match err {
        GithubError::Auth => SyncError::Auth,
        GithubError::RateLimited { until } => SyncError::RateLimited { until: *until },
        GithubError::Http { status: None, .. } => SyncError::Offline,
        GithubError::Http { .. } | GithubError::Graphql(_) | GithubError::Parse(_) => {
            SyncError::Other(err.to_string())
        }
    }
}

/// Waits until the session has a login, and returns it. `app` sets the
/// login after `StartupDone`. `None` when `app` is gone.
pub(crate) async fn wait_for_login(session: &mut watch::Receiver<Session>) -> Option<String> {
    let session = session
        .wait_for(|session| session.login.is_some())
        .await
        .ok()?;
    session.login.clone()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn sync_error_cases() {
        let until = Utc::now();
        let http = |status| GithubError::Http {
            status,
            message: "bad".into(),
        };
        assert_eq!(sync_error(&GithubError::Auth), SyncError::Auth);
        assert_eq!(
            sync_error(&GithubError::RateLimited { until }),
            SyncError::RateLimited { until }
        );
        assert_eq!(sync_error(&http(None)), SyncError::Offline);
        assert!(
            matches!(sync_error(&http(Some(500))), SyncError::Other(text) if text.contains("500"))
        );
        assert!(matches!(
            sync_error(&GithubError::Graphql(vec!["boom".into()])),
            SyncError::Other(text) if text.contains("boom")
        ));
        assert!(matches!(
            sync_error(&GithubError::Parse("odd".into())),
            SyncError::Other(text) if text.contains("odd")
        ));
    }

    #[tokio::test]
    async fn wait_for_login_waits_for_the_session() {
        let (tx, mut rx) = watch::channel(Session {
            login: None,
            repos: Vec::new(),
        });
        let waiter = tokio::spawn(async move { wait_for_login(&mut rx).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());
        tx.send_modify(|session| session.login = Some("chaosteil".into()));
        assert_eq!(waiter.await.unwrap().as_deref(), Some("chaosteil"));
    }

    #[tokio::test]
    async fn wait_for_login_ends_without_a_sender() {
        let (tx, mut rx) = watch::channel(Session {
            login: None,
            repos: Vec::new(),
        });
        drop(tx);
        assert_eq!(wait_for_login(&mut rx).await, None);
    }
}
