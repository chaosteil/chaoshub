//! The merge action.
//!
//! It merges a pull request through the async merge endpoint of GitHub.
//! A merge of the pull request at position k of a stack includes the
//! positions 1 to k, so the blockers and the confirm lines cover every
//! entry up to k. The stack strategy picks how: `Atomic` sends one
//! request for position k, and GitHub merges the entries below it with
//! the default squash message of the repo. `Sequential` merges position 1
//! with its own title and body, waits until GitHub moves the next entry
//! to the stack base, merges that entry, and so on up to k.
//!
//! The settings come from the `[actions.merge]` table, which also gets
//! the keys of `[merge]`: `method`, `bypass_rules` (off by default) and
//! `stack` directly under the table are the defaults, and a
//! `repos."owner/name"` table overrides each key that it sets for one
//! repo.

use std::time::Duration;

use futures::future::BoxFuture;
use tokio::time::Instant;

use crate::bus::{
    ActionEvent, ActionInput, ConfirmSpec, KeyChord, KeyCode, KeyModifiers, MergeProgress,
    MergeStep, Msg, Selection, Target, TargetKey,
};
use crate::config::schema::{MergeConfig, RepoMerge, parse_repo_name};
use crate::domain::merge::{
    MergeMethod, StackStrategy, merge_blockers, resolve_method, squash_message, stack_strategy,
};
use crate::domain::pr::{CiState, PullRequest, ReviewDecision};
use crate::domain::repo::Repo;
use crate::github::parse::Me;
use crate::github::rest::MergeArgs;
use crate::vcs::{self, Tool, git, jj};

use super::{Action, Applicability, Group, Prepare, Services};

/// The id of the merge action.
pub(crate) const ID: &str = "merge";

/// The id of the stack merge action.
pub(crate) const STACK_ID: &str = "merge_stack";

/// The merge action.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MergeAction;

impl MergeAction {
    /// Disabled when the configuration table is invalid, when the stack
    /// entries up to the position are not loaded, or when an entry has a
    /// hard blocker.
    pub(crate) fn applies(sel: &Selection) -> Applicability {
        let Target::Pr { pr, stack, .. } = &sel.target;
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
        } = &sel.target;
        let entries = merged_entries(pr, stack).unwrap_or_else(|| vec![pr.clone()]);
        let settings = repo_merge(sel).unwrap_or_default();
        Prepare::Confirm(ConfirmSpec {
            title: format!("merge #{}?", pr.key.number),
            lines: confirm_lines(&entries, repo, &settings, strategy(pr, repo, &settings)),
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

    fn group(&self) -> Group {
        Group::Change
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
        let key = sel.target.key();
        Box::pin(merge(sel, key, ID, svc, SCHEDULE, RETARGET))
    }
}

/// The stack merge action. It merges the whole stack of the selected pull
/// request: it is the merge action on the top entry of the stack.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MergeStackAction;

/// The top entry of the stack of a selection.
enum Top {
    /// The pull request has no stack.
    NoStack,
    /// The top entry is not in the loaded stack entries.
    NotLoaded,
    /// The selection with the top entry as the pull request.
    Loaded(Box<Selection>),
}

/// The selection of the top entry of the stack of `sel`. The stack, the
/// local match, the repo, the user and the configuration table do not
/// change.
fn top_selection(sel: &Selection) -> Top {
    let Target::Pr { pr, stack, .. } = &sel.target;
    let Some(stack_ref) = &pr.head.value.stack else {
        return Top::NoStack;
    };
    let Some(top) = stack
        .iter()
        .find(|entry| position(entry) == Some(stack_ref.size))
    else {
        return Top::NotLoaded;
    };
    let mut top_sel = sel.clone();
    let Target::Pr { pr, .. } = &mut top_sel.target;
    *pr = top.clone();
    Top::Loaded(Box::new(top_sel))
}

impl Action for MergeStackAction {
    fn id(&self) -> &'static str {
        STACK_ID
    }

    fn label(&self) -> &'static str {
        "Merge stack"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('s'),
            mods: KeyModifiers::NONE,
        })
    }

    fn group(&self) -> Group {
        Group::Change
    }

    fn applies(&self, sel: &Selection) -> Applicability {
        match top_selection(sel) {
            Top::NoStack => Applicability::Hidden,
            Top::NotLoaded => Applicability::Disabled("stack not loaded"),
            Top::Loaded(top) => MergeAction::applies(&top),
        }
    }

    fn prepare(&self, sel: &Selection) -> Prepare {
        let Top::Loaded(top) = top_selection(sel) else {
            return MergeAction::prepare(sel);
        };
        match MergeAction::prepare(&top) {
            Prepare::Confirm(spec) => {
                let Target::Pr { pr, .. } = &top.target;
                Prepare::Confirm(ConfirmSpec {
                    title: format!("merge the stack up to #{}?", pr.key.number),
                    ..spec
                })
            }
            other => other,
        }
    }

    fn execute(
        &self,
        sel: Selection,
        _input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        let key = sel.target.key();
        let Top::Loaded(top) = top_selection(&sel) else {
            return Box::pin(async { Err("stack not loaded".to_owned()) });
        };
        Box::pin(merge(*top, key, STACK_ID, svc, SCHEDULE, RETARGET))
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

/// When the action polls an entry of a stack for its new base, after the
/// merge of the entry below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Retarget {
    /// The poll interval.
    every: Duration,
    /// The time after which the action stops the poll.
    limit: Duration,
}

