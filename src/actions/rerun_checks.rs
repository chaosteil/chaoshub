//! The action that runs failed checks again.
//!
//! A failed check run of a GitHub Actions workflow has a workflow run id:
//! the action re-runs the failed jobs of each distinct run. A failed check
//! run without a workflow run has a check suite id: the action
//! re-requests each distinct suite. A status context has neither and
//! cannot run again from chaoshub.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Prepare, Services};
use crate::bus::{ActionInput, KeyChord, KeyCode, KeyModifiers, Selection, Target};
use crate::domain::check::{Check, CheckConclusion};

/// The action that re-runs the failed checks of a pull request.
pub(crate) struct RerunChecks;

/// What to run again: the distinct workflow runs and check suites of the
/// failed checks, in check order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Rerun {
    pub runs: Vec<u64>,
    pub suites: Vec<u64>,
}

impl Rerun {
    fn is_empty(&self) -> bool {
        self.runs.is_empty() && self.suites.is_empty()
    }
}

/// True when the check completed with a failure.
fn failed(check: &Check) -> bool {
    matches!(
        check.conclusion,
        Some(
            CheckConclusion::Failure
                | CheckConclusion::TimedOut
                | CheckConclusion::Cancelled
                | CheckConclusion::StartupFailure
        )
    )
}

/// The runs and suites of the failed checks: the workflow run of a check
/// when it has one, else its check suite.
pub(crate) fn rerun_plan(checks: &[Check]) -> Rerun {
    let mut plan = Rerun::default();
    for check in checks.iter().filter(|check| failed(check)) {
        match (check.run_id, check.suite_id) {
            (Some(run), _) => {
                if !plan.runs.contains(&run) {
                    plan.runs.push(run);
                }
            }
            (None, Some(suite)) => {
                if !plan.suites.contains(&suite) {
                    plan.suites.push(suite);
                }
            }
            (None, None) => {}
        }
    }
    plan
}

