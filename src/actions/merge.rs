//! The merge action.
//!
//! It merges a pull request through the async merge endpoint of GitHub.
//! For a pull request at position k of a stack, GitHub merges the
//! positions 1 to k atomically, so the blockers and the confirm lines
//! cover every entry up to k. The settings come from the
//! `[actions.merge]` table, which also gets the keys of `[merge]`:
//! `method` and `bypass_rules` (off by default) directly under the table
//! are the defaults, and a `repos."owner/name"` table overrides each key
//! that it sets for one repo.

use std::time::Duration;

use futures::future::BoxFuture;
use tokio::time::Instant;

use crate::bus::{
    ActionEvent, ActionInput, ConfirmSpec, KeyChord, KeyCode, KeyModifiers, MergeProgress, Msg,
    Selection, Target, TargetKey,
};
use crate::config::schema::{MergeConfig, RepoMerge, parse_repo_name};
use crate::domain::merge::{MergeMethod, merge_blockers, resolve_method, squash_message};
use crate::domain::pr::{CiState, PullRequest, ReviewDecision};
use crate::domain::repo::Repo;
use crate::github::rest::MergeArgs;
use crate::vcs::jj;

use super::{Action, Applicability, Prepare, Services};

/// The id of the merge action.
pub(crate) const ID: &str = "merge";

/// The merge action.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MergeAction;

impl MergeAction {
    /// Hidden for a local-only change. Disabled when the configuration
    /// table is invalid, when the stack entries up to the position are
    /// not loaded, or when an entry has a hard blocker.
    pub(crate) fn applies(sel: &Selection) -> Applicability {
        let Target::Pr { pr, stack, .. } = &sel.target else {
            return Applicability::Hidden;
        };
        if repo_merge(sel).is_err() {
            return Applicability::Disabled("invalid [actions.merge] configuration");
        }
        let Some(entries) = merged_entries(pr, stack) else {
            return Applicability::Disabled("stack not loaded");
        };
        if merge_blockers(&entries).hard.is_empty() {
            Applicability::Enabled
        } else {
            Applicability::Disabled(hard_reason(&entries))
        }
    }

    /// The confirm popup: every merged entry bottom first with its CI and
    /// review state, the merge method, the bypass flag and the warnings.
    pub(crate) fn prepare(sel: &Selection) -> Prepare {
        let Target::Pr {
            pr, stack, repo, ..
        } = &sel.target
        else {
            return Prepare::Run;
        };
        let entries = merged_entries(pr, stack).unwrap_or_else(|| vec![pr.clone()]);
        let settings = repo_merge(sel).unwrap_or_default();
        Prepare::Confirm(ConfirmSpec {
            title: format!("merge #{}?", pr.key.number),
            lines: confirm_lines(&entries, repo, &settings),
            danger: true,
        })
    }
}

impl Action for MergeAction {
    fn id(&self) -> &'static str {
        ID
    }

    fn label(&self) -> &'static str {
        "Merge"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('m'),
            mods: KeyModifiers::NONE,
        })
    }

    fn hint(&self) -> crate::actions::Hint {
        crate::actions::Hint::Primary
    }

    fn applies(&self, sel: &Selection) -> Applicability {
        Self::applies(sel)
    }

    fn prepare(&self, sel: &Selection) -> Prepare {
        Self::prepare(sel)
    }

    fn execute(
        &self,
        sel: Selection,
        _input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(merge(sel, svc, SCHEDULE))
    }
}

/// When the action polls the state of an async merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Schedule {
    /// The poll interval at the start.
    fast: Duration,
    /// How long the action polls at the `fast` interval.
    fast_for: Duration,
    /// The poll interval after `fast_for`.
    slow: Duration,
    /// The time after which the action stops the poll.
    limit: Duration,
}

/// Every 2 s for 30 s, then every 5 s, up to 5 min.
const SCHEDULE: Schedule = Schedule {
    fast: Duration::from_secs(2),
    fast_for: Duration::from_secs(30),
    slow: Duration::from_secs(5),
    limit: Duration::from_mins(5),
};

impl Schedule {
    /// The wait before the next poll, `elapsed` after the merge request.
    /// `None` when the poll must stop.
    fn delay(&self, elapsed: Duration) -> Option<Duration> {
        if elapsed >= self.limit {
            None
        } else if elapsed < self.fast_for {
            Some(self.fast)
        } else {
            Some(self.slow)
        }
    }
}