/// Every 3 s, up to 2 min.
const RETARGET: Retarget = Retarget {
    every: Duration::from_secs(3),
    limit: Duration::from_mins(2),
};

/// The final state of one merge request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Final {
    Merged,
    Enqueued,
}

/// One run of the merge action: the services, the key and the action id
/// of its events and the settings that every merge request of the run
/// shares.
struct Run<'a> {
    svc: &'a Services,
    key: TargetKey,
    id: &'static str,
    method: MergeMethod,
    bypass_rules: bool,
    schedule: Schedule,
    retarget: Retarget,
}

/// Merges the target, and fetches the repo after a merge. A stack with
/// more than one merged entry and the `Sequential` strategy merges one
/// entry after the other. Every step goes to `app` as
/// `ActionEvent::MergeStep`, and every state as `ActionEvent::MergeStatus`,
/// with the action `id` and the row `key`. A cancel stops the poll, not
/// the merge on GitHub.
async fn merge(
    sel: Selection,
    key: TargetKey,
    id: &'static str,
    svc: Services,
    schedule: Schedule,
    retarget: Retarget,
) -> Result<String, String> {
    let Target::Pr {
        pr, stack, repo, ..
    } = &sel.target;
    let settings = repo_merge(&sel)?;
    let run = Run {
        svc: &svc,
        key,
        id,
        method: method(repo, &settings),
        bypass_rules: settings.bypass_rules,
        schedule,
        retarget,
    };
    let entries = merged_entries(pr, stack).unwrap_or_else(|| vec![pr.clone()]);
    if let Some(stack_ref) = &pr.head.value.stack
        && entries.len() > 1
        && strategy(pr, repo, &settings) == StackStrategy::Sequential
    {
        return run
            .sequence(&entries, &stack_ref.base_ref, repo, &sel.me)
            .await;
    }
    let number = pr.key.number;
    let step = MergeStep::Entry {
        number,
        index: 1,
        total: 1,
    };
    run.send(ActionEvent::MergeStep(step)).await;
    match run.merge_one(pr).await? {
        Final::Merged => Ok(run.fetch(repo, format!("merged #{number}")).await),
        Final::Enqueued => Ok(format!("#{number} in merge queue")),
    }
}

