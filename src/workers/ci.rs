//! The CI worker: polls the CI state of the visible pull requests and the
//! checks of the selected one.
//!
//! Every `refresh.ci_secs` (45 s by default) it sends one `ci_poll`
//! request with the node ids of `Interest.visible` and
//! `Interest.selected`, and of at most `WATCH_PER_POLL` pull requests of
//! `Interest.ci_watch`. When `ci_watch` holds more, each poll takes the
//! next ones, so each one gets a turn. It sends `CiUpdated`, or `SyncFailed` with the
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

/// The maximum number of pull requests of `Interest.ci_watch` in one
/// poll.
const WATCH_PER_POLL: usize = 20;

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
    let mut offset = 0;
    loop {
        ticker.tick().await;
        let interest = ctx.interest.borrow().clone();
        let (watch, next) = watch_round(&interest.ci_watch, offset);
        offset = next;
        if let Some(msg) = poll(&ctx, &interest, &watch).await
            && ctx.tx.send(msg).await.is_err()
        {
            return;
        }
    }
}

/// The pull requests of `watch` for one poll, from the place `offset`,
/// and the place for the next poll. All of them when they are at most
/// `WATCH_PER_POLL`.
fn watch_round(watch: &[PrRef], offset: usize) -> (Vec<PrRef>, usize) {
    if watch.len() <= WATCH_PER_POLL {
        return (watch.to_vec(), 0);
    }
    let start = offset % watch.len();
    let round = watch
        .iter()
        .cycle()
        .skip(start)
        .take(WATCH_PER_POLL)
        .cloned()
        .collect();
    (round, (start + WATCH_PER_POLL) % watch.len())
}

/// One poll for `interest` and the watched pull requests `watch`. Returns
/// the message for `app`, or `None` when nothing is visible, selected or
/// watched. It sends `Rate` after a response.
async fn poll(ctx: &WorkerCtx, interest: &Interest, watch: &[PrRef]) -> Option<Msg> {
    if interest.visible.is_empty() && interest.selected.is_none() && watch.is_empty() {
        return None;
    }
    let mut keys: HashMap<&str, &PrRef> = HashMap::new();
    let mut visible = Vec::new();
    for pr in interest.visible.iter().chain(watch) {
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
    const GREEN: &str = "PR_kwDOAAAAAc8AAAAAAAAA02";
    const RED: &str = "PR_kwDOAAAAAc8AAAAAAAAA01";
    const RED_OID: &str = "00000000000000000000000000000000c0de0005";

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
                    "00000000000000000000000000000000c0de0003".to_owned(),
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
    async fn watched_pull_requests_are_polled_with_the_visible_ones() {
        let server = MockServer::start().await;
        serve(
            &server,
            ResponseTemplate::new(200).set_body_raw(CI_POLL.as_bytes(), "application/json"),
        )
        .await;
        let (ctx, _harness) = testkit::ctx(&server.uri());
        let green = testkit::pr_ref(1, GREEN, "a");
        let red = testkit::pr_ref(2, RED, "b");
        let interest = testkit::interest(None, vec![green.clone()]);
        let Some(Msg::CiUpdated { states, .. }) =
            poll(&ctx, &interest, &[green, red.clone()]).await
        else {
            panic!("expected CiUpdated");
        };
        assert!(
            states
                .iter()
                .any(|(key, _, ci)| *key == red.key && ci.value == CiState::Failure)
        );
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["variables"]["visible"], json!([GREEN, RED]));
    }

    #[tokio::test]
    async fn only_watched_pull_requests_send_a_request() {
        let server = MockServer::start().await;
        serve(
            &server,
            ResponseTemplate::new(200).set_body_raw(CI_POLL.as_bytes(), "application/json"),
        )
        .await;
        let (ctx, _harness) = testkit::ctx(&server.uri());
        let interest = testkit::interest(None, Vec::new());
        let watch = [testkit::pr_ref(2, RED, "b")];
        assert!(matches!(
            poll(&ctx, &interest, &watch).await,
            Some(Msg::CiUpdated { .. })
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[test]
    fn watch_round_takes_turns_and_is_bounded() {
        let watch: Vec<PrRef> = (0..45)
            .map(|n| testkit::pr_ref(n, &format!("PR_{n}"), "a"))
            .collect();
        let numbers = |prs: &[PrRef]| prs.iter().map(|pr| pr.key.number).collect::<Vec<_>>();
        let (first, next) = watch_round(&watch, 0);
        assert_eq!(numbers(&first), (0..20).collect::<Vec<_>>());
        let (second, next) = watch_round(&watch, next);
        assert_eq!(numbers(&second), (20..40).collect::<Vec<_>>());
        let (third, next) = watch_round(&watch, next);
        assert_eq!(
            numbers(&third),
            (40..45).chain(0..15).collect::<Vec<_>>(),
            "the list wraps"
        );
        assert_eq!(next, 15);

        let (few, next) = watch_round(&watch[..3], 7);
        assert_eq!(numbers(&few), [0, 1, 2]);
        assert_eq!(next, 0);
        assert_eq!(watch_round(&[], 3), (Vec::new(), 0));
    }

    #[tokio::test]
    async fn nothing_visible_sends_no_request() {
        let server = MockServer::start().await;
        let (ctx, mut harness) = testkit::ctx(&server.uri());
        assert_eq!(
            poll(&ctx, &testkit::interest(None, Vec::new()), &[]).await,
            None
        );
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
            poll(&ctx, &interest, &[]).await,
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
            poll(&ctx, &interest, &[]).await,
            Some(Msg::SyncFailed {
                source: "ci",
                scope: None,
                err: SyncError::Offline,
            })
        );
    }
}
