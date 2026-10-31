//! The CI worker: polls the CI state of the visible pull requests and the
//! checks of the selected one.
//!
//! Every `refresh.ci_secs` (45 s by default) it sends one `ci_poll`
//! request with the node ids of `Interest.visible` and
//! `Interest.selected`. It sends `CiUpdated`, or `SyncFailed` with the
//! source `ci`. Check runs do not change `updatedAt`, so the list sync
//! does not see them; this poll does.

use std::collections::HashMap;
use std::time::Duration;

use futures::FutureExt;
use tokio::time::MissedTickBehavior;

use crate::bus::{Interest, Msg, PrRef};
use crate::domain::keys::PrKey;
use crate::workers::ctx::WorkerCtx;
use crate::workers::supervisor::Worker;
use crate::workers::sync_error;

/// The CI worker.
pub(crate) const WORKER: Worker = Worker {
    name: "ci",
    run: |ctx| run(ctx).boxed(),
};

/// Polls on every tick of the interval. It stops when `app` is gone.
async fn run(ctx: WorkerCtx) {
    let period = Duration::from_secs(ctx.config.refresh.ci_secs.max(1));
    let mut ticker = tokio::time::interval(period);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let interest = ctx.interest.borrow().clone();
        if let Some(msg) = poll(&ctx, &interest).await
            && ctx.tx.send(msg).await.is_err()
        {
            return;
        }
    }
}

/// One poll for `interest`. Returns the message for `app`, or `None` when
/// nothing is visible or selected. It sends `Rate` after a response.
async fn poll(ctx: &WorkerCtx, interest: &Interest) -> Option<Msg> {
    if interest.visible.is_empty() && interest.selected.is_none() {
        return None;
    }
    let mut keys: HashMap<&str, &PrRef> = HashMap::new();
    let mut visible = Vec::new();
    for pr in &interest.visible {
        if keys.insert(pr.node_id.as_str(), pr).is_none() {
            visible.push(pr.node_id.clone());
        }
    }
    let selected = interest.selected.as_ref();
    if let Some(pr) = selected {
        keys.insert(pr.node_id.as_str(), pr);
    }
    let result = ctx
        .client
        .ci_poll(&visible, selected.map(|pr| pr.node_id.as_str()))
        .await;
    let (result, rate) = match result {
        Ok(ok) => ok,
        Err(err) => {
            tracing::warn!(%err, "ci poll failed");
            return Some(Msg::SyncFailed {
                source: "ci",
                scope: None,
                err: sync_error(&err),
            });
        }
    };
    let _ = ctx.tx.send(Msg::Rate(rate)).await;

    let key_of = |node_id: &str| -> Option<PrKey> { keys.get(node_id).map(|pr| pr.key.clone()) };
    let mut states = Vec::new();
    for status in &result.visible {
        if let Some(key) = key_of(&status.node_id) {
            states.push((key, status.head_oid.clone(), status.state.clone()));
        }
    }
    let mut selected_checks = None;
    if let Some((status, checks)) = result.selected
        && let Some(key) = key_of(&status.node_id)
    {
        if !states.iter().any(|(known, _, _)| *known == key) {
            states.push((key.clone(), status.head_oid, status.state));
        }
        selected_checks = Some((key, checks));
    }
    Some(Msg::CiUpdated {
        states,
        selected_checks,
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::bus::SyncError;
    use crate::config::Config;
    use crate::domain::pr::CiState;
    use crate::workers::testkit;

    const CI_POLL: &str = include_str!("../github/testdata/ci_poll.json");
    const GREEN: &str = "PR_kwDOO_HURc8AAAABHbg2Aw";
    const RED: &str = "PR_kwDOO_HURc8AAAABHRcLbw";
    const RED_OID: &str = "8b54d4ec035d4dfb2f940468d2dce5b58a666fd1";

    fn fast_config() -> Config {
        let mut config = Config::default();
        config.refresh.ci_secs = 1;
        config
    }

    async fn serve(server: &MockServer, template: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(body_string_contains("query CiPoll"))
            .respond_with(template)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn sends_states_and_selected_checks() {
        let server = MockServer::start().await;
        serve(
            &server,
            ResponseTemplate::new(200).set_body_raw(CI_POLL.as_bytes(), "application/json"),
        )
        .await;
        let (ctx, mut harness) =
            testkit::ctx_with(&server.uri(), fast_config(), testkit::memory_store());
        let green = testkit::pr_ref(1, GREEN, "a");
        let red = testkit::pr_ref(2, RED, "b");
        harness
            .interest
            .send(testkit::interest(
                Some(red.clone()),
                vec![green.clone(), red.clone()],
            ))
            .unwrap();
        let worker = tokio::spawn(run(ctx));

        assert!(matches!(harness.recv().await, Msg::Rate(_)));
        let Msg::CiUpdated {
            states,
            selected_checks,
        } = harness.recv().await
        else {
            panic!("expected CiUpdated");
        };
        worker.abort();
        let summary: Vec<(u64, String, CiState)> = states
            .into_iter()
            .map(|(key, oid, state)| (key.number, oid, state.value))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    1,
                    "1e2defa1ebcdb1cb5232604e4ba6000c00d2d0e2".to_owned(),
                    CiState::Success
                ),
                (2, RED_OID.to_owned(), CiState::Failure),
            ]
        );
        let (key, checks) = selected_checks.unwrap();
        assert_eq!(key, red.key);
        assert_eq!(checks.value.len(), 19);

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            body["variables"],
            json!({ "visible": [GREEN, RED], "selected": RED, "withSelected": true })
        );
    }

    #[tokio::test]
    async fn nothing_visible_sends_no_request() {
        let server = MockServer::start().await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        assert_eq!(poll(&ctx, &testkit::interest(None, Vec::new())).await, None);
        assert!(harness.quiet_for(Duration::from_millis(50)).await);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failed_poll_is_a_ci_sync_failure() {
        let server = MockServer::start().await;
        serve(&server, ResponseTemplate::new(500)).await;
        let (ctx, _harness) = testkit::ctx(&server.uri());
        let interest = testkit::interest(None, vec![testkit::pr_ref(1, GREEN, "a")]);
        assert!(matches!(
            poll(&ctx, &interest).await,
            Some(Msg::SyncFailed {
                source: "ci",
                scope: None,
                err: SyncError::Other(_),
            })
        ));
    }

    #[tokio::test]
    async fn no_network_is_offline() {
        let (ctx, _harness) = testkit::ctx("http://127.0.0.1:1");
        let interest = testkit::interest(None, vec![testkit::pr_ref(1, GREEN, "a")]);
        assert_eq!(
            poll(&ctx, &interest).await,
            Some(Msg::SyncFailed {
                source: "ci",
                scope: None,
                err: SyncError::Offline,
            })
        );
    }
}