/// Requests the async merge, polls its state, and runs `jj git fetch`
/// after a merge. Every state goes to `app` as
/// `ActionEvent::MergeStatus`. A cancel stops the poll, not the merge on
/// GitHub.
async fn merge(sel: Selection, svc: Services, schedule: Schedule) -> Result<String, String> {
    let Target::Pr { pr, repo, .. } = &sel.target else {
        return Err("merge needs a pull request".to_owned());
    };
    let key = sel.target.key();
    let number = pr.key.number;
    let settings = repo_merge(&sel)?;
    let method = method(repo, &settings);
    let (title, body) = squash_message(pr);
    let args = MergeArgs {
        method: method.as_str(),
        commit_title: &title,
        commit_message: &body,
        sha: &pr.head.value.head_oid,
        bypass_rules: settings.bypass_rules,
    };
    let started = match svc.client.merge_async(&pr.key, &args).await {
        Ok(started) => started,
        Err(err) => return fail(&svc, &key, err.to_string()).await,
    };
    let mut progress = started.progress;
    send(&svc, &key, ActionEvent::MergeStatus(progress.clone())).await;
    let start = Instant::now();
    while progress == MergeProgress::Pending {
        let Some(uuid) = &started.uuid else {
            return fail(&svc, &key, "GitHub gave no merge id".to_owned()).await;
        };
        let Some(delay) = schedule.delay(start.elapsed()) else {
            let text = format!("merge of #{number} still pending, see GitHub");
            return fail(&svc, &key, text).await;
        };
        tokio::select! {
            () = svc.cancel.cancelled() => {
                return Err(format!("stopped the poll, the merge of #{number} goes on"));
            }
            () = tokio::time::sleep(delay) => {}
        }
        progress = match svc.client.merge_async_status(&pr.key, uuid).await {
            Ok(progress) => progress,
            Err(err) => return fail(&svc, &key, format!("merge state unknown: {err}")).await,
        };
        send(&svc, &key, ActionEvent::MergeStatus(progress.clone())).await;
    }
    match progress {
        MergeProgress::Merged { .. } => Ok(fetch(&svc, &key, repo, number).await),
        MergeProgress::Enqueued => Ok(format!("#{number} in merge queue")),
        MergeProgress::Failed(message) => Err(message),
        MergeProgress::Pending => Err(format!("merge of #{number} still pending")),
    }
}

/// Runs `jj git fetch` in the repo after a merge, under the repo lock,
/// and sends `Progress("fetched")`. A repo without a path has nothing to
/// fetch. A failed fetch does not fail the merge.
async fn fetch(svc: &Services, key: &TargetKey, repo: &Repo, number: u64) -> String {
    let Some(path) = &repo.path else {
        return format!("merged #{number}");
    };
    match jj::git_fetch(&svc.locks, path).await {
        Ok(()) => {
            send(svc, key, ActionEvent::Progress("fetched".to_owned())).await;
            format!("merged #{number}")
        }
        Err(err) => {
            tracing::warn!(pr = number, %err, "jj git fetch after the merge failed");
            format!("merged #{number}, jj git fetch failed: {err}")
        }
    }
}

/// Sends `MergeStatus(Failed)` and returns the failure.
async fn fail(svc: &Services, key: &TargetKey, text: String) -> Result<String, String> {
    let event = ActionEvent::MergeStatus(MergeProgress::Failed(text.clone()));
    send(svc, key, event).await;
    Err(text)
}

/// Sends an event of the merge action to `app`. A closed channel means
/// that `app` is gone, so the event has no reader.
async fn send(svc: &Services, key: &TargetKey, event: ActionEvent) {
    let msg = Msg::ActionEvent {
        id: ID,
        key: key.clone(),
        event,
    };
    if svc.tx.send(msg).await.is_err() {
        tracing::debug!("app is gone, merge event dropped");
    }
}

/// The merge settings of the repo of the target, from the
/// `[actions.merge]` table: `method` and `bypass_rules` directly under
/// the table, with the keys of the `repos."owner/name"` entry of the
/// repo on top. The defaults without a table. An error when the table
/// does not parse.
pub(crate) fn repo_merge(sel: &Selection) -> Result<RepoMerge, String> {
    let Some(table) = &sel.action_config else {
        return Ok(RepoMerge::default());
    };
    let text = |err: toml::de::Error| err.to_string();
    toml::Value::Table(table.clone())
        .try_into::<MergeConfig>()
        .map_err(text)?;
    let mut layered = table.clone();
    if let Some(toml::Value::Table(repos)) = layered.remove("repos") {
        let repo = sel.target.repo();
        let entry = repos
            .into_iter()
            .find(|(name, _)| parse_repo_name(name).as_ref() == Some(repo));
        if let Some((_, toml::Value::Table(entry))) = entry {
            layered.extend(entry);
        }
    }
    toml::Value::Table(layered).try_into().map_err(text)
}