impl Run<'_> {
    /// Requests the async merge of `pr` with its own squash message and
    /// polls its state up to a final state. A failure sends
    /// `MergeStatus(Failed)` and gives the message. A merge that is still
    /// pending at the poll limit sends no `Failed`, like a cancel: the
    /// merge can still land on GitHub.
    async fn merge_one(&self, pr: &PullRequest) -> Result<Final, String> {
        let number = pr.key.number;
        let (title, body) = squash_message(pr);
        let args = MergeArgs {
            method: self.method.as_str(),
            commit_title: &title,
            commit_message: &body,
            sha: &pr.head.value.head_oid,
            bypass_rules: self.bypass_rules,
        };
        let started = match self.svc.client.merge_async(&pr.key, &args).await {
            Ok(started) => started,
            Err(err) => return self.fail(err.to_string()).await,
        };
        let mut progress = started.progress;
        self.send(ActionEvent::MergeStatus(progress.clone())).await;
        let start = Instant::now();
        while progress == MergeProgress::Pending {
            let Some(uuid) = &started.uuid else {
                return self.fail("GitHub gave no merge id".to_owned()).await;
            };
            let Some(delay) = self.schedule.delay(start.elapsed()) else {
                return Err(format!("merge of #{number} still pending, see GitHub"));
            };
            tokio::select! {
                () = self.svc.cancel.cancelled() => {
                    return Err(format!("stopped the poll, the merge of #{number} goes on"));
                }
                () = tokio::time::sleep(delay) => {}
            }
            progress = match self.svc.client.merge_async_status(&pr.key, uuid).await {
                Ok(progress) => progress,
                Err(err) => return self.fail(format!("merge state unknown: {err}")).await,
            };
            self.send(ActionEvent::MergeStatus(progress.clone())).await;
        }
        match progress {
            MergeProgress::Merged { .. } => Ok(Final::Merged),
            MergeProgress::Enqueued => Ok(Final::Enqueued),
            MergeProgress::Failed(message) => Err(message),
            MergeProgress::Pending => Err(format!("merge of #{number} still pending")),
        }
    }

    /// Merges `entries` one by one, position 1 first, each with its own
    /// squash message. Before each entry above position 1 it waits until
    /// GitHub moves the entry to `base`, the base of the stack. Sends
    /// `MergeStep::Retarget` before each wait, `MergeStep::Entry` before
    /// each merge request and `Progress("merged #N (k of n)")` per merged
    /// entry. A failure stops
    /// the sequence, and the error names the entries that merged.
    async fn sequence(
        &self,
        entries: &[PullRequest],
        base: &str,
        repo: &Repo,
        me: &str,
    ) -> Result<String, String> {
        let total = entries.len();
        let mut merged = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let number = entry.key.number;
            let current = if index == 0 {
                entry.clone()
            } else {
                let step = MergeStep::Retarget { number };
                self.send(ActionEvent::MergeStep(step)).await;
                self.retargeted(entry, base, me)
                    .await
                    .map_err(|err| stopped(number, &err, &merged))?
            };
            let step = MergeStep::Entry {
                number,
                index: index + 1,
                total,
            };
            self.send(ActionEvent::MergeStep(step)).await;
            match self.merge_one(&current).await {
                Ok(Final::Merged) => {
                    merged.push(number);
                    let text = format!("merged #{number} ({} of {total})", index + 1);
                    self.send(ActionEvent::Progress(text)).await;
                }
                Ok(Final::Enqueued) if index + 1 == total => {
                    let text = format!("merged {}, #{number} in merge queue", numbers(&merged));
                    return Ok(self.fetch(repo, text).await);
                }
                Ok(Final::Enqueued) => {
                    return Err(stopped(number, "in merge queue", &merged));
                }
                Err(err) => return Err(stopped(number, &err, &merged)),
            }
        }
        Ok(self
            .fetch(repo, format!("merged {}", numbers(&merged)))
            .await)
    }

    /// Reads `entry` by id until its base is `base`, every
    /// `retarget.every`, up to `retarget.limit`. Gives the fresh pull
    /// request, so the merge uses its current head and message.
    async fn retargeted(
        &self,
        entry: &PullRequest,
        base: &str,
        me: &str,
    ) -> Result<PullRequest, String> {
        let number = entry.key.number;
        let ids = [entry.node_id.clone()];
        let me = Me {
            login: me,
            teams: &[],
        };
        let start = Instant::now();
        loop {
            match self.svc.client.pr_by_ids(&ids, &me).await {
                Ok((prs, _)) => {
                    let fresh = prs.into_iter().find(|pr| pr.key == entry.key);
                    if let Some(pr) = fresh.filter(|pr| pr.head.value.base_ref == base) {
                        return Ok(pr);
                    }
                }
                Err(err) => {
                    return self
                        .fail(format!("refresh of #{number} failed: {err}"))
                        .await;
                }
            }
            if start.elapsed() >= self.retarget.limit {
                return self.fail(format!("#{number} did not move to {base}")).await;
            }
            tokio::select! {
                () = self.svc.cancel.cancelled() => {
                    return Err(format!("stopped the wait for #{number}"));
                }
                () = tokio::time::sleep(self.retarget.every) => {}
            }
        }
    }

    /// Fetches the repo after a merge, under the repo lock, and sends
    /// `Progress("fetched")`. Gives `text`. A jj repo runs `jj git fetch`,
    /// and a git repo runs `git fetch origin`. A repo without a path has
    /// nothing to fetch. A failed fetch does not fail the merge, and its
    /// text names the tool that ran.
    async fn fetch(&self, repo: &Repo, text: String) -> String {
        let Some(path) = &repo.path else {
            return text;
        };
        let (tool, result) = match vcs::detect(path, vcs::jj_on_path()) {
            Ok(Tool::Jj) => ("jj git fetch", jj::git_fetch(&self.svc.locks, path).await),
            Ok(Tool::Git | Tool::GitWithoutJj) => {
                ("git fetch", git::fetch_origin(&self.svc.locks, path).await)
            }
            Err(err) => {
                tracing::warn!(%text, %err, "no fetch after the merge");
                return format!("{text}, no fetch: {err}");
            }
        };
        match result {
            Ok(()) => {
                self.send(ActionEvent::Progress("fetched".to_owned())).await;
                text
            }
            Err(err) => {
                tracing::warn!(%text, %err, tool, "the fetch after the merge failed");
                format!("{text}, {tool} failed: {err}")
            }
        }
    }

    /// Sends `MergeStatus(Failed)` and returns the failure.
    async fn fail<T>(&self, text: String) -> Result<T, String> {
        let event = ActionEvent::MergeStatus(MergeProgress::Failed(text.clone()));
        self.send(event).await;
        Err(text)
    }

    /// Sends an event of the merge action to `app`. A closed channel means
    /// that `app` is gone, so the event has no reader.
    async fn send(&self, event: ActionEvent) {
        let msg = Msg::ActionEvent {
            id: self.id,
            key: self.key.clone(),
            event,
        };
        if self.svc.tx.send(msg).await.is_err() {
            tracing::debug!("app is gone, merge event dropped");
        }
    }
}