impl Action for RerunChecks {
    fn id(&self) -> &'static str {
        "rerun_checks"
    }

    fn label(&self) -> &'static str {
        "re-run failed checks"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('x'),
            mods: KeyModifiers::NONE,
        })
    }

    /// A pull request with failed checks that can run again.
    fn applies(&self, sel: &Selection) -> Applicability {
        let Target::Pr { pr, .. } = &sel.target else {
            return Applicability::Hidden;
        };
        let checks = &pr.checks.value;
        if !checks.iter().any(failed) {
            Applicability::Disabled("no failed checks")
        } else if rerun_plan(checks).is_empty() {
            Applicability::Disabled("the failed checks cannot run again")
        } else {
            Applicability::Enabled
        }
    }

    fn prepare(&self, _sel: &Selection) -> Prepare {
        Prepare::Run
    }

    /// `rerun-failed-jobs` for each workflow run, then `rerequest` for
    /// each check suite. The first failed request stops the action.
    fn execute(
        &self,
        sel: Selection,
        _input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let Target::Pr { pr, .. } = sel.target else {
                return Err("a local change has no checks".to_owned());
            };
            let plan = rerun_plan(&pr.checks.value);
            let repo = &pr.key.repo;
            for &run in &plan.runs {
                svc.client
                    .rerun_failed_jobs(repo, run)
                    .await
                    .map_err(|err| err.to_string())?;
            }
            for &suite in &plan.suites {
                svc.client
                    .rerequest_suite(repo, suite)
                    .await
                    .map_err(|err| err.to_string())?;
            }
            Ok(format!(
                "re-running the failed checks of #{}",
                pr.key.number
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::domain::check::{CheckKind, CheckStatus};
    use crate::domain::local::LocalMatch;
    use crate::domain::pr::PullRequest;
    use crate::domain::repo::Repo;
    use crate::github::{GithubClient, TokenSource};
    use crate::vcs::lock::RepoLocks;

    /// A completed check run with a conclusion, a run id and a suite id.
    fn check(
        name: &str,
        conclusion: CheckConclusion,
        run_id: Option<u64>,
        suite_id: Option<u64>,
    ) -> Check {
        Check {
            name: name.into(),
            kind: CheckKind::CheckRun,
            status: CheckStatus::Completed,
            conclusion: Some(conclusion),
            started: None,
            completed: None,
            url: None,
            run_id,
            suite_id,
        }
    }

    fn selection(checks: Vec<Check>) -> Selection {
        let mut pr: PullRequest = serde_json::from_str(include_str!("testdata/pr.json")).unwrap();
        pr.checks.value = checks;
        Selection {
            target: Target::Pr {
                repo: Repo {
                    key: pr.key.repo.clone(),
                    path: None,
                    settings: None,
                },
                pr,
                stack: Vec::new(),
                local: LocalMatch::None,
            },
            me: "chaosteil".into(),
            action_config: None,
        }
    }

    fn services(server: &MockServer) -> Services {
        let client = GithubClient::new(TokenSource::command("echo", &["tok"]), &server.uri());
        Services {
            client: Arc::new(client.unwrap()),
            locks: RepoLocks::default(),
            cancel: CancellationToken::new(),
            tx: mpsc::channel(8).0,
        }
    }

    #[test]
    fn the_plan_takes_distinct_runs_then_suites_of_failed_checks() {
        use CheckConclusion::{
            Cancelled, Failure, Neutral, Skipped, StartupFailure, Success, TimedOut,
        };
        let mut pending = check("pending", Failure, Some(9), Some(90));
        pending.conclusion = None;
        let mut status = check("ci/legacy", Failure, None, None);
        status.kind = CheckKind::StatusContext;
        let checks = [
            check("build", Failure, Some(11), Some(21)),
            check("test", TimedOut, Some(11), Some(21)),
            check("lint", Cancelled, Some(12), Some(22)),
            check("app", StartupFailure, None, Some(31)),
            check("app 2", Failure, None, Some(31)),
            check("ok", Success, Some(13), Some(23)),
            check("neutral", Neutral, Some(14), Some(24)),
            check("skipped", Skipped, Some(15), Some(25)),
            pending,
            status,
        ];
        assert_eq!(
            rerun_plan(&checks),
            Rerun {
                runs: vec![11, 12],
                suites: vec![31],
            }
        );
        assert_eq!(rerun_plan(&[]), Rerun::default());
    }

    #[test]
    fn applies_needs_a_failed_check_that_can_run_again() {
        let ok = check("ok", CheckConclusion::Success, Some(1), Some(2));
        assert_eq!(
            RerunChecks.applies(&selection(vec![ok])),
            Applicability::Disabled("no failed checks")
        );
        let status = check("ci/legacy", CheckConclusion::Failure, None, None);
        assert_eq!(
            RerunChecks.applies(&selection(vec![status])),
            Applicability::Disabled("the failed checks cannot run again")
        );
        let failed = check("build", CheckConclusion::Failure, Some(11), Some(21));
        let sel = selection(vec![failed]);
        assert_eq!(RerunChecks.applies(&sel), Applicability::Enabled);
        assert_eq!(RerunChecks.prepare(&sel), Prepare::Run);
    }

    #[tokio::test]
    async fn execute_reruns_the_runs_and_rerequests_the_suites() {
        let server = MockServer::start().await;
        for (route, expected) in [
            (
                "/repos/chaosteil/chaoshub/actions/runs/11/rerun-failed-jobs",
                1,
            ),
            (
                "/repos/chaosteil/chaoshub/actions/runs/12/rerun-failed-jobs",
                1,
            ),
            ("/repos/chaosteil/chaoshub/check-suites/31/rerequest", 1),
        ] {
            Mock::given(method("POST"))
                .and(path(route))
                .respond_with(ResponseTemplate::new(201))
                .expect(expected)
                .mount(&server)
                .await;
        }
        let checks = vec![
            check("build", CheckConclusion::Failure, Some(11), Some(21)),
            check("lint", CheckConclusion::Failure, Some(12), Some(22)),
            check("app", CheckConclusion::Failure, None, Some(31)),
        ];
        let result = RerunChecks
            .execute(selection(checks), ActionInput::None, services(&server))
            .await;
        assert_eq!(result, Ok("re-running the failed checks of #7".to_owned()));
        server.verify().await;
    }

    #[tokio::test]
    async fn a_failed_request_stops_with_its_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/repos/chaosteil/chaoshub/actions/runs/11/rerun-failed-jobs",
            ))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(serde_json::json!({ "message": "Must have admin rights" })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let checks = vec![
            check("build", CheckConclusion::Failure, Some(11), Some(21)),
            check("app", CheckConclusion::Failure, None, Some(31)),
        ];
        let result = RerunChecks
            .execute(selection(checks), ActionInput::None, services(&server))
            .await;
        let err = result.unwrap_err();
        assert!(err.contains("Must have admin rights"), "{err}");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "the suite was not re-requested");
    }
}