/// The merge method of the repo: the override of the configuration, else
/// what the repo permits.
pub(crate) fn method(repo: &Repo, settings: &RepoMerge) -> MergeMethod {
    resolve_method(repo, settings.method.map(MergeMethod::from))
}

/// The pull requests that a merge of `pr` includes: the stack entries
/// with a position up to the position of `pr`, position 1 first, or `pr`
/// alone without a stack. `None` when fewer entries are loaded than the
/// position.
fn merged_entries(pr: &PullRequest, stack: &[PullRequest]) -> Option<Vec<PullRequest>> {
    let Some(stack_ref) = &pr.head.value.stack else {
        return Some(vec![pr.clone()]);
    };
    let top = stack_ref.position;
    let mut entries: Vec<PullRequest> = stack
        .iter()
        .filter(|entry| position(entry).is_some_and(|p| p <= top))
        .cloned()
        .collect();
    entries.sort_by_key(position);
    let wanted = usize::try_from(top).unwrap_or(usize::MAX);
    (entries.len() >= wanted).then_some(entries)
}

/// The stack position of a pull request.
fn position(pr: &PullRequest) -> Option<u32> {
    pr.head.value.stack.as_ref().map(|s| s.position)
}

/// The status line reason for a hard blocker. The reason is static, so
/// it names the kind and not the pull request.
fn hard_reason(entries: &[PullRequest]) -> &'static str {
    let single = entries.len() == 1;
    let draft = entries.iter().any(|pr| pr.head.value.is_draft);
    match (single, draft) {
        (true, true) => "the pull request is a draft",
        (true, false) => "the pull request has conflicts",
        (false, true) => "a pull request of the stack is a draft",
        (false, false) => "a pull request of the stack has conflicts",
    }
}

/// The lines of the confirm popup.
fn confirm_lines(entries: &[PullRequest], repo: &Repo, settings: &RepoMerge) -> Vec<String> {
    let mut lines = Vec::new();
    if entries.len() > 1 {
        lines.push(format!(
            "merges {} pull requests atomically, bottom first:",
            entries.len()
        ));
    }
    for pr in entries {
        let head = &pr.head.value;
        let place = position(pr).map_or_else(String::new, |p| format!("{p} "));
        lines.push(format!(
            "  {place}#{} {}  {}, {}",
            pr.key.number,
            head.title,
            ci_text(pr.ci_state.value),
            review_text(head.review_decision),
        ));
    }
    lines.push(format!("method: {}", method(repo, settings).as_str()));
    if settings.bypass_rules {
        lines.push("bypasses the branch rules".to_owned());
    }
    for warning in merge_blockers(entries).warnings {
        lines.push(format!("warning: {warning}"));
    }
    lines
}

/// The CI state for the confirm popup.
fn ci_text(state: CiState) -> &'static str {
    match state {
        CiState::Success => "CI passed",
        CiState::Failure => "CI failed",
        CiState::Pending => "CI running",
        CiState::None => "no CI",
    }
}