/// The error of a stopped sequence: the entry, the reason and the entries
/// that merged before the stop.
fn stopped(number: u64, reason: &str, merged: &[u64]) -> String {
    let merged = if merged.is_empty() {
        "none".to_owned()
    } else {
        numbers(merged)
    };
    format!("#{number}: {reason}; merged before the stop: {merged}")
}

/// The pull request numbers as `#1, #2`.
fn numbers(numbers: &[u64]) -> String {
    numbers
        .iter()
        .map(|number| format!("#{number}"))
        .collect::<Vec<_>>()
        .join(", ")
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

/// The stack strategy for a merge of `pr`, from the squash defaults of
/// its repo, the merge method and the `stack` setting.
fn strategy(pr: &PullRequest, repo: &Repo, settings: &RepoMerge) -> StackStrategy {
    stack_strategy(
        &pr.head.value.repo_settings,
        method(repo, settings),
        settings.stack.stack_override(),
    )
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
fn confirm_lines(
    entries: &[PullRequest],
    repo: &Repo,
    settings: &RepoMerge,
    strategy: StackStrategy,
) -> Vec<String> {
    let mut lines = Vec::new();
    if entries.len() > 1 {
        let count = entries.len();
        lines.push(match strategy {
            StackStrategy::Atomic => {
                format!("merges {count} pull requests atomically, bottom first:")
            }
            StackStrategy::Sequential => format!(
                "merges {count} pull requests one by one, bottom first, each with its own message:"
            ),
        });
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
    use crate::domain::local::LocalMatch;
    use crate::domain::matching::tests::pr;
    use crate::domain::pr::{MergeStateStatus, Mergeable};
    use crate::domain::repo::{RepoSettings, SquashMessageDefault, SquashTitleDefault};

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
                    bypass_rules,
                    stack: crate::config::schema::StackSetting::Auto,
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
                        "merges 3 pull requests one by one, bottom first, each with its own message:",
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
                        "merges 2 pull requests one by one, bottom first, each with its own message:",
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
                        "merges 2 pull requests one by one, bottom first, each with its own message:",
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
    fn the_stack_strategy_picks_the_first_confirm_line() {
        let mut body_defaults = vec![entry(1, 1), entry(2, 2)];
        for pr in &mut body_defaults {
            let settings = &mut pr.head.value.repo_settings;
            settings.squash_title_default = Some(SquashTitleDefault::PrTitle);
            settings.squash_message_default = Some(SquashMessageDefault::PrBody);
        }
        let cases = [
            (
                "squash defaults of title and body merge atomically",
                select(body_defaults[1].clone(), body_defaults, None),
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
                "the atomic setting merges atomically",
                select(entry(2, 2), stack3(), Some("stack = \"atomic\"\n")),
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
        ];
        for (name, sel, prepare) in cases {
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

    #[test]
    fn merge_stack_merges_up_to_the_top_entry() {
        let sel = select(entry(1, 1), stack3(), None);
        assert_eq!(MergeStackAction.applies(&sel), Applicability::Enabled);
        assert_eq!(
            MergeStackAction.prepare(&sel),
            confirm(
                "merge the stack up to #3?",
                &[
                    "merges 3 pull requests one by one, bottom first, each with its own message:",
                    "  1 #1 Change 1  CI passed, approved",
                    "  2 #2 Change 2  CI passed, approved",
                    "  3 #3 Change 3  CI passed, approved",
                    "method: squash",
                ],
            )
        );
    }

    #[test]
    fn merge_stack_is_hidden_or_disabled() {
        let mut draft_top = stack3();
        draft_top[2].head.value.is_draft = true;
        let draft_sel = select(entry(1, 1), draft_top, None);
        assert_eq!(MergeAction::applies(&draft_sel), Applicability::Enabled);
        let cases = [
            (
                "no stack",
                select(entry(7, 0), Vec::new(), None),
                Applicability::Hidden,
            ),
            (
                "top entry not loaded",
                select(entry(1, 1), vec![entry(1, 1), entry(2, 2)], None),
                Applicability::Disabled("stack not loaded"),
            ),
            (
                "draft at the top",
                draft_sel,
                Applicability::Disabled("a pull request of the stack is a draft"),
            ),
        ];
        for (name, sel, applies) in cases {
            assert_eq!(MergeStackAction.applies(&sel), applies, "{name}");
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

    /// The step event of the merge of #`number`, the entry `index` of
    /// `total`.
    fn step(number: u64, index: usize, total: usize) -> ActionEvent {
        ActionEvent::MergeStep(MergeStep::Entry {
            number,
            index,
            total,
        })
    }

    /// The step event of the wait for the retarget of #`number`.
    fn retarget(number: u64) -> ActionEvent {
        ActionEvent::MergeStep(MergeStep::Retarget { number })
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
        let mut sel = select(entry(7, 0), Vec::new(), None);
        let Target::Pr { repo, .. } = &mut sel.target;
        repo.path = path;
        run_selection(server, sel).await
    }

    /// A retarget poll in milliseconds.
    const FAST_RETARGET: Retarget = Retarget {
        every: Duration::from_millis(5),
        limit: Duration::from_secs(5),
    };

    /// Runs the merge of `sel` against `server` and returns the result and
    /// the events. Every event has the key of the target.
    async fn run_selection(
        server: &wiremock::MockServer,
        sel: Selection,
    ) -> (Result<String, String>, Vec<ActionEvent>) {
        run_scheduled(server, sel, FAST).await
    }

    /// `run_selection` with the poll schedule `schedule`.
    async fn run_scheduled(
        server: &wiremock::MockServer,
        sel: Selection,
        schedule: Schedule,
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
        let target = sel.target.key();
        let result = merge(sel, target.clone(), ID, svc, schedule, FAST_RETARGET).await;
        let mut events = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            let Msg::ActionEvent { id, key, event } = msg else {
                panic!("unexpected message {msg:?}");
            };
            assert_eq!(id, ID);
            assert_eq!(key, target);
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
                step(7, 1, 1),
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
                step(7, 1, 1),
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
            [
                step(7, 1, 1),
                ActionEvent::MergeStatus(MergeProgress::Failed(message.to_owned())),
            ]
        );
    }

    #[tokio::test]
    async fn a_merge_pending_at_the_poll_limit_sends_no_failure() {
        use wiremock::matchers::method;
        use wiremock::{Mock, ResponseTemplate};
        let server = wiremock::MockServer::start().await;
        mount_put(&server, 202, pending("u-3")).await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending("u-3")))
            .mount(&server)
            .await;
        let schedule = Schedule {
            limit: Duration::from_millis(20),
            ..FAST
        };

        let sel = select(entry(7, 0), Vec::new(), None);
        let (result, events) = run_scheduled(&server, sel, schedule).await;

        assert_eq!(
            result,
            Err("merge of #7 still pending, see GitHub".to_owned())
        );
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, ActionEvent::MergeStatus(MergeProgress::Failed(_)))),
            "{events:?}"
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
        assert!(text.starts_with("merged #7, git fetch failed"), "{text}");
        assert_eq!(
            events,
            [
                step(7, 1, 1),
                ActionEvent::MergeStatus(MergeProgress::Merged {
                    sha: "def456".into()
                }),
            ]
        );
    }

    #[tokio::test]
    async fn the_fetch_after_a_merge_runs_git_in_a_git_only_repo() {
        use crate::vcs::testrepo::{self, git, s};

        if testrepo::git_missing() {
            eprintln!("skipped: git is missing");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let repo = testrepo::git_only_repo(tmp.path());
        // A second clone pushes a branch that the repo does not know yet.
        let other = tmp.path().join("other");
        git(
            tmp.path(),
            &[
                "clone",
                "--quiet",
                s(&tmp.path().join("origin.git")),
                s(&other),
            ],
        );
        git(
            &other,
            &["commit", "--quiet", "--allow-empty", "-m", "third"],
        );
        git(
            &other,
            &["push", "--quiet", "origin", "HEAD:refs/heads/later"],
        );
        let later = git(&other, &["rev-parse", "HEAD"]);
        let server = wiremock::MockServer::start().await;
        mount_put(
            &server,
            200,
            serde_json::json!({ "status": "merged", "details": { "sha": "def456" } }),
        )
        .await;

        let (result, events) = run_merge(&server, Some(repo.clone())).await;

        assert_eq!(result, Ok("merged #7".to_owned()));
        assert_eq!(
            events,
            [
                step(7, 1, 1),
                ActionEvent::MergeStatus(MergeProgress::Merged {
                    sha: "def456".into()
                }),
                ActionEvent::Progress("fetched".into()),
            ]
        );
        assert_eq!(git(&repo, &["rev-parse", "origin/later"]), later);
    }

    /// A pull request node of the `PrByIds` query: #`number` of `o/r` at
    /// position `number` of the stack `s`, with the base `base`.
    fn node_json(number: u64, base: &str) -> serde_json::Value {
        serde_json::json!({
            "__typename": "PullRequest",
            "id": format!("PR_{number}"),
            "number": number,
            "title": format!("Fresh {number}"),
            "body": format!("Fresh body {number}\r\n"),
            "bodyHTML": format!("<p dir=\"auto\">Fresh body {number}</p>"),
            "url": format!("https://github.com/o/r/pull/{number}"),
            "isDraft": false,
            "state": "OPEN",
            "createdAt": "2026-09-01T12:00:00Z",
            "updatedAt": "2026-10-30T05:09:51Z",
            "headRefName": format!("me/{number}"),
            "headRefOid": format!("fresh{number}"),
            "additions": 5,
            "deletions": 1,
            "changedFiles": 1,
            "baseRefName": base,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "reviewDecision": "APPROVED",
            "author": { "__typename": "User", "login": "me" },
            "autoMergeRequest": null,
            "mergeQueueEntry": null,
            "viewerLatestReviewRequest": null,
            "reviewRequests": { "nodes": [] },
            "latestOpinionatedReviews": { "nodes": [] },
            "statusCheckRollup": null,
            "stack": { "id": "s", "number": 1, "size": 3, "baseRefName": "main" },
            "stackEntry": { "position": number },
            "repository": {
                "nameWithOwner": "o/r",
                "squashMergeAllowed": true,
                "mergeCommitAllowed": true,
                "rebaseMergeAllowed": true,
                "squashMergeCommitTitle": "COMMIT_OR_PR_TITLE",
                "squashMergeCommitMessage": "COMMIT_MESSAGES"
            }
        })
    }

    /// Answers the `PrByIds` query for #`number` with the base `base`,
    /// `times` times, or always when `times` is `None`.
    async fn mount_by_id(
        server: &wiremock::MockServer,
        number: u64,
        base: &str,
        times: Option<u64>,
    ) {
        use wiremock::matchers::{body_string_contains, method, path};
        let body = serde_json::json!({
            "data": {
                "nodes": [node_json(number, base)],
                "rateLimit": { "cost": 1, "remaining": 4999, "resetAt": "2026-10-30T12:00:00Z" }
            }
        });
        let mock = wiremock::Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains(format!("\"PR_{number}\"")))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body));
        match times {
            Some(times) => mock.up_to_n_times(times).mount(server).await,
            None => mock.mount(server).await,
        }
    }

    /// Answers the merge request of #`number`.
    async fn mount_merge(
        server: &wiremock::MockServer,
        number: u64,
        status: u16,
        body: serde_json::Value,
        times: u64,
    ) {
        use wiremock::matchers::{method, path};
        wiremock::Mock::given(method("PUT"))
            .and(path(format!("/repos/o/r/pulls/{number}/merge-async")))
            .respond_with(wiremock::ResponseTemplate::new(status).set_body_json(body))
            .expect(times)
            .mount(server)
            .await;
    }

    /// The bodies of the merge requests, in the order of the requests.
    async fn merge_bodies(server: &wiremock::MockServer) -> Vec<serde_json::Value> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method == wiremock::http::Method::PUT)
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect()
    }

    /// The stack entries 1 to `count` with a body each, and a selection of
    /// the top entry.
    fn sequential_stack(count: u32) -> Selection {
        let entries: Vec<PullRequest> = (1..=count)
            .map(|position| {
                let mut pr = entry(u64::from(position), position);
                pr.head.value.body = format!("Body {position}");
                pr.head.value.base_ref = if position == 1 {
                    "main".into()
                } else {
                    format!("me/{}", position - 1)
                };
                pr
            })
            .collect();
        let top = entries[entries.len() - 1].clone();
        select(top, entries, None)
    }

    #[tokio::test]
    async fn sequential_stack_merges_bottom_first_with_each_message() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        let server = wiremock::MockServer::start().await;
        mount_merge(&server, 1, 202, pending("u-1"), 1).await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/pulls/1/merge-async/u-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "merged",
                "details": { "sha": "sha1" }
            })))
            .mount(&server)
            .await;
        mount_by_id(&server, 2, "me/1", Some(1)).await;
        mount_by_id(&server, 2, "main", None).await;
        mount_merge(
            &server,
            2,
            200,
            serde_json::json!({ "status": "merged", "details": { "sha": "sha2" } }),
            1,
        )
        .await;

        let (result, events) = run_selection(&server, sequential_stack(2)).await;

        assert_eq!(result, Ok("merged #1, #2".to_owned()));
        let merged = |sha: &str| {
            ActionEvent::MergeStatus(MergeProgress::Merged {
                sha: sha.to_owned(),
            })
        };
        assert_eq!(
            events,
            [
                step(1, 1, 2),
                ActionEvent::MergeStatus(MergeProgress::Pending),
                merged("sha1"),
                ActionEvent::Progress("merged #1 (1 of 2)".into()),
                retarget(2),
                step(2, 2, 2),
                merged("sha2"),
                ActionEvent::Progress("merged #2 (2 of 2)".into()),
            ]
        );
        let by_ids = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path() == "/graphql")
            .count();
        assert_eq!(by_ids, 2, "one poll before the retarget, one after");
        assert_eq!(
            merge_bodies(&server).await,
            [
                serde_json::json!({
                    "merge_method": "squash",
                    "commit_title": "Change 1 (#1)",
                    "commit_message": "Body 1",
                    "sha": "oid1",
                    "merge_action": "default"
                }),
                serde_json::json!({
                    "merge_method": "squash",
                    "commit_title": "Fresh 2 (#2)",
                    "commit_message": "Fresh body 2",
                    "sha": "fresh2",
                    "merge_action": "default"
                }),
            ]
        );
    }

    #[tokio::test]
    async fn merge_stack_from_the_bottom_merges_every_entry() {
        let server = wiremock::MockServer::start().await;
        let merged =
            |sha: &str| serde_json::json!({ "status": "merged", "details": { "sha": sha } });
        mount_merge(&server, 1, 200, merged("sha1"), 1).await;
        mount_by_id(&server, 2, "main", None).await;
        mount_merge(&server, 2, 200, merged("sha2"), 1).await;
        let mut sel = sequential_stack(2);
        let Target::Pr { pr, stack, .. } = &mut sel.target;
        for entry in stack.iter_mut() {
            entry.head.value.stack.as_mut().unwrap().size = 2;
        }
        *pr = stack[0].clone();
        let bottom = sel.target.key();
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

        let result = MergeStackAction
            .execute(sel, ActionInput::Confirmed, svc)
            .await;

        assert_eq!(result, Ok("merged #1, #2".to_owned()));
        let mut events = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            let Msg::ActionEvent { id, key, event } = msg else {
                panic!("unexpected message {msg:?}");
            };
            assert_eq!(id, STACK_ID);
            assert_eq!(key, bottom);
            events.push(event);
        }
        assert_eq!(
            events,
            [
                step(1, 1, 2),
                ActionEvent::MergeStatus(MergeProgress::Merged { sha: "sha1".into() }),
                ActionEvent::Progress("merged #1 (1 of 2)".into()),
                retarget(2),
                step(2, 2, 2),
                ActionEvent::MergeStatus(MergeProgress::Merged { sha: "sha2".into() }),
                ActionEvent::Progress("merged #2 (2 of 2)".into()),
            ]
        );
        let titles: Vec<_> = merge_bodies(&server)
            .await
            .iter()
            .map(|body| body["commit_title"].clone())
            .collect();
        assert_eq!(titles, ["Change 1 (#1)", "Fresh 2 (#2)"]);
    }

    #[tokio::test]
    async fn a_failure_in_the_middle_names_the_merged_entries() {
        let server = wiremock::MockServer::start().await;
        let merged = serde_json::json!({ "status": "merged", "details": { "sha": "sha1" } });
        mount_merge(&server, 1, 200, merged, 1).await;
        mount_by_id(&server, 2, "main", None).await;
        let message = "Required status check \"ci\" is failing";
        mount_merge(
            &server,
            2,
            409,
            serde_json::json!({ "status": "failed", "details": { "message": message } }),
            1,
        )
        .await;
        mount_merge(&server, 3, 200, serde_json::json!({}), 0).await;

        let (result, events) = run_selection(&server, sequential_stack(3)).await;

        assert_eq!(
            result,
            Err(format!("#2: {message}; merged before the stop: #1"))
        );
        assert_eq!(
            events,
            [
                step(1, 1, 3),
                ActionEvent::MergeStatus(MergeProgress::Merged { sha: "sha1".into() }),
                ActionEvent::Progress("merged #1 (1 of 3)".into()),
                retarget(2),
                step(2, 2, 3),
                ActionEvent::MergeStatus(MergeProgress::Failed(message.to_owned())),
            ]
        );
    }
}