/// The review decision for the confirm popup.
fn review_text(decision: Option<ReviewDecision>) -> &'static str {
    match decision {
        Some(ReviewDecision::Approved) => "approved",
        Some(ReviewDecision::ChangesRequested) => "changes requested",
        Some(ReviewDecision::ReviewRequired) => "review required",
        None => "no review needed",
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::domain::keys::RepoKey;
    use crate::domain::local::{LocalChange, LocalMatch};
    use crate::domain::matching::tests::pr;
    use crate::domain::pr::{MergeStateStatus, Mergeable};
    use crate::domain::repo::RepoSettings;

    fn repo() -> Repo {
        Repo {
            key: RepoKey {
                owner: "o".into(),
                name: "r".into(),
            },
            path: None,
            settings: Some(RepoSettings {
                squash: true,
                merge: true,
                rebase: true,
                default_branch: None,
                squash_title_default: None,
                squash_message_default: None,
            }),
        }
    }

    /// A clean, approved pull request with green CI, in the stack `s` at
    /// `position`, or without a stack at position 0.
    fn entry(number: u64, position: u32) -> PullRequest {
        let mut pr = pr(number, if position == 0 { "" } else { "s" }, position);
        pr.head.value.mergeable = Mergeable::Mergeable;
        pr.head.value.merge_state = MergeStateStatus::Clean;
        pr.head.value.review_decision = Some(ReviewDecision::Approved);
        pr.ci_state.value = CiState::Success;
        pr
    }

    fn select(pr: PullRequest, stack: Vec<PullRequest>, config: Option<&str>) -> Selection {
        Selection {
            target: Target::Pr {
                pr,
                stack,
                local: LocalMatch::None,
                repo: repo(),
            },
            me: "me".into(),
            action_config: config.map(|text| toml::from_str(text).unwrap()),
        }
    }

    /// The settings of the repo `o/r` for the `[actions.merge]` table
    /// `config`.
    fn settings(config: &str) -> RepoMerge {
        repo_merge(&select(entry(9, 0), Vec::new(), Some(config))).unwrap()
    }

    #[test]
    fn top_level_settings_are_defaults_and_repo_entries_override_them() {
        use crate::config::schema::MergeMethodSetting::{Merge, Rebase};

        let top = "method = \"rebase\"\nbypass_rules = true\n";
        let other = "[repos.\"o/other\"]\nmethod = \"merge\"\nbypass_rules = false\n";
        let cases = [
            ("", None, false),
            (top, Some(Rebase), true),
            (&format!("{top}{other}"), Some(Rebase), true),
            (
                &format!("{top}[repos.\"o/r\"]\nmethod = \"merge\"\n"),
                Some(Merge),
                true,
            ),
            (
                &format!("{top}[repos.\"o/r\"]\nbypass_rules = false\n"),
                Some(Rebase),
                false,
            ),
            ("[repos.\"o/r\"]\nmethod = \"merge\"\n", Some(Merge), false),
        ];
        for (config, method, bypass_rules) in cases {
            assert_eq!(
                settings(config),
                RepoMerge {
                    method,
                    bypass_rules
                },
                "{config}"
            );
        }
    }

    #[test]
    fn a_bad_top_level_method_disables_the_merge() {
        let sel = select(entry(9, 0), Vec::new(), Some("method = \"fast\"\n"));
        assert!(repo_merge(&sel).is_err());
        assert_eq!(
            MergeAction::applies(&sel),
            Applicability::Disabled("invalid [actions.merge] configuration")
        );
    }

    fn stack3() -> Vec<PullRequest> {
        vec![entry(1, 1), entry(2, 2), entry(3, 3)]
    }

    fn confirm(title: &str, lines: &[&str]) -> Prepare {
        Prepare::Confirm(ConfirmSpec {
            title: title.into(),
            lines: lines.iter().map(|line| (*line).to_owned()).collect(),
            danger: true,
        })
    }

    #[test]
    fn enabled_cases_open_the_confirm_popup() {
        let mut warned = stack3();
        warned[0].head.value.merge_state = MergeStateStatus::Blocked;
        warned[1].head.value.mergeable = Mergeable::Unknown;
        warned[1].ci_state.value = CiState::Pending;
        warned[1].head.value.review_decision = Some(ReviewDecision::ReviewRequired);
        let mut draft_above = stack3();
        draft_above[2].head.value.is_draft = true;
        let override_config = "[repos.\"o/r\"]\nmethod = \"rebase\"\nbypass_rules = true\n";
        let other_repo = "[repos.\"o/other\"]\nmethod = \"merge\"\n";
        let single = ["  #7 Change 7  CI passed, approved", "method: squash"];
        let cases = [
            (
                "single pull request",
                select(entry(7, 0), Vec::new(), None),
                confirm("merge #7?", &single),
            ),
            (
                "stack top merges every entry, bottom first",
                select(
                    entry(3, 3),
                    vec![entry(3, 3), entry(1, 1), entry(2, 2)],
                    None,
                ),
                confirm(
                    "merge #3?",
                    &[
                        "merges 3 pull requests atomically, bottom first:",
                        "  1 #1 Change 1  CI passed, approved",
                        "  2 #2 Change 2  CI passed, approved",
                        "  3 #3 Change 3  CI passed, approved",
                        "method: squash",
                    ],
                ),
            ),
            (
                "stack middle ignores the entries above it",
                select(entry(2, 2), draft_above, None),
                confirm(
                    "merge #2?",
                    &[
                        "merges 2 pull requests atomically, bottom first:",
                        "  1 #1 Change 1  CI passed, approved",
                        "  2 #2 Change 2  CI passed, approved",
                        "method: squash",
                    ],
                ),
            ),
            (
                "stack bottom is one pull request",
                select(entry(1, 1), stack3(), None),
                confirm(
                    "merge #1?",
                    &["  1 #1 Change 1  CI passed, approved", "method: squash"],
                ),
            ),
            (
                "warnings show in the popup",
                select(entry(2, 2), warned, None),
                confirm(
                    "merge #2?",
                    &[
                        "merges 2 pull requests atomically, bottom first:",
                        "  1 #1 Change 1  CI passed, approved",
                        "  2 #2 Change 2  CI running, review required",
                        "method: squash",
                        "warning: #1 is blocked by the branch rules",
                        "warning: #2 merge state is not known yet",
                    ],
                ),
            ),
            (
                "the repo override sets the method and the bypass",
                select(entry(7, 0), Vec::new(), Some(override_config)),
                confirm(
                    "merge #7?",
                    &[
                        "  #7 Change 7  CI passed, approved",
                        "method: rebase",
                        "bypasses the branch rules",
                    ],
                ),
            ),
            (
                "an override of another repo does not apply",
                select(entry(7, 0), Vec::new(), Some(other_repo)),
                confirm("merge #7?", &single),
            ),
        ];
        for (name, sel, prepare) in cases {
            assert_eq!(MergeAction::applies(&sel), Applicability::Enabled, "{name}");
            assert_eq!(MergeAction::prepare(&sel), prepare, "{name}");
        }
    }

    #[test]
    fn disabled_cases_give_the_reason() {
        let mut draft_below = stack3();
        draft_below[0].head.value.is_draft = true;
        let mut conflict = entry(7, 0);
        conflict.head.value.mergeable = Mergeable::Conflicting;
        let mut draft = entry(7, 0);
        draft.head.value.is_draft = true;
        let mut conflict_below = stack3();
        conflict_below[1].head.value.mergeable = Mergeable::Conflicting;
        let cases = [
            (
                "stack not loaded",
                select(entry(3, 3), vec![entry(1, 1), entry(3, 3)], None),
                "stack not loaded",
            ),
            (
                "draft below in the stack",
                select(entry(3, 3), draft_below, None),
                "a pull request of the stack is a draft",
            ),
            (
                "conflict below in the stack",
                select(entry(3, 3), conflict_below, None),
                "a pull request of the stack has conflicts",
            ),
            (
                "draft pull request",
                select(draft, Vec::new(), None),
                "the pull request is a draft",
            ),
            (
                "conflicting pull request",
                select(conflict, Vec::new(), None),
                "the pull request has conflicts",
            ),
            (
                "invalid configuration",
                select(entry(7, 0), Vec::new(), Some("repos = 3")),
                "invalid [actions.merge] configuration",
            ),
        ];
        for (name, sel, reason) in cases {
            assert_eq!(
                MergeAction::applies(&sel),
                Applicability::Disabled(reason),
                "{name}"
            );
        }
    }

    /// A schedule in milliseconds, so the poll tests run fast.
    const FAST: Schedule = Schedule {
        fast: Duration::from_millis(5),
        fast_for: Duration::from_millis(50),
        slow: Duration::from_millis(10),
        limit: Duration::from_secs(5),
    };

    #[test]
    fn schedule_polls_fast_then_slow_then_stops() {
        let secs = Duration::from_secs;
        let cases = [
            (secs(0), Some(secs(2))),
            (secs(29), Some(secs(2))),
            (secs(30), Some(secs(5))),
            (secs(299), Some(secs(5))),
            (secs(300), None),
        ];
        for (elapsed, delay) in cases {
            assert_eq!(SCHEDULE.delay(elapsed), delay, "{elapsed:?}");
        }
    }

    fn pending(uuid: &str) -> serde_json::Value {
        serde_json::json!({
            "status": "pending",
            "details": { "message": "Merge request accepted", "uuid": uuid }
        })
    }

    /// Runs the merge of #7 against `server` and returns the result and
    /// the events.
    async fn run_merge(
        server: &wiremock::MockServer,
        path: Option<std::path::PathBuf>,
    ) -> (Result<String, String>, Vec<ActionEvent>) {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let client = crate::github::GithubClient::new(
            crate::github::TokenSource::command("echo", &["tok"]),
            &server.uri(),
        )
        .unwrap();
        let svc = Services {
            client: std::sync::Arc::new(client),
            locks: crate::vcs::lock::RepoLocks::default(),
            cancel: tokio_util::sync::CancellationToken::new(),
            tx,
        };
        let mut sel = select(entry(7, 0), Vec::new(), None);
        if let Target::Pr { repo, .. } = &mut sel.target {
            repo.path = path;
        }
        let result = merge(sel, svc, FAST).await;
        let mut events = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            let Msg::ActionEvent { id, key, event } = msg else {
                panic!("unexpected message {msg:?}");
            };
            assert_eq!(id, ID);
            assert_eq!(key, TargetKey::Pr(entry(7, 0).key));
            events.push(event);
        }
        (result, events)
    }

    async fn mount_put(server: &wiremock::MockServer, status: u16, body: serde_json::Value) {
        use wiremock::matchers::{method, path};
        wiremock::Mock::given(method("PUT"))
            .and(path("/repos/o/r/pulls/7/merge-async"))
            .respond_with(wiremock::ResponseTemplate::new(status).set_body_json(body))
            .expect(1)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn accepted_then_pending_then_merged() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        let server = wiremock::MockServer::start().await;
        mount_put(&server, 202, pending("u-1")).await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/pulls/7/merge-async/u-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending("u-1")))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/pulls/7/merge-async/u-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "merged",
                "details": { "message": "Pull request merged", "sha": "def456" }
            })))
            .mount(&server)
            .await;

        let (result, events) = run_merge(&server, None).await;

        assert_eq!(result, Ok("merged #7".to_owned()));
        assert_eq!(
            events,
            [
                ActionEvent::MergeStatus(MergeProgress::Pending),
                ActionEvent::MergeStatus(MergeProgress::Pending),
                ActionEvent::MergeStatus(MergeProgress::Merged {
                    sha: "def456".into()
                }),
            ]
        );
        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "merge_method": "squash",
                "commit_title": "Change 7 (#7)",
                "commit_message": "",
                "sha": "oid7",
                "merge_action": "default"
            })
        );
    }

    #[tokio::test]
    async fn accepted_then_failed() {
        use wiremock::matchers::method;
        use wiremock::{Mock, ResponseTemplate};
        let server = wiremock::MockServer::start().await;
        mount_put(&server, 202, pending("u-2")).await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "failed",
                "details": { "message": "Required status check \"ci\" is failing" }
            })))
            .mount(&server)
            .await;

        let (result, events) = run_merge(&server, None).await;

        let message = "Required status check \"ci\" is failing".to_owned();
        assert_eq!(result, Err(message.clone()));
        assert_eq!(
            events,
            [
                ActionEvent::MergeStatus(MergeProgress::Pending),
                ActionEvent::MergeStatus(MergeProgress::Failed(message)),
            ]
        );
    }

    #[tokio::test]
    async fn conflict_gives_the_message_of_github() {
        let server = wiremock::MockServer::start().await;
        let message = "Head branch was modified. Review and try the merge again.";
        mount_put(
            &server,
            409,
            serde_json::json!({ "status": "failed", "details": { "message": message } }),
        )
        .await;

        let (result, events) = run_merge(&server, None).await;

        assert_eq!(result, Err(message.to_owned()));
        assert_eq!(
            events,
            [ActionEvent::MergeStatus(MergeProgress::Failed(
                message.to_owned()
            ))]
        );
    }

    #[tokio::test]
    async fn a_failed_fetch_does_not_fail_the_merge() {
        let server = wiremock::MockServer::start().await;
        mount_put(
            &server,
            200,
            serde_json::json!({ "status": "merged", "details": { "sha": "def456" } }),
        )
        .await;
        let dir = tempfile::tempdir().unwrap();

        let (result, events) = run_merge(&server, Some(dir.path().to_path_buf())).await;

        let text = result.unwrap();
        assert!(text.starts_with("merged #7, jj git fetch failed"), "{text}");
        assert_eq!(
            events,
            [ActionEvent::MergeStatus(MergeProgress::Merged {
                sha: "def456".into()
            })]
        );
    }

    #[test]
    fn a_local_change_hides_the_action() {
        let sel = Selection {
            target: Target::Local {
                change: LocalChange {
                    workspace: "default".into(),
                    change_id: "kxyz".into(),
                    commit_id: "abc".into(),
                    bookmarks: Vec::new(),
                    empty: false,
                    parents: Vec::new(),
                    description: "local".into(),
                },
                repo: repo(),
            },
            me: "me".into(),
            action_config: None,
        };
        assert_eq!(MergeAction::applies(&sel), Applicability::Hidden);
    }
}
