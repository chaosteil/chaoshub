//! The `update` function: applies a `Msg` and returns commands.
//!
//! There is one function per message family. Commands are plain data, so
//! the tests compare them with `assert_eq!`.

use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use crate::actions::{Action, Applicability, Prepare, REGISTRY};
use crate::app::frame;
use crate::app::help::help_model;
use crate::app::keymap::{self, Keymap};
use crate::app::layout::Areas;
use crate::app::models::{Cell, CellStyle, PaneModel, RowItem};
use crate::app::nav::{self, Motion};
use crate::app::panes::{PANES, Pane, PaneState, applicable, display_rows, items};
use crate::app::state::MergeState;
use crate::app::state::PrefetchBatch;
use crate::app::state::{
    AUTHOR_KEY, ActionDialog, AppData, AppState, AuthorPicker, COLUMN_HEADERS_KEY, Condition,
    FILTER_KEY, Failure, OUTPUT_LINES, Overlay, OverlayState, PaneHandleState, PopupState,
    error_text, ui_key,
};
use crate::app::tabs::{Handled, KeyCx, TABS, Tab, cells, pr_ref};
use crate::app::target::{pr_target, row_target};
use crate::app::term::term_status;
use crate::bus::MergeProgress;
use crate::bus::{
    ActionEvent, ActionInput, AgentJob, Cmd, Hit, KeyChord, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, LaunchRequest, MouseKind, Msg, OverlayKind, Persist, PrRef, Scope, Selection,
    Size, SyncError, Target, TargetKey, UiEvent,
};
use crate::bus::{ConfirmSpec, PromptSpec};
use crate::domain::agent::{AgentResults, Record, Tool, ToolKind, short_tokens};
use crate::domain::check::Check;
use crate::domain::detail::PrDetail;
use crate::domain::files::PrFiles;
use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::local::LocalState;
use crate::domain::observed::{Observed, apply_newer};
use crate::domain::pr::{CiState, PrState, PullRequest};
use crate::domain::repo::Repo;

/// Applies one message to the state, with the tabs of `TABS`.
pub(crate) fn update(state: &mut AppState, keymap: &Keymap, msg: Msg) -> Vec<Cmd> {
    update_with(state, keymap, &TABS, msg)
}

/// Applies one message to the state, with the tabs of `tabs`, the panes
/// of `PANES` and the actions of `REGISTRY`.
pub(crate) fn update_with(
    state: &mut AppState,
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    msg: Msg,
) -> Vec<Cmd> {
    update_full(state, keymap, tabs, &PANES, &REGISTRY, msg)
}

/// Applies one message to the state, with the tabs of `tabs`, the panes
/// of `panes` and the actions of `actions`.
#[allow(
    clippy::too_many_lines,
    reason = "the dispatch has one arm per message"
)]
pub(crate) fn update_full(
    state: &mut AppState,
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    panes: &[Box<dyn Pane>],
    actions: &[Box<dyn Action>],
    msg: Msg,
) -> Vec<Cmd> {
    match msg {
        Msg::Key(event) => key(state, keymap, tabs, panes, actions, &event),
        Msg::Mouse { kind, hit, .. } => mouse(state, keymap, tabs, panes, actions, kind, hit),
        Msg::Resize(size) => resize(state, size),
        Msg::Tick => tick(state, tabs, Utc::now()),
        Msg::TransitionsDone => transitions_done(state),
        Msg::Signal => quit(),
        Msg::StartupDone { login, cwd_repo } => {
            let mut cmds = startup_done(state, tabs, login, cwd_repo);
            cmds.extend(after_change(state, tabs));
            cmds
        }
        Msg::SyncStarted { scope } => sync_started(state, scope),
        Msg::PrsSynced {
            scope,
            prs,
            seen,
            started_at,
            last,
        } => {
            back_online(state);
            let before = glow_before(state, tabs, state.data.synced.contains_key(&scope));
            let mut cmds = prs_synced(state, scope, prs, seen, started_at, last);
            push_glows(state, &before);
            cmds.extend(after_change(state, tabs));
            cmds
        }
        Msg::PrRefreshed(pr) => {
            back_online(state);
            if pr_refreshed(state, *pr) {
                after_change(state, tabs)
            } else {
                Vec::new()
            }
        }
        Msg::JjScanned { repo, state: scan } => {
            if jj_scanned(state, repo, scan) {
                after_change(state, tabs)
            } else {
                Vec::new()
            }
        }
        Msg::CiUpdated {
            states,
            selected_checks,
        } => {
            back_online(state);
            let before = glow_before(state, tabs, !state.data.synced.is_empty());
            if ci_updated(state, states, selected_checks) {
                push_glows(state, &before);
                after_change(state, tabs)
            } else {
                Vec::new()
            }
        }
        Msg::DetailLoading(key) => detail_loading(state, key),
        Msg::DetailLoaded(key, result) => {
            if result.is_ok() {
                back_online(state);
            }
            if detail_loaded(state, key, result) {
                after_change(state, tabs)
            } else {
                Vec::new()
            }
        }
        Msg::FilesLoading(key) => files_loading(state, key),
        Msg::FilesLoaded(key, result) => {
            if result.is_ok() {
                back_online(state);
            }
            if files_loaded(state, key, result) {
                after_change(state, tabs)
            } else {
                Vec::new()
            }
        }
        Msg::SyncFailed { source, scope, err } => sync_failed(state, source, scope, &err),
        Msg::WorkerDied { name, restart_in } => worker_died(state, name, restart_in),
        Msg::Rate(info) => {
            state.data.sync.rate = Some(info);
            Vec::new()
        }
        Msg::ActionEvent {
            key: TargetKey::Pr(key),
            event: ActionEvent::MergeStatus(progress),
            ..
        } => merge_status(state, key, progress),
        Msg::ActionEvent { id, key, event } => action_event(state, actions, id, key, event),
        msg @ (Msg::AgentStarted { .. } | Msg::AgentFinished { .. } | Msg::AgentLoaded { .. }) => {
            agent_msg(state, msg)
        }
        Msg::CodeviewLoaded { key, comments } => codeview_loaded(state, key, comments),
        Msg::PaneStarted { id, key } => {
            state.data.panes.insert((id, key), PaneHandleState::Running);
            state.bump();
            Vec::new()
        }
        Msg::PaneEnded { id, key } => {
            if state.data.panes.remove(&(id, key.clone())).is_some() {
                state.bump();
            }
            let mut cmds = follow_up(&state.data, &key);
            // A review in codeview can add comments or send them.
            if id == REVIEW_ID {
                let TargetKey::Pr(pr) = &key;
                cmds.push(Cmd::LoadCodeview(pr.clone()));
            }
            cmds
        }
        Msg::PrefetchDone(..) => {
            tracing::trace!("message not handled yet");
            Vec::new()
        }
        Msg::PrefetchBatch { done, total } => {
            prefetch_batch(state, done, total);
            Vec::new()
        }
    }
}

/// Stores the progress of a prefetch batch. `done == total` ends the
/// batch and clears it.
fn prefetch_batch(state: &mut AppState, done: usize, total: usize) {
    let batch = (done < total).then_some(PrefetchBatch { done, total });
    if state.data.prefetch != batch {
        state.data.prefetch = batch;
        state.bump();
    }
}

/// A key press: the quit keys quit, the app keys and the action keys
/// act, every other key goes to the active tab. After a handled key, the
/// generation moves when the tab says so, the UI events queue up, and a
/// changed `Interest` goes to the workers.
fn key(
    state: &mut AppState,
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    panes: &[Box<dyn Pane>],
    actions: &[Box<dyn Action>],
    event: &KeyEvent,
) -> Vec<Cmd> {
    if event.kind == KeyEventKind::Release {
        return Vec::new();
    }
    if state.data.sync.failure.take().is_some() {
        state.bump();
    }
    if state
        .data
        .merge
        .as_ref()
        .is_some_and(|merge| merge.progress != MergeProgress::Pending)
    {
        state.data.merge = None;
        state.bump();
    }
    let chord = keymap::chord(event);
    let control = chord.mods.contains(KeyModifiers::CONTROL);
    let quits = keymap.matches("quit", chord);
    if quits && control {
        return quit();
    }
    if state.ui.overlays.iter().any(|overlay| !overlay.closing) {
        return overlay_key(state, keymap, tabs, panes, actions, chord);
    }
    if quits && !state.ui.filter_edit {
        return quit();
    }
    if state.ui.filter_edit {
        return filter_key(state, tabs, chord);
    }
    if keymap.matches("refresh", chord) {
        let mut cmds = refresh(state, tabs);
        cmds.extend(after_change(state, tabs));
        return cmds;
    }
    if let Some(to) = tab_target(keymap, chord, state.ui.active, tabs.len()) {
        switch_tab(state, to);
        return after_change(state, tabs);
    }
    if let Some(motion) = motion(keymap, chord) {
        navigate(state, tabs, motion);
        return after_change(state, tabs);
    }
    if keymap.matches("filter", chord) {
        state.ui.filter_edit = true;
        state.bump();
        return Vec::new();
    }
    if keymap.matches("authors", chord) {
        open_authors(state, tabs);
        return Vec::new();
    }
    if keymap.matches("column_headers", chord) {
        return toggle_column_headers(state, tabs);
    }
    if keymap.matches("open", chord) {
        return open_selected(state, tabs, panes);
    }
    if keymap.matches("help", chord) {
        let help = help_model(keymap, tabs, actions);
        open_overlay(
            state,
            OverlayKind::Help,
            OverlayState::Help { help, scroll: 0 },
        );
        return Vec::new();
    }
    if let Some(cmds) = action_key(state, keymap, tabs, panes, actions, chord) {
        return cmds;
    }
    let index = state.ui.active;
    let areas = frame::areas(state);
    let (Some(tab), Some(ts)) = (tabs.get(index), state.ui.tabs.get_mut(index)) else {
        return Vec::new();
    };
    let cx = KeyCx {
        data: &state.data,
        target: None,
        areas: &areas,
        keys: keymap,
    };
    let Some(handled) = tab.handle_key(chord, &cx, ts) else {
        return Vec::new();
    };
    if handled.changed {
        state.bump();
    }
    state.ui.ui_events.extend(handled.events);
    let mut cmds = handled.cmds;
    cmds.extend(after_change(state, tabs));
    cmds
}

/// An action key on the cursor row. `None` when no action has the key,
/// when the row has no target, or when the action is hidden for it; the
/// key then goes to the tab.
fn action_key(
    state: &mut AppState,
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    panes: &[Box<dyn Pane>],
    actions: &[Box<dyn Action>],
    chord: KeyChord,
) -> Option<Vec<Cmd>> {
    actions
        .iter()
        .find(|action| keymap.matches(action.id(), chord))?;
    let model = frame::active_list(state, tabs);
    let item = &model.rows.get(model.cursor)?.item;
    let target = row_target(&state.data, item)?;
    run_action_key(state, keymap, panes, actions, target, chord)
}

/// An action key on `target`: `applies`, then `prepare`, then the route
/// of the preparation. `None` when no action has the key, or when the
/// action is hidden for the target.
fn run_action_key(
    state: &mut AppState,
    keymap: &Keymap,
    panes: &[Box<dyn Pane>],
    actions: &[Box<dyn Action>],
    target: Target,
    chord: KeyChord,
) -> Option<Vec<Cmd>> {
    let action = actions
        .iter()
        .find(|action| keymap.matches(action.id(), chord))?;
    let id = action.id();
    let selection = Selection {
        target,
        me: state.data.session.login.clone().unwrap_or_default(),
        action_config: state.data.action_config.get(id).cloned(),
    };
    match action.applies(&selection) {
        Applicability::Enabled => {}
        Applicability::Hidden => return None,
        Applicability::Disabled(reason) => {
            state.message(reason, CellStyle::Warn, INFO_TTL);
            return Some(Vec::new());
        }
    }
    let cmds = match action.prepare(&selection) {
        Prepare::Run => vec![Cmd::RunAction {
            id,
            selection: Box::new(selection),
            input: ActionInput::None,
        }],
        Prepare::Confirm(spec) => {
            let action = ActionDialog {
                id,
                selection: Box::new(selection),
            };
            open_overlay(
                state,
                OverlayKind::Confirm,
                OverlayState::Confirm { action, spec },
            );
            Vec::new()
        }
        Prepare::Prompt(spec) => {
            let action = ActionDialog {
                id,
                selection: Box::new(selection),
            };
            let prompt = OverlayState::Prompt {
                action,
                spec,
                text: String::new(),
            };
            open_overlay(state, OverlayKind::Prompt, prompt);
            Vec::new()
        }
        Prepare::Terminal(request) => {
            let key = selection.target.key();
            if state.data.panes.contains_key(&(id, key.clone())) {
                return Some(vec![Cmd::FocusPane { id, key }]);
            }
            vec![start_pane(state, id, key, request)]
        }
        Prepare::Command(cmd) => vec![cmd],
        Prepare::OpenPopup(pane) => open_popup(state, panes, &selection.target, Some(pane)),
    };
    Some(cmds)
}

/// A key for the prompt popup on top of the overlay stack: characters
/// and backspace edit the text, Enter runs the action with it, Esc
/// closes the popup. `None` when the top overlay is not a prompt.
fn prompt_key(state: &mut AppState, chord: KeyChord) -> Option<Vec<Cmd>> {
    let overlay = state.ui.overlays.iter_mut().rev().find(|o| !o.closing)?;
    let OverlayState::Prompt { action, text, .. } = &mut overlay.state else {
        return None;
    };
    let cmd = match chord.code {
        KeyCode::Esc => None,
        KeyCode::Enter if action.id == AGENT_ID => {
            let (selection, question) = (action.selection.clone(), text.trim().to_owned());
            close_overlay(state);
            return Some(ask(state, &selection, question));
        }
        KeyCode::Enter => Some(Cmd::RunAction {
            id: action.id,
            selection: action.selection.clone(),
            input: ActionInput::Text(text.clone()),
        }),
        KeyCode::Backspace => {
            text.pop();
            state.bump();
            return Some(Vec::new());
        }
        KeyCode::Char(c)
            if !chord
                .mods
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            text.push(c);
            state.bump();
            return Some(Vec::new());
        }
        _ => return Some(Vec::new()),
    };
    close_overlay(state);
    Some(cmd.into_iter().collect())
}

/// Opens the author picker of the active tab, with the cursor on the
/// current author. A tab without authors shows a message.
fn open_authors(state: &mut AppState, tabs: &[Box<dyn Tab>]) {
    let index = state.ui.active;
    let Some(tab) = tabs.get(index) else {
        return;
    };
    let areas = frame::areas(state);
    let cx = KeyCx {
        data: &state.data,
        target: None,
        areas: &areas,
        keys: &keymap::DEFAULT,
    };
    let authors = tab.authors(&cx);
    if authors.is_empty() {
        state.message("no authors to pick here", CellStyle::Dim, INFO_TTL);
        return;
    }
    let current = state.ui.tabs.get(index).and_then(|ts| ts.author.as_ref());
    let cursor = current
        .and_then(|author| authors.iter().position(|a| a == author))
        .map_or(0, |i| i + 1);
    open_overlay(
        state,
        OverlayKind::Picker,
        OverlayState::Authors(AuthorPicker {
            tab: index,
            authors,
            cursor,
        }),
    );
}

/// The pane bookkeeping of a terminal command: the pane of `(id, key)`
/// is `Starting` until `Msg::PaneStarted`.
fn start_pane(
    state: &mut AppState,
    id: &'static str,
    key: TargetKey,
    request: LaunchRequest,
) -> Cmd {
    state
        .data
        .panes
        .insert((id, key.clone()), PaneHandleState::Starting);
    state.bump();
    Cmd::RunTerminal { id, key, request }
}

/// The action id of the detail pane.
const DETAIL_ID: &str = "detail";

/// Opens the pull request of the cursor row: in the detail pane when
/// `run` gave its command, else in the sidebar. A row that is not a pull
/// request opens nothing.
fn open_selected(state: &mut AppState, tabs: &[Box<dyn Tab>], panes: &[Box<dyn Pane>]) -> Vec<Cmd> {
    let model = frame::active_list(state, tabs);
    let Some(RowItem::Pr(pr)) = model.rows.get(model.cursor).map(|row| &row.item) else {
        return Vec::new();
    };
    let Some(target) = pr_target(&state.data, &pr.key) else {
        return Vec::new();
    };
    if state.data.detail.is_some() {
        return open_pane(state, &target);
    }
    open_popup(state, panes, &target, None)
}

/// Opens `chaoshub pr OWNER/NAME NUMBER` for `target` in the detail
/// pane. A second open of the same pull request focuses its pane. One
/// detail pane is open at a time, so the pane of another pull request
/// closes.
fn open_pane(state: &mut AppState, target: &Target) -> Vec<Cmd> {
    let Some(detail) = state.data.detail.clone() else {
        return Vec::new();
    };
    let Target::Pr { pr, repo, .. } = target;
    let key = target.key();
    if state.data.panes.contains_key(&(DETAIL_ID, key.clone())) {
        return vec![Cmd::FocusPane { id: DETAIL_ID, key }];
    }
    let mut cmds: Vec<Cmd> = state
        .data
        .panes
        .keys()
        .filter(|(id, other)| *id == DETAIL_ID && *other != key)
        .map(|(_, other)| Cmd::CancelAction {
            id: DETAIL_ID,
            key: other.clone(),
        })
        .collect();
    let PrKey {
        repo: RepoKey { owner, name },
        number,
    } = &pr.key;
    let mut argv = detail.argv;
    argv.extend([
        "pr".to_owned(),
        format!("{owner}/{name}"),
        number.to_string(),
    ]);
    let request = LaunchRequest {
        argv,
        cwd: repo.path.clone().unwrap_or(detail.cwd),
        env: Vec::new(),
        label: format!(
            "chaoshub-detail-{owner}-{name}-{number}-{:04x}",
            rand::random::<u16>()
        ),
        zoom: false,
        split: detail.split,
        ratio: detail.ratio,
    };
    cmds.push(start_pane(state, DETAIL_ID, key, request));
    cmds
}

/// Opens the pull request popup for `target`, on the pane with the id
/// `pane` when it applies, else on the first pane that applies. When the
/// popup of `target` is on top already, only the pane changes. For a
/// repo with a path, it loads the pr-agent results and the codeview
/// comments.
fn open_popup(
    state: &mut AppState,
    panes: &[Box<dyn Pane>],
    target: &Target,
    pane: Option<&str>,
) -> Vec<Cmd> {
    let Target::Pr { pr, repo, .. } = target;
    let order = applicable(panes, target);
    let active = pane
        .and_then(|id| order.iter().copied().find(|&i| panes[i].id() == id))
        .or_else(|| order.first().copied())
        .unwrap_or(0);
    let top = state.ui.overlays.iter_mut().rev().find(|o| !o.closing);
    match top {
        Some(Overlay {
            state: OverlayState::Popup(popup),
            ..
        }) if popup.key == pr.key => {
            let from = popup.active;
            popup.active = active;
            if from != active {
                state
                    .ui
                    .ui_events
                    .push(UiEvent::PaneTabChanged { from, to: active });
            }
            state.bump();
        }
        _ => open_overlay(
            state,
            OverlayKind::Popup,
            OverlayState::Popup(PopupState {
                key: pr.key.clone(),
                active,
                panes: panes.iter().map(|pane| pane.init_state()).collect(),
            }),
        ),
    }
    if repo.path.is_some() {
        vec![
            Cmd::LoadAgent(pr.key.clone()),
            Cmd::LoadCodeview(pr.key.clone()),
        ]
    } else {
        Vec::new()
    }
}

/// The action id of the review in codeview.
const REVIEW_ID: &str = "review";

/// The codeview comments of `key` from the comment files. A change
/// redraws.
fn codeview_loaded(
    state: &mut AppState,
    key: PrKey,
    comments: Result<crate::domain::codeview::Comments, String>,
) -> Vec<Cmd> {
    if state.data.codeview.get(&key) != Some(&comments) {
        state.data.codeview.insert(key, comments);
        state.bump();
    }
    Vec::new()
}

/// Opens the confirm popup of the action `id` on the pull request `key`.
fn open_confirm(state: &mut AppState, key: &PrKey, id: &'static str, spec: ConfirmSpec) {
    let Some(target) = pr_target(&state.data, key) else {
        return;
    };
    let action = ActionDialog {
        id,
        selection: Box::new(Selection {
            target,
            me: state.data.session.login.clone().unwrap_or_default(),
            action_config: state.data.action_config.get(id).cloned(),
        }),
    };
    open_overlay(
        state,
        OverlayKind::Confirm,
        OverlayState::Confirm { action, spec },
    );
}

/// The action id of the agent action. Its prompt asks pr-agent.
const AGENT_ID: &str = crate::agent::ID;

/// The commands of a pane, with each `Cmd::RunAgent` through
/// `start_agent`.
fn pane_cmds(state: &mut AppState, cmds: Vec<Cmd>) -> Vec<Cmd> {
    let mut out = Vec::new();
    for cmd in cmds {
        match cmd {
            Cmd::RunAgent(job) => out.extend(start_agent(state, job)),
            cmd => out.push(cmd),
        }
    }
    out
}

/// Starts a pr-agent run, unless the same tool runs for the same pull
/// request: then the status line says so. An `Ask` without a question
/// opens the question prompt first.
fn start_agent(state: &mut AppState, job: AgentJob) -> Vec<Cmd> {
    if let Tool::Ask(question) = &job.tool
        && question.trim().is_empty()
    {
        open_ask(state, &job.key);
        return Vec::new();
    }
    let kind = job.tool.kind();
    if !state.data.agent_jobs.insert((job.key.clone(), kind)) {
        state.message(
            format!("{} runs already", kind.name()),
            CellStyle::Warn,
            INFO_TTL,
        );
        return Vec::new();
    }
    state.bump();
    vec![Cmd::RunAgent(job)]
}

/// Opens the prompt for a question to pr-agent about `key`.
fn open_ask(state: &mut AppState, key: &PrKey) {
    let Some(target) = pr_target(&state.data, key) else {
        return;
    };
    let selection = Selection {
        target,
        me: state.data.session.login.clone().unwrap_or_default(),
        action_config: None,
    };
    let prompt = OverlayState::Prompt {
        action: ActionDialog {
            id: AGENT_ID,
            selection: Box::new(selection),
        },
        spec: PromptSpec {
            title: format!("ask pr-agent about #{}", key.number),
            placeholder: "a question about the pull request".to_owned(),
        },
        text: String::new(),
    };
    open_overlay(state, OverlayKind::Prompt, prompt);
}

/// Starts an ask run with `question` on the pull request of
/// `selection`. An empty question starts nothing.
fn ask(state: &mut AppState, selection: &Selection, question: String) -> Vec<Cmd> {
    let Target::Pr { pr, repo, .. } = &selection.target;
    let Some(repo_path) = repo.path.clone() else {
        return Vec::new();
    };
    if question.is_empty() {
        return Vec::new();
    }
    let job = AgentJob {
        key: pr.key.clone(),
        head_sha: crate::app::panes::agent::head(&state.data, pr),
        repo_path,
        tool: Tool::Ask(question),
    };
    start_agent(state, job)
}

/// A message of the pr-agent runs.
fn agent_msg(state: &mut AppState, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::AgentStarted { key, kind } => {
            state.data.agent_jobs.insert((key, kind));
            state.bump();
            Vec::new()
        }
        Msg::AgentFinished { key, kind, result } => agent_finished(state, key, kind, result),
        Msg::AgentLoaded { key, results } => {
            agent_loaded(state, key, results);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// The end of a pr-agent run: the job leaves the job set, the record or
/// the error goes to the results, and the status line says
/// `review done (12k tokens)` or the error. The files load again.
fn agent_finished(
    state: &mut AppState,
    key: PrKey,
    kind: ToolKind,
    result: Result<Record, String>,
) -> Vec<Cmd> {
    state.data.agent_jobs.remove(&(key.clone(), kind));
    let results = state.data.agent.entry(key.clone()).or_default();
    let name = kind.name();
    let (text, style, ttl) = match result {
        Ok(record) => {
            let text = match record.total_tokens() {
                Some(tokens) => format!("{name} done ({} tokens)", short_tokens(tokens)),
                None => format!("{name} done"),
            };
            results.errors.remove(&kind);
            results.records.insert(kind, record);
            (text, CellStyle::Success, INFO_TTL)
        }
        Err(err) => {
            results.errors.insert(kind, err.clone());
            (
                format!("{name} failed: {err}"),
                CellStyle::Danger,
                ERROR_TTL,
            )
        }
    };
    state.message(text, style, ttl);
    vec![Cmd::LoadAgent(key)]
}

/// The result files of `key` from disk. The records replace the old
/// ones. The errors of failed runs stay until a run succeeds.
fn agent_loaded(state: &mut AppState, key: PrKey, loaded: AgentResults) {
    let results = state.data.agent.entry(key).or_default();
    if results.records == loaded.records && loaded.errors.is_empty() {
        return;
    }
    results.records = loaded.records;
    results.errors.extend(loaded.errors);
    state.bump();
}

/// A key in the pull request popup: a pane key or `next_tab` selects a
/// pane, the navigation keys move the cursor of the pane (or scroll a
/// pane of lines), and the other keys go to the active pane. `None` when
/// nothing handled the key.
fn popup_key(
    data: &AppData,
    areas: &Areas,
    keymap: &Keymap,
    panes: &[Box<dyn Pane>],
    popup: &mut PopupState,
    chord: KeyChord,
) -> Option<Handled> {
    let target = pr_target(data, &popup.key)?;
    let order = applicable(panes, &target);
    let changed = Handled {
        changed: true,
        ..Handled::default()
    };
    let select = |popup: &mut PopupState, to: usize| {
        let from = popup.active;
        popup.active = to;
        Handled {
            events: if from == to {
                Vec::new()
            } else {
                vec![UiEvent::PaneTabChanged { from, to }]
            },
            ..changed.clone()
        }
    };
    if let KeyCode::Char(c) = chord.code
        && chord.mods.is_empty()
        && let Some(&index) = order.iter().find(|&&i| panes[i].key() == c)
    {
        return Some(select(popup, index));
    }
    if keymap.matches("next_tab", chord) && !order.is_empty() {
        let at = order
            .iter()
            .position(|&i| i == popup.active)
            .map_or(0, |p| p + 1);
        return Some(select(popup, order[at % order.len()]));
    }
    in_pane(data, areas, keymap, panes, popup, |pane, cx, ps, size| {
        if let Some(motion) = motion(keymap, chord) {
            move_pane(pane, cx, ps, size, motion);
            return Some(changed);
        }
        pane.handle_key(chord, cx, ps)
    })
    .flatten()
}

/// Calls `f` with the active pane of `popup`, its key context, its state
/// and the size of the pane body. `None` when the pull request or the
/// pane is gone.
fn in_pane<R>(
    data: &AppData,
    areas: &Areas,
    keymap: &Keymap,
    panes: &[Box<dyn Pane>],
    popup: &mut PopupState,
    f: impl FnOnce(&dyn Pane, &KeyCx<'_>, &mut PaneState, Size) -> R,
) -> Option<R> {
    let target = pr_target(data, &popup.key)?;
    let pane = panes.get(popup.active)?;
    let ps = popup.panes.get_mut(popup.active)?;
    let tabs = frame::pane_tabs(panes, &applicable(panes, &target));
    let size = frame::pane_size(areas, &tabs);
    let cx = KeyCx {
        data,
        target: Some(&target),
        areas,
        keys: keymap,
    };
    Some(f(pane.as_ref(), &cx, ps, size))
}

/// Moves the cursor of a pane one step, or the scroll of a pane of
/// lines.
fn move_pane(pane: &dyn Pane, cx: &KeyCx<'_>, ps: &mut PaneState, size: Size, motion: Motion) {
    let model = pane.model(cx, ps, size);
    if let PaneModel::Lines { lines, .. } = &model {
        ps.scroll = move_in(ps.scroll, display_rows(lines, size.width), motion);
    } else {
        let count = items(&model);
        ps.cursor = move_in(ps.cursor, count, motion);
        ps.scroll = nav::keep_visible(count, ps.cursor, ps.scroll, usize::from(size.height));
    }
}

/// Pushes an overlay and starts its slide in.
fn open_overlay(state: &mut AppState, kind: OverlayKind, overlay: OverlayState) {
    state.ui.overlays.push(Overlay {
        kind,
        closing: false,
        state: overlay,
    });
    state.ui.ui_events.push(UiEvent::OverlayOpened(kind));
    state.bump();
}

/// The index of the top overlay that is not closing.
fn top_overlay(state: &AppState) -> Option<usize> {
    state.ui.overlays.iter().rposition(|o| !o.closing)
}

/// True when the top overlay that is not closing is the pinned popup:
/// the overlay at index 0 in the pinned mode.
fn pinned_on_top(state: &AppState) -> bool {
    state.ui.pinned.is_some() && top_overlay(state) == Some(0)
}

/// The `close_overlay` key: it quits when the pinned popup is on top,
/// else it closes the top overlay that is not closing.
fn close_top(state: &mut AppState) -> Vec<Cmd> {
    if pinned_on_top(state) {
        return quit();
    }
    close_overlay(state);
    Vec::new()
}

/// Starts the slide out of the top overlay that is not closing. It stays
/// in the stack until `Msg::TransitionsDone`.
fn close_overlay(state: &mut AppState) {
    let Some(overlay) = state.ui.overlays.iter_mut().rev().find(|o| !o.closing) else {
        return;
    };
    overlay.closing = true;
    let kind = overlay.kind;
    state.ui.ui_events.push(UiEvent::OverlayClosed(kind));
    state.bump();
}

/// A key for the top overlay that is not closing. In the pull request
/// popup, a key that no pane handles goes to the actions, with the pull
/// request of the popup as the target.
fn overlay_key(
    state: &mut AppState,
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    panes: &[Box<dyn Pane>],
    actions: &[Box<dyn Action>],
    chord: KeyChord,
) -> Vec<Cmd> {
    if let Some(cmds) = prompt_key(state, chord) {
        return cmds;
    }
    if keymap.matches("close_overlay", chord) {
        return close_top(state);
    }
    let areas = frame::areas(state);
    let Some(overlay) = state.ui.overlays.iter_mut().rev().find(|o| !o.closing) else {
        return Vec::new();
    };
    if matches!(overlay.state, OverlayState::Help { .. }) && keymap.matches("help", chord) {
        close_overlay(state);
        return Vec::new();
    }
    match &mut overlay.state {
        OverlayState::Popup(popup) => {
            let Some(handled) = popup_key(&state.data, &areas, keymap, panes, popup, chord) else {
                if keymap.matches("refresh", chord) {
                    return refresh_popup(&state.data, &popup.key);
                }
                if keymap.matches("help", chord) {
                    let help = help_model(keymap, tabs, actions);
                    open_overlay(
                        state,
                        OverlayKind::Help,
                        OverlayState::Help { help, scroll: 0 },
                    );
                    return Vec::new();
                }
                let Some(target) = pr_target(&state.data, &popup.key) else {
                    return Vec::new();
                };
                return run_action_key(state, keymap, panes, actions, target, chord)
                    .unwrap_or_default();
            };
            let key = popup.key.clone();
            if handled.changed {
                state.bump();
            }
            state.ui.ui_events.extend(handled.events);
            if let Some((id, spec)) = handled.confirm {
                open_confirm(state, &key, id, spec);
            }
            pane_cmds(state, handled.cmds)
        }
        OverlayState::Authors(picker) => {
            if let Some(motion) = motion(keymap, chord) {
                picker.cursor = move_in(picker.cursor, picker.authors.len() + 1, motion);
                state.bump();
                return Vec::new();
            }
            if !keymap.matches("open", chord) {
                return Vec::new();
            }
            let (tab, author) = (picker.tab, picker.chosen());
            pick_author(state, tabs, tab, author)
        }
        OverlayState::Confirm { action, .. } => {
            if keymap.matches("cancel", chord) {
                close_overlay(state);
                return Vec::new();
            }
            if !keymap.matches("confirm", chord) && !keymap.matches("open", chord) {
                return Vec::new();
            }
            let cmd = Cmd::RunAction {
                id: action.id,
                selection: action.selection.clone(),
                input: ActionInput::Confirmed,
            };
            close_overlay(state);
            vec![cmd]
        }
        OverlayState::Help { .. } | OverlayState::Output { .. } => {
            if let Some(motion) = motion(keymap, chord) {
                scroll_text(&mut overlay.state, motion);
                state.bump();
            }
            Vec::new()
        }
        OverlayState::Prompt { .. } => Vec::new(),
    }
}

/// The `refresh` key in the popup of `key`: a load by number when the
/// pull request is not loaded, else a refresh of its stack group, a load
/// of its codeview comments and a scan of its repo.
fn refresh_popup(data: &AppData, key: &PrKey) -> Vec<Cmd> {
    if !data.prs.contains_key(key) {
        return vec![Cmd::LoadPr(key.clone())];
    }
    vec![
        Cmd::RefreshPr(stack_group_refs(data, key)),
        Cmd::LoadCodeview(key.clone()),
        Cmd::ScanJj(key.repo.clone()),
    ]
}

/// Closes the author picker and filters the tab at `tab` by `author`.
/// `None` shows every author.
fn pick_author(
    state: &mut AppState,
    tabs: &[Box<dyn Tab>],
    tab: usize,
    author: Option<String>,
) -> Vec<Cmd> {
    close_overlay(state);
    if let Some(ts) = state.ui.tabs.get_mut(tab) {
        ts.author.clone_from(&author);
    }
    let mut cmds: Vec<Cmd> = tabs
        .get(tab)
        .map(|t| persist(AUTHOR_KEY, t.id(), author.unwrap_or_default()))
        .into_iter()
        .collect();
    cmds.extend(after_change(state, tabs));
    cmds
}

/// Moves the scroll of a help or an output overlay over its lines.
/// False for the other overlays.
fn scroll_text(overlay: &mut OverlayState, motion: Motion) -> bool {
    match overlay {
        OverlayState::Help { help, scroll } => {
            *scroll = move_in(*scroll, help.line_count(), motion);
        }
        OverlayState::Output { lines, scroll, .. } => {
            *scroll = move_in(*scroll, lines.len(), motion);
        }
        OverlayState::Popup(_)
        | OverlayState::Authors(_)
        | OverlayState::Confirm { .. }
        | OverlayState::Prompt { .. } => return false,
    }
    true
}

/// The rows that one wheel step moves.
const WHEEL_ROWS: usize = 3;

/// A mouse event: the wheel moves the top overlay, or the list when no
/// overlay is open, by `WHEEL_ROWS`. A click acts on its hit.
fn mouse(
    state: &mut AppState,
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    panes: &[Box<dyn Pane>],
    actions: &[Box<dyn Action>],
    kind: MouseKind,
    hit: Option<Hit>,
) -> Vec<Cmd> {
    match (kind, hit) {
        (MouseKind::ScrollDown, _) => wheel(state, keymap, tabs, panes, Motion::Down),
        (MouseKind::ScrollUp, _) => wheel(state, keymap, tabs, panes, Motion::Up),
        (MouseKind::Click, Some(hit)) => click(state, keymap, tabs, panes, actions, hit),
        (MouseKind::Click, None) => Vec::new(),
    }
}

/// One wheel step: `WHEEL_ROWS` moves through the code of the motion
/// keys. With no overlay, the list cursor moves. In the sidebar, the
/// active pane moves. In the help or the output overlay, the text
/// scrolls.
fn wheel(
    state: &mut AppState,
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    panes: &[Box<dyn Pane>],
    motion: Motion,
) -> Vec<Cmd> {
    let areas = frame::areas(state);
    let Some(overlay) = state.ui.overlays.iter_mut().rev().find(|o| !o.closing) else {
        for _ in 0..WHEEL_ROWS {
            navigate(state, tabs, motion);
        }
        return after_change(state, tabs);
    };
    let moved = match &mut overlay.state {
        OverlayState::Popup(popup) => (0..WHEEL_ROWS).all(|_| {
            in_pane(
                &state.data,
                &areas,
                keymap,
                panes,
                popup,
                |pane, cx, ps, size| {
                    move_pane(pane, cx, ps, size, motion);
                },
            )
            .is_some()
        }),
        text => (0..WHEEL_ROWS).all(|_| scroll_text(text, motion)),
    };
    if moved {
        state.bump();
    }
    Vec::new()
}

/// A click on `hit`.
fn click(
    state: &mut AppState,
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    panes: &[Box<dyn Pane>],
    actions: &[Box<dyn Action>],
    hit: Hit,
) -> Vec<Cmd> {
    match hit {
        Hit::Tab(index) if index < tabs.len() => {
            switch_tab(state, index);
            after_change(state, tabs)
        }
        Hit::Row(index) => click_row(state, tabs, panes, index),
        Hit::Key(chord) => {
            let event = KeyEvent::new(chord.code, chord.mods);
            key(state, keymap, tabs, panes, actions, &event)
        }
        Hit::PaneTab(index) => {
            click_pane_tab(state, panes, index);
            Vec::new()
        }
        Hit::PaneRow(index) => click_pane_row(state, keymap, panes, index),
        Hit::Item(index) => click_item(state, tabs, index),
        Hit::Outside => {
            if !pinned_on_top(state) {
                close_overlay(state);
            }
            Vec::new()
        }
        Hit::Tab(_) | Hit::Inside => Vec::new(),
    }
}

/// A click on the list row at `index`: the cursor moves to it when it is
/// selectable. A click on the cursor row opens the sidebar.
fn click_row(
    state: &mut AppState,
    tabs: &[Box<dyn Tab>],
    panes: &[Box<dyn Pane>],
    index: usize,
) -> Vec<Cmd> {
    if state.ui.overlays.iter().any(|o| !o.closing) {
        return Vec::new();
    }
    let model = frame::active_list(state, tabs);
    let Some(row) = model.rows.get(index).filter(|row| nav::selectable(row)) else {
        return Vec::new();
    };
    if index == model.cursor {
        return open_selected(state, tabs, panes);
    }
    let height = usize::from(frame::list_size(state).height);
    let Some(ts) = state.ui.tabs.get_mut(state.ui.active) else {
        return Vec::new();
    };
    ts.cursor = index;
    ts.scroll = nav::keep_visible(model.rows.len(), index, model.scroll, height);
    ts.selected = Some(row.item.clone());
    state.bump();
    after_change(state, tabs)
}

/// A click on the pane tab at `index` of the sidebar tabs: that pane
/// becomes active. The tabs are the panes that apply, in registry order.
fn click_pane_tab(state: &mut AppState, panes: &[Box<dyn Pane>], index: usize) {
    let Some(Overlay {
        state: OverlayState::Popup(popup),
        ..
    }) = state.ui.overlays.iter_mut().rev().find(|o| !o.closing)
    else {
        return;
    };
    let Some(target) = pr_target(&state.data, &popup.key) else {
        return;
    };
    if let Some(&active) = applicable(panes, &target).get(index) {
        let from = popup.active;
        popup.active = active;
        if from != active {
            state
                .ui
                .ui_events
                .push(UiEvent::PaneTabChanged { from, to: active });
        }
        state.bump();
    }
}

/// A click on the row or the node at `index` of the active pane: the
/// pane cursor moves to it. A click on the cursor node of a tree runs
/// the `open` key of the pane, which expands or collapses it.
fn click_pane_row(
    state: &mut AppState,
    keymap: &Keymap,
    panes: &[Box<dyn Pane>],
    index: usize,
) -> Vec<Cmd> {
    let areas = frame::areas(state);
    let Some(Overlay {
        state: OverlayState::Popup(popup),
        ..
    }) = state.ui.overlays.iter_mut().rev().find(|o| !o.closing)
    else {
        return Vec::new();
    };
    let handled = in_pane(
        &state.data,
        &areas,
        keymap,
        panes,
        popup,
        |pane, cx, ps, size| {
            let model = pane.model(cx, ps, size);
            let count = items(&model);
            if index >= count || matches!(model, PaneModel::Lines { .. }) {
                return None;
            }
            if index == ps.cursor {
                if !matches!(model, PaneModel::Tree { .. }) {
                    return None;
                }
                let open = cx.keys.chords("open").first().copied()?;
                return pane.handle_key(open, cx, ps);
            }
            ps.cursor = index;
            ps.scroll = nav::keep_visible(count, index, ps.scroll, usize::from(size.height));
            Some(Handled {
                changed: true,
                ..Handled::default()
            })
        },
    )
    .flatten();
    let Some(handled) = handled else {
        return Vec::new();
    };
    if handled.changed {
        state.bump();
    }
    state.ui.ui_events.extend(handled.events);
    handled.cmds
}

/// A click on the item at `index` of the author picker: it picks that
/// item, as Enter does on the cursor item.
fn click_item(state: &mut AppState, tabs: &[Box<dyn Tab>], index: usize) -> Vec<Cmd> {
    let Some(Overlay {
        state: OverlayState::Authors(picker),
        ..
    }) = state.ui.overlays.iter_mut().rev().find(|o| !o.closing)
    else {
        return Vec::new();
    };
    if index > picker.authors.len() {
        return Vec::new();
    }
    picker.cursor = index;
    let (tab, author) = (picker.tab, picker.chosen());
    pick_author(state, tabs, tab, author)
}

/// The cursor in a list of `len` items after a move.
fn move_in(cursor: usize, len: usize, motion: Motion) -> usize {
    const PAGE: usize = 5;
    let last = len.saturating_sub(1);
    match motion {
        Motion::Down => cursor + 1,
        Motion::Up => cursor.saturating_sub(1),
        Motion::Top => 0,
        Motion::Bottom => last,
        Motion::PageDown => cursor + PAGE,
        Motion::PageUp => cursor.saturating_sub(PAGE),
    }
    .min(last)
}

/// A key while the user types the filter: characters and backspace edit
/// it, Enter keeps it, Esc clears it, the arrows move the cursor.
fn filter_key(state: &mut AppState, tabs: &[Box<dyn Tab>], chord: KeyChord) -> Vec<Cmd> {
    let (Some(tab), Some(ts)) = (
        tabs.get(state.ui.active),
        state.ui.tabs.get_mut(state.ui.active),
    ) else {
        return Vec::new();
    };
    let mut cmds = Vec::new();
    match chord.code {
        KeyCode::Esc => {
            ts.filter.clear();
            state.ui.filter_edit = false;
            cmds.push(persist(FILTER_KEY, tab.id(), String::new()));
        }
        KeyCode::Enter => {
            state.ui.filter_edit = false;
            cmds.push(persist(FILTER_KEY, tab.id(), ts.filter.clone()));
        }
        KeyCode::Backspace => {
            ts.filter.pop();
        }
        KeyCode::Char(c)
            if !chord
                .mods
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            ts.filter.push(c);
        }
        KeyCode::Down => navigate(state, tabs, Motion::Down),
        KeyCode::Up => navigate(state, tabs, Motion::Up),
        _ => return Vec::new(),
    }
    state.bump();
    cmds.extend(after_change(state, tabs));
    cmds
}

/// Shows or hides the header row of the list for the session, and
/// writes the switch into `ui_state`. The cursor stays visible in the
/// new list height.
fn toggle_column_headers(state: &mut AppState, tabs: &[Box<dyn Tab>]) -> Vec<Cmd> {
    state.ui.column_headers = !state.ui.column_headers;
    state.bump();
    let height = usize::from(frame::list_size(state).height);
    let model = frame::active_list(state, tabs);
    if let Some(ts) = state.ui.tabs.get_mut(state.ui.active) {
        ts.scroll = nav::keep_visible(model.rows.len(), model.cursor, model.scroll, height);
    }
    state.bump();
    let mut cmds = vec![Cmd::Persist(Persist::UiState {
        key: COLUMN_HEADERS_KEY.to_owned(),
        value: state.ui.column_headers.to_string(),
    })];
    cmds.extend(after_change(state, tabs));
    cmds
}

/// The write of one tab setting into `ui_state`. An empty value clears
/// it.
fn persist(kind: &str, tab: &str, value: String) -> Cmd {
    Cmd::Persist(Persist::UiState {
        key: ui_key(kind, tab),
        value,
    })
}

/// Scores the restored filters once before the first frame.
pub(crate) fn start(state: &mut AppState) {
    after_change(state, &TABS);
}

/// Pins the pull request `key`: it sets `UiState.pinned`, pushes the
/// popup of `key` without a `UiEvent` (the popup is in place from the
/// first frame), and runs `start` again, so the interest is the one of the
/// pinned pull request.
pub(crate) fn pin(state: &mut AppState, key: PrKey) {
    state.ui.pinned = Some(key.clone());
    state.ui.overlays.push(Overlay {
        kind: OverlayKind::Popup,
        closing: false,
        state: OverlayState::Popup(PopupState {
            key,
            active: 0,
            panes: PANES.iter().map(|pane| pane.init_state()).collect(),
        }),
    });
    state.bump();
    start(state);
}

/// After a change of the data or the UI state: the filters score again
/// where their text or the data moved, and a changed interest goes to the
/// workers.
fn after_change(state: &mut AppState, tabs: &[Box<dyn Tab>]) -> Vec<Cmd> {
    let areas = frame::areas(state);
    let cx = KeyCx {
        data: &state.data,
        target: None,
        areas: &areas,
        keys: &keymap::DEFAULT,
    };
    for (tab, ts) in tabs.iter().zip(state.ui.tabs.iter_mut()) {
        tab.rescore(&cx, ts);
    }
    refresh_interest(state, tabs).into_iter().collect()
}

/// The tab that a chord selects: `tab_1` to `tab_9` by registry order,
/// and `next_tab` cycles.
fn tab_target(keymap: &Keymap, chord: KeyChord, active: usize, count: usize) -> Option<usize> {
    if count > 0 && keymap.matches("next_tab", chord) {
        return Some((active + 1) % count);
    }
    (1..=9)
        .find(|n| keymap.matches(&format!("tab_{n}"), chord))
        .map(|n| n - 1)
        .filter(|&index| index < count)
}

/// Makes the tab at `to` active. `UiEvent::TabChanged` starts the slide
/// of the active tab in the tab bar.
fn switch_tab(state: &mut AppState, to: usize) {
    let from = state.ui.active;
    if to == from {
        return;
    }
    state.ui.active = to;
    state.ui.ui_events.push(UiEvent::TabChanged { from, to });
    state.bump();
}

/// The navigation binding of a chord, if any.
fn motion(keymap: &Keymap, chord: KeyChord) -> Option<Motion> {
    [
        ("down", Motion::Down),
        ("up", Motion::Up),
        ("top", Motion::Top),
        ("bottom", Motion::Bottom),
        ("page_down", Motion::PageDown),
        ("page_up", Motion::PageUp),
    ]
    .into_iter()
    .find(|(name, _)| keymap.matches(name, chord))
    .map(|(_, motion)| motion)
}

/// Moves the cursor of the active tab. A move that scrolls the list pushes
/// `UiEvent::ScrollTo`; a move inside the visible rows is immediate.
fn navigate(state: &mut AppState, tabs: &[Box<dyn Tab>], motion: Motion) {
    let Some(tab) = tabs.get(state.ui.active) else {
        return;
    };
    let height = usize::from(frame::list_size(state).height);
    let model = frame::active_list(state, tabs);
    let Some(cursor) = nav::step(&model.rows, model.cursor, motion, height) else {
        return;
    };
    let scroll = nav::keep_visible(model.rows.len(), cursor, model.scroll, height);
    let Some(ts) = state.ui.tabs.get_mut(state.ui.active) else {
        return;
    };
    ts.cursor = cursor;
    ts.scroll = scroll;
    ts.selected = model.rows.get(cursor).map(|row| row.item.clone());
    if cursor == model.cursor && scroll == model.scroll {
        return;
    }
    if scroll != model.scroll {
        state.ui.ui_events.push(UiEvent::ScrollTo {
            tab: tab.id(),
            from: model.scroll,
            to: scroll,
        });
    }
    state.bump();
}

/// `Cmd::SetInterest` when the interest of the active tab, or of the
/// pinned pull request, differs from the interest that the workers have.
fn refresh_interest(state: &mut AppState, tabs: &[Box<dyn Tab>]) -> Option<Cmd> {
    let interest = if let Some(key) = &state.ui.pinned {
        pinned_interest(&state.data, key)
    } else {
        let index = state.ui.active;
        let (tab, ts) = (tabs.get(index)?, state.ui.tabs.get(index)?);
        let areas = frame::areas(state);
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
            keys: &keymap::DEFAULT,
        };
        tab.interest(&cx, ts)
    };
    if state.ui.interest.as_ref() == Some(&interest) {
        return None;
    }
    state.ui.interest = Some(interest.clone());
    Some(Cmd::SetInterest(interest))
}

/// The interest of the pinned mode: the pinned pull request is selected
/// when it is loaded, and the other loaded entries of its stack are
/// visible. No scope and no jj data.
fn pinned_interest(data: &AppData, key: &PrKey) -> crate::bus::Interest {
    crate::bus::Interest {
        tab: "pr",
        scope: None,
        wants_local: false,
        selected: data.prs.get(key).map(pr_ref),
        visible: stack_group_refs(data, key)
            .into_iter()
            .filter(|other| other.key != *key)
            .collect(),
    }
}

/// The startup worker read the login and the repo of the current
/// directory. The session goes to the workers, both scopes sync, and every
/// repo with a path gets a jj scan.
fn startup_done(
    state: &mut AppState,
    tabs: &[Box<dyn Tab>],
    login: String,
    cwd_repo: Option<Repo>,
) -> Vec<Cmd> {
    let data = &mut state.data;
    data.session.login = Some(login);
    if let Some(cwd) = cwd_repo {
        match data
            .session
            .repos
            .iter_mut()
            .find(|repo| repo.key == cwd.key)
        {
            Some(repo) => {
                if repo.path.is_none() {
                    repo.path.clone_from(&cwd.path);
                }
            }
            None => data.session.repos.push(cwd.clone()),
        }
        let known = data
            .repos
            .entry(cwd.key.clone())
            .or_insert_with(|| cwd.clone());
        if known.path.is_none() {
            known.path = cwd.path;
        }
    }
    let mut cmds = vec![Cmd::SetSession(data.session.clone())];
    if let Some(key) = &state.ui.pinned {
        cmds.extend(pinned_startup(data, key));
        state.bump();
        return cmds;
    }
    let mut scopes: Vec<Scope> = Vec::new();
    for scope in tabs.iter().filter_map(|tab| tab.scope()) {
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    for scope in scopes {
        data.sync.pending.insert(scope);
        cmds.push(Cmd::SyncNow(scope));
    }
    cmds.extend(
        data.session
            .repos
            .iter()
            .filter(|repo| repo.path.is_some())
            .map(|repo| Cmd::ScanJj(repo.key.clone())),
    );
    state.bump();
    cmds
}

/// The startup commands of the pinned mode: a jj scan of the pinned repo
/// when it has a path, then a load of the pinned pull request when it is
/// not cached, else a refresh of it. No list sync.
fn pinned_startup(data: &AppData, key: &PrKey) -> Vec<Cmd> {
    let mut cmds: Vec<Cmd> = data
        .session
        .repos
        .iter()
        .filter(|repo| repo.key == key.repo && repo.path.is_some())
        .map(|repo| Cmd::ScanJj(repo.key.clone()))
        .take(1)
        .collect();
    cmds.push(match data.prs.get(key) {
        Some(pr) => Cmd::RefreshPr(vec![pr_ref(pr)]),
        None => Cmd::LoadPr(key.clone()),
    });
    if data
        .session
        .repos
        .iter()
        .any(|repo| repo.key == key.repo && repo.path.is_some())
    {
        cmds.push(Cmd::LoadAgent(key.clone()));
        cmds.push(Cmd::LoadCodeview(key.clone()));
    }
    cmds
}

/// The sync worker started a sync of `scope`. It runs until its end or
/// a failure.
fn sync_started(state: &mut AppState, scope: Scope) -> Vec<Cmd> {
    if state.data.sync.running.insert(scope) {
        state.bump();
    }
    Vec::new()
}

/// The detail worker started to read the conversation of `key`. It runs
/// until `DetailLoaded`.
fn detail_loading(state: &mut AppState, key: PrKey) -> Vec<Cmd> {
    if state.data.details_loading.insert(key) {
        state.bump();
    }
    Vec::new()
}

/// The detail worker started to read the changed files of `key`. It runs
/// until `FilesLoaded`.
fn files_loading(state: &mut AppState, key: PrKey) -> Vec<Cmd> {
    if state.data.files_loading.insert(key) {
        state.bump();
    }
    Vec::new()
}

/// A page of a sync of one scope, or its end: the pull requests go into
/// the data under the freshness rule. After a complete full sync (`seen`
/// is `Some`), the open pull requests of the scope that the sync did not
/// return leave the data, under the gone rule. Only the end (`last`) sets
/// the sync time and ends a requested sync. Each repo of the result with
/// a local path gets a jj scan.
fn prs_synced(
    state: &mut AppState,
    scope: Scope,
    prs: Vec<PullRequest>,
    seen: Option<Vec<PrKey>>,
    started_at: DateTime<Utc>,
    last: bool,
) -> Vec<Cmd> {
    let repos: BTreeSet<_> = prs.iter().map(|pr| pr.key.repo.clone()).collect();
    let page: BTreeSet<PrKey> = prs.iter().map(|pr| pr.key.clone()).collect();
    let mut changed = false;
    for pr in prs {
        state.data.scopes.insert(pr.key.clone(), scope);
        changed |= merge_pr(&mut state.data, pr);
    }
    if let Some(seen) = seen {
        let seen: BTreeSet<PrKey> = seen.into_iter().chain(page).collect();
        let candidates = gone_candidates(&state.data, scope, &seen, started_at);
        tracing::debug!(
            candidates = candidates.len(),
            ?scope,
            "gone after a full sync"
        );
        for key in candidates {
            changed |= drop_gone(&mut state.data, &key, started_at);
        }
    }
    if last {
        state.data.synced.insert(scope, started_at);
        state.data.sync.pending.remove(&scope);
        state.data.sync.running.remove(&scope);
    }
    state.data.sync.failure = None;
    if changed {
        state.data.revision += 1;
    }
    state.bump();
    repos
        .into_iter()
        .filter(|key| {
            state
                .data
                .repos
                .get(key)
                .is_some_and(|repo| repo.path.is_some())
        })
        .map(Cmd::ScanJj)
        .collect()
}

/// A pull request that the sync worker read again by id. True when it
/// changed the data.
fn pr_refreshed(state: &mut AppState, pr: PullRequest) -> bool {
    let data = &mut state.data;
    if !data.scopes.contains_key(&pr.key) {
        let mine = data.session.login.as_deref() == Some(pr.head.value.author.as_str());
        let scope = if mine { Scope::Mine } else { Scope::Team };
        data.scopes.insert(pr.key.clone(), scope);
    }
    if !merge_pr(data, pr) {
        return false;
    }
    data.revision += 1;
    state.bump();
    true
}

/// A CI poll: the CI state of each polled pull request whose head is
/// still the polled head, and the checks of the selected pull request,
/// under the freshness rule. True when it changed the data.
fn ci_updated(
    state: &mut AppState,
    states: Vec<(PrKey, String, Observed<CiState>)>,
    selected_checks: Option<(PrKey, Observed<Vec<Check>>)>,
) -> bool {
    let prs = &mut state.data.prs;
    let mut changed = false;
    for (key, head_oid, ci) in states {
        match prs.get_mut(&key) {
            Some(pr) if pr.head.value.head_oid == head_oid => {
                changed |= apply(&mut pr.ci_state, ci, &key, "ci state");
            }
            _ => tracing::debug!(?key, "ci state for another head dropped"),
        }
    }
    if let Some((key, checks)) = selected_checks
        && let Some(pr) = prs.get_mut(&key)
    {
        changed |= apply(&mut pr.checks, checks, &key, "checks");
    }
    if changed {
        state.data.revision += 1;
        state.bump();
    }
    changed
}

/// What a row glow shows a change of: the CI state, and the checks and
/// the reviews cells of the list.
type GlowValues = (CiState, Cell, Cell);

/// The glow values of the pull requests in the window of the active
/// list, before a sync or a CI poll. Empty when `glow` is false: the
/// first sync after start makes every row new, so nothing glows.
fn glow_before(
    state: &mut AppState,
    tabs: &[Box<dyn Tab>],
    glow: bool,
) -> Vec<(PrKey, GlowValues)> {
    if !glow {
        return Vec::new();
    }
    let height = usize::from(frame::list_size(state).height);
    let model = frame::active_list(state, tabs);
    model
        .rows
        .iter()
        .skip(model.scroll)
        .take(height)
        .filter_map(|row| match &row.item {
            RowItem::Pr(pr) => state.data.prs.get(&pr.key),
            _ => None,
        })
        .map(|pr| (pr.key.clone(), pr_glow_values(pr)))
        .collect()
}

/// The glow values of one pull request.
fn pr_glow_values(pr: &PullRequest) -> GlowValues {
    (
        pr.ci_state.value,
        cells::checks(pr),
        cells::reviews(&pr.head.value),
    )
}

/// A `UiEvent::RowGlow` for each pull request of `before` whose glow
/// values changed.
fn push_glows(state: &mut AppState, before: &[(PrKey, GlowValues)]) {
    for (key, values) in before {
        if let Some(pr) = state.data.prs.get(key)
            && pr_glow_values(pr) != *values
        {
            state.ui.ui_events.push(UiEvent::RowGlow(key.clone()));
        }
    }
}

/// The conversation of a pull request, under the freshness rule, or the
/// reason why it did not load. Its load ends. True when it changed the
/// data.
fn detail_loaded(
    state: &mut AppState,
    key: PrKey,
    result: Result<Observed<PrDetail>, SyncError>,
) -> bool {
    if state.data.details_loading.remove(&key) {
        state.bump();
    }
    let data = &mut state.data;
    let changed = match result {
        Ok(detail) => {
            let cleared = data.detail_errors.remove(&key).is_some();
            let applied = match data.details.get_mut(&key) {
                None => {
                    data.details.insert(key, detail);
                    true
                }
                Some(current) => apply(current, detail, &key, "detail"),
            };
            cleared || applied
        }
        Err(err) => {
            let reason = error_text(&err);
            data.detail_errors.insert(key, reason.clone()) != Some(reason)
        }
    };
    if changed {
        state.bump();
    }
    changed
}

/// The changed files of a pull request, under the freshness rule, or the
/// reason why they did not load. Their load ends. True when it changed
/// the data.
fn files_loaded(
    state: &mut AppState,
    key: PrKey,
    result: Result<Observed<PrFiles>, SyncError>,
) -> bool {
    if state.data.files_loading.remove(&key) {
        state.bump();
    }
    let data = &mut state.data;
    let changed = match result {
        Ok(files) => {
            let cleared = data.files_errors.remove(&key).is_some();
            let applied = match data.files.get_mut(&key) {
                None => {
                    data.files.insert(key, files);
                    true
                }
                Some(current) => apply(current, files, &key, "files"),
            };
            cleared || applied
        }
        Err(err) => {
            let reason = error_text(&err);
            data.files_errors.insert(key, reason.clone()) != Some(reason)
        }
    };
    if changed {
        state.bump();
    }
    changed
}

/// A jj scan of one repo, under the freshness rule. True when it changed
/// the data.
fn jj_scanned(state: &mut AppState, repo: RepoKey, scan: Observed<LocalState>) -> bool {
    let local = &mut state.data.local;
    match local.get_mut(&repo) {
        None => {
            local.insert(repo, scan);
        }
        Some(current) => {
            let same = current.value == scan.value;
            if !apply_newer(current, scan) {
                tracing::debug!(?repo, "older jj scan dropped");
                return false;
            }
            if same {
                return false;
            }
        }
    }
    state.data.revision += 1;
    state.bump();
    true
}

/// Merges one pull request into the data. Each field group follows the
/// freshness rule on its own. The merge settings of the repo come along.
/// True when a value changed.
fn merge_pr(data: &mut AppData, incoming: PullRequest) -> bool {
    fold_repo_settings(data, &incoming);
    let Some(current) = data.prs.get_mut(&incoming.key) else {
        data.prs.insert(incoming.key.clone(), incoming);
        return true;
    };
    let PullRequest {
        key,
        node_id,
        head,
        ci_state,
        checks,
    } = incoming;
    if head.observed_at >= current.head.observed_at {
        current.node_id = node_id;
    }
    let mut changed = apply(&mut current.head, head, &key, "head");
    changed |= apply(&mut current.ci_state, ci_state, &key, "ci state");
    changed |= apply(&mut current.checks, checks, &key, "checks");
    changed
}

/// Applies `incoming` when it is not older. True when the value changed.
/// Older data is dropped and logged.
fn apply<T: PartialEq>(
    current: &mut Observed<T>,
    incoming: Observed<T>,
    key: &PrKey,
    group: &str,
) -> bool {
    let same = current.value == incoming.value;
    if apply_newer(current, incoming) {
        !same
    } else {
        tracing::debug!(?key, group, "older data dropped");
        false
    }
}

/// Folds the merge settings of the head of `pr` into the repo of `pr`. A
/// known default branch stays when the new settings have none.
fn fold_repo_settings(data: &mut AppData, pr: &PullRequest) {
    let key = &pr.key.repo;
    let repo = data.repos.entry(key.clone()).or_insert_with(|| Repo {
        key: key.clone(),
        path: None,
        settings: None,
    });
    let mut settings = pr.head.value.repo_settings.clone();
    if settings.default_branch.is_none() {
        settings.default_branch = repo
            .settings
            .as_ref()
            .and_then(|old| old.default_branch.clone());
    }
    repo.settings = Some(settings);
}

/// The open pull requests of `scope` that a complete full sync did not
/// return (`seen`) and whose head was observed before the sync started.
fn gone_candidates(
    data: &AppData,
    scope: Scope,
    seen: &BTreeSet<PrKey>,
    started_at: DateTime<Utc>,
) -> Vec<PrKey> {
    data.prs
        .values()
        .filter(|pr| data.scopes.get(&pr.key) == Some(&scope))
        .filter(|pr| pr.head.value.state == PrState::Open)
        .filter(|pr| pr.head.observed_at <= started_at)
        .filter(|pr| !seen.contains(&pr.key))
        .map(|pr| pr.key.clone())
        .collect()
}

/// The gone rule: a candidate leaves only when its head was observed
/// before the full sync started. True when it left.
fn drop_gone(data: &mut AppData, key: &PrKey, started_at: DateTime<Utc>) -> bool {
    let Some(pr) = data.prs.get(key) else {
        return false;
    };
    if pr.head.observed_at > started_at {
        return false;
    }
    data.prs.remove(key);
    data.scopes.remove(key);
    data.details.remove(key);
    true
}

/// An event of a running action. Output lines go to the output popup of
/// the action and its target. Progress and the result show in the status
/// line, and the result runs the follow-up rule.
fn action_event(
    state: &mut AppState,
    actions: &[Box<dyn Action>],
    id: &'static str,
    key: TargetKey,
    event: ActionEvent,
) -> Vec<Cmd> {
    let label = actions
        .iter()
        .find(|action| action.id() == id)
        .map_or(id, |action| action.label());
    match event {
        ActionEvent::Output(lines) => {
            output(state, id, key, label, lines);
            Vec::new()
        }
        ActionEvent::Progress(text) => {
            state.message(text, CellStyle::Info, INFO_TTL);
            Vec::new()
        }
        ActionEvent::Done(result) => {
            if result.is_ok() {
                let TargetKey::Pr(pr) = &key;
                state.ui.ui_events.push(UiEvent::RowFlash(pr.clone()));
                state.bump();
            }
            match result {
                Ok(text) if text.is_empty() => {}
                Ok(text) => state.message(text, CellStyle::Success, INFO_TTL),
                Err(err) => {
                    state.message(
                        format!("{label} failed: {err}"),
                        CellStyle::Danger,
                        ERROR_TTL,
                    );
                }
            }
            if merge_pending(&state.data, &key) {
                state.data.merge = None;
                state.bump();
            }
            // A pane whose launch failed never started: its `Done` is the
            // last message of the pane.
            let pane = (id, key.clone());
            if state.data.panes.get(&pane) == Some(&PaneHandleState::Starting) {
                state.data.panes.remove(&pane);
                state.bump();
            }
            follow_up(&state.data, &key)
        }
        ActionEvent::Popup(_) | ActionEvent::MergeStatus(_) => {
            tracing::trace!(id, "action event not handled yet");
            Vec::new()
        }
    }
}

/// Appends `lines` to the open output popup of `(id, key)`, or opens one.
/// The popup keeps the last `OUTPUT_LINES` lines.
fn output(
    state: &mut AppState,
    id: &'static str,
    key: TargetKey,
    label: &str,
    mut lines: Vec<String>,
) {
    let open = state
        .ui
        .overlays
        .iter_mut()
        .find_map(|overlay| match &mut overlay.state {
            OverlayState::Output {
                id: open_id,
                key: open_key,
                lines,
                scroll,
                ..
            } if !overlay.closing && *open_id == id && *open_key == key => Some((lines, scroll)),
            _ => None,
        });
    let Some((open, scroll)) = open else {
        cap(&mut lines);
        let title = format!("{label} output");
        let overlay = OverlayState::Output {
            id,
            key,
            title,
            scroll: lines.len().saturating_sub(1),
            lines,
        };
        open_overlay(state, OverlayKind::Output, overlay);
        return;
    };
    // A popup on its last line follows the new lines. Else its first
    // visible line stays, also when old lines drop.
    let follow = *scroll + 1 >= open.len();
    open.append(&mut lines);
    let dropped = open.len().saturating_sub(OUTPUT_LINES);
    cap(open);
    *scroll = if follow {
        open.len().saturating_sub(1)
    } else {
        scroll.saturating_sub(dropped)
    };
    state.bump();
}

/// Drops the oldest lines above `OUTPUT_LINES`.
fn cap(lines: &mut Vec<String>) {
    let extra = lines.len().saturating_sub(OUTPUT_LINES);
    lines.drain(..extra);
}

/// The follow-up rule after an action or a pane: scan the repo of the
/// target, and refresh the stack group of a pull request (or the pull
/// request alone when it has no stack).
fn follow_up(data: &AppData, key: &TargetKey) -> Vec<Cmd> {
    let TargetKey::Pr(pr) = key;
    let mut cmds = vec![Cmd::ScanJj(pr.repo.clone())];
    let refs = stack_group_refs(data, pr);
    if !refs.is_empty() {
        cmds.push(Cmd::RefreshPr(refs));
    }
    cmds
}

/// A new state of an async merge.
fn merge_status(state: &mut AppState, key: PrKey, progress: MergeProgress) -> Vec<Cmd> {
    state.data.merge = Some(MergeState {
        key,
        progress,
        at: state.data.now,
    });
    state.bump();
    Vec::new()
}

/// True while the merge of `key` is pending. A `Done` then means that the
/// poll stopped before a final state, for example on a cancel.
fn merge_pending(data: &AppData, key: &TargetKey) -> bool {
    data.merge.as_ref().is_some_and(|merge| {
        merge.progress == MergeProgress::Pending && *key == TargetKey::Pr(merge.key.clone())
    })
}

/// The refs of the loaded pull requests in the stack of `key`, position 1
/// first, or of `key` alone when it has no stack. Empty when `key` is not
/// loaded.
fn stack_group_refs(data: &AppData, key: &PrKey) -> Vec<PrRef> {
    let Some(pr) = data.prs.get(key) else {
        return Vec::new();
    };
    let Some(stack) = &pr.head.value.stack else {
        return vec![pr_ref(pr)];
    };
    let mut group: Vec<&PullRequest> = data
        .prs
        .values()
        .filter(|other| {
            other
                .head
                .value
                .stack
                .as_ref()
                .is_some_and(|s| s.id == stack.id)
        })
        .collect();
    group.sort_by_key(|other| other.head.value.stack.as_ref().map(|s| s.position));
    group.into_iter().map(pr_ref).collect()
}

/// A new terminal size.
fn resize(state: &mut AppState, size: Size) -> Vec<Cmd> {
    state.data.size = size;
    state.bump();
    Vec::new()
}

/// The deadline of the event loop at `now`. A lasting state of the
/// status bar that ended goes away. An expired status message leaves the
/// status bar on this tick and starts its fade; the message of a lasting
/// state that still holds takes its place. Else the generation moves
/// only when a rendered string changed since the last frame: at a new
/// minute (ages have minute granularity), or when the status bar or the
/// terminal status changed.
fn tick(state: &mut AppState, tabs: &[Box<dyn Tab>], now: DateTime<Utc>) -> Vec<Cmd> {
    let minute = |time: DateTime<Utc>| time.timestamp().div_euclid(60);
    let new_minute = minute(now) != minute(state.data.now);
    state.data.now = now;
    let sync = &mut state.data.sync;
    if sync.condition.as_ref().is_some_and(|c| !c.holds(now)) {
        sync.condition = None;
    }
    if state
        .ui
        .status
        .as_ref()
        .is_some_and(|msg| msg.expires <= now)
    {
        state.ui.status = state.data.sync.condition.as_ref().map(Condition::message);
        state.ui.ui_events.push(UiEvent::StatusExpired);
        state.bump();
        return Vec::new();
    }
    let Some((status, term)) = frame::built_status(state) else {
        return Vec::new();
    };
    if new_minute || *status != frame::status_model(state, tabs) || *term != term_status(state) {
        state.bump();
    }
    Vec::new()
}

/// How long an info message stays in the status bar.
const INFO_TTL: TimeDelta = TimeDelta::seconds(5);
/// How long an error message stays in the status bar.
const ERROR_TTL: TimeDelta = TimeDelta::seconds(10);

/// The `refresh` key: a full sync of every scope that feeds a tab.
fn refresh(state: &mut AppState, tabs: &[Box<dyn Tab>]) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    for scope in tabs.iter().filter_map(|tab| tab.scope()) {
        if state.data.sync.pending.insert(scope) {
            cmds.push(Cmd::SyncNow(scope));
        }
    }
    state.bump();
    cmds
}

/// A worker failed. A sync failure and a rate limit become the failure
/// of the terminal status. A rate limit and no network become a lasting
/// state of the status bar; the other failures only show a message.
fn sync_failed(
    state: &mut AppState,
    source: &'static str,
    scope: Option<Scope>,
    err: &SyncError,
) -> Vec<Cmd> {
    let sync = &mut state.data.sync;
    match scope {
        Some(scope) => {
            sync.pending.remove(&scope);
            sync.running.remove(&scope);
        }
        None if source == "sync" => {
            sync.pending.clear();
            sync.running.clear();
        }
        None => {}
    }
    let reason = error_text(err);
    let rate_limited_until = match err {
        SyncError::RateLimited { until } => Some(*until),
        _ => None,
    };
    if source == "sync" || rate_limited_until.is_some() {
        let reason = match rate_limited_until {
            Some(_) => reason.clone(),
            None => format!("sync failed: {reason}"),
        };
        state.data.sync.failure = Some(Failure {
            source,
            reason,
            rate_limited_until,
            at: state.data.now,
        });
    }
    match err {
        SyncError::RateLimited { until } => {
            show_condition(state, Condition::RateLimited { until: *until });
        }
        SyncError::Offline => show_condition(state, Condition::Offline),
        _ if source == "sync" => state.message(
            format!("sync failed: {reason}"),
            CellStyle::Danger,
            ERROR_TTL,
        ),
        _ => state.message(
            format!("{source} failed: {reason}"),
            CellStyle::Warn,
            INFO_TTL,
        ),
    }
    Vec::new()
}

/// The supervisor restarts a worker that panicked. The status bar shows
/// the restart until it happens. The syncs of a dead sync worker and the
/// loads of a dead detail worker end.
fn worker_died(state: &mut AppState, name: &'static str, restart_in: Duration) -> Vec<Cmd> {
    match name {
        "sync" => state.data.sync.running.clear(),
        "detail" => {
            state.data.details_loading.clear();
            state.data.files_loading.clear();
        }
        _ => {}
    }
    state.data.sync.failure = Some(Failure {
        source: name,
        reason: format!("{name} worker died"),
        rate_limited_until: None,
        at: state.data.now,
    });
    let delay = TimeDelta::from_std(restart_in).unwrap_or(TimeDelta::MAX);
    let at = state
        .data
        .now
        .checked_add_signed(delay)
        .unwrap_or(DateTime::<Utc>::MAX_UTC);
    let condition = Condition::Restarting {
        name,
        secs: restart_in.as_secs(),
        at,
    };
    show_condition(state, condition);
    Vec::new()
}

/// Sets the lasting state of the status bar and shows its message.
fn show_condition(state: &mut AppState, condition: Condition) {
    state.ui.status = Some(condition.message());
    state.data.sync.condition = Some(condition);
    state.bump();
}

/// A request succeeded, so the network is back: the `offline` state
/// ends. Its message expires now, so it leaves the status bar on the
/// next tick, with the fade.
fn back_online(state: &mut AppState) {
    let sync = &mut state.data.sync;
    if sync.condition != Some(Condition::Offline) {
        return;
    }
    sync.condition = None;
    let offline = Condition::Offline.message();
    if let Some(msg) = state.ui.status.as_mut().filter(|msg| **msg == offline) {
        msg.expires = state.data.now;
    }
}

/// The transitions are done: the overlays that slid out go away.
fn transitions_done(state: &mut AppState) -> Vec<Cmd> {
    let overlays = state.ui.overlays.len();
    state.ui.overlays.retain(|overlay| !overlay.closing);
    if state.ui.overlays.len() != overlays {
        state.bump();
    }
    Vec::new()
}

/// Quit, from a quit key or a signal.
fn quit() -> Vec<Cmd> {
    vec![Cmd::Quit]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::fake::FakeAction;
    use crate::app::models::{ListModel, OverlayContent};
    use crate::app::state::Overlay;
    use crate::app::tabs::fake::FakeTab;
    use crate::app::testdata;
    use crate::bus::{Interest, RateInfo, Session};
    use crate::domain::test_time;

    const SIZE: Size = Size {
        width: 100,
        height: 30,
    };

    /// A state with one fake tab that handles every key with `handled`.
    fn setup(handled: Option<Handled>) -> (Vec<Box<dyn Tab>>, AppState) {
        let mut tab = FakeTab::new("one");
        tab.handled = handled;
        let tabs: Vec<Box<dyn Tab>> = vec![Box::new(tab)];
        let state = AppState::with_tabs(SIZE, &tabs);
        (tabs, state)
    }

    fn press(code: KeyCode, mods: KeyModifiers) -> Msg {
        Msg::Key(KeyEvent::new(code, mods))
    }

    fn run(state: &mut AppState, tabs: &[Box<dyn Tab>], msg: Msg) -> Vec<Cmd> {
        update_with(state, &Keymap::default(), tabs, msg)
    }

    fn empty_list() -> ListModel {
        ListModel {
            columns: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
        }
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        let (tabs, mut state) = setup(None);
        let q = press(KeyCode::Char('q'), KeyModifiers::NONE);
        let ctrl_c = press(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(run(&mut state, &tabs, q), vec![Cmd::Quit]);
        assert_eq!(run(&mut state, &tabs, ctrl_c), vec![Cmd::Quit]);
    }

    #[test]
    fn esc_quits_when_no_overlay_is_open() {
        let (tabs, mut state) = setup(None);
        let esc = press(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(run(&mut state, &tabs, esc), vec![Cmd::Quit]);
    }

    #[test]
    fn esc_closes_the_sidebar_and_does_not_quit() {
        let mut state = mine_state(10);
        key(&mut state, 'j');
        update(
            &mut state,
            &Keymap::default(),
            press(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(state.ui.overlays.len(), 1);
        let esc = press(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(update(&mut state, &Keymap::default(), esc), vec![]);
        assert!(state.ui.overlays[0].closing, "the sidebar closes");
    }

    #[test]
    fn esc_clears_the_filter_and_does_not_quit() {
        let mut state = mine_state(10);
        key(&mut state, '/');
        typed(&mut state, "pane");
        let esc = press(KeyCode::Esc, KeyModifiers::NONE);
        let cmds = update(&mut state, &Keymap::default(), esc);
        assert!(!cmds.contains(&Cmd::Quit));
        assert_eq!(cmds[0], persist("filter", "mine", String::new()));
        assert_eq!(state.ui.tabs[0].filter, "");
        assert!(!state.ui.filter_edit);
    }

    #[test]
    fn a_signal_quits() {
        let (tabs, mut state) = setup(None);
        assert_eq!(run(&mut state, &tabs, Msg::Signal), vec![Cmd::Quit]);
    }

    #[test]
    fn a_key_release_does_nothing() {
        let (tabs, mut state) = setup(None);
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert_eq!(run(&mut state, &tabs, Msg::Key(release)), vec![]);
    }

    #[test]
    fn a_key_goes_to_the_active_tab() {
        let handled = Handled {
            cmds: vec![Cmd::SyncNow(Scope::Mine)],
            events: vec![UiEvent::StatusExpired],
            changed: true,
            confirm: None,
        };
        let (tabs, mut state) = setup(Some(handled));
        let j = press(KeyCode::Char('z'), KeyModifiers::NONE);
        let interest = Interest {
            tab: "one",
            scope: None,
            wants_local: true,
            selected: None,
            visible: Vec::new(),
        };
        assert_eq!(
            run(&mut state, &tabs, j.clone()),
            vec![Cmd::SyncNow(Scope::Mine), Cmd::SetInterest(interest)]
        );
        assert_eq!(state.generation, 1);
        assert_eq!(state.ui.tabs[0].cursor, 1);
        assert_eq!(state.ui.ui_events, vec![UiEvent::StatusExpired]);
        assert_eq!(
            run(&mut state, &tabs, j),
            vec![Cmd::SyncNow(Scope::Mine)],
            "an unchanged interest is not sent again"
        );
        assert_eq!(state.generation, 2);
    }

    #[test]
    fn an_unhandled_key_changes_nothing() {
        let (tabs, mut state) = setup(None);
        let j = press(KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(run(&mut state, &tabs, j), vec![]);
        assert_eq!(state.generation, 0);
    }

    #[test]
    fn resize_stores_the_size_and_moves_the_generation() {
        let (tabs, mut state) = setup(None);
        let size = Size {
            width: 160,
            height: 50,
        };
        assert_eq!(run(&mut state, &tabs, Msg::Resize(size)), vec![]);
        assert_eq!(state.data.size, size);
        assert_eq!(state.generation, 1);
    }

    #[test]
    fn tick_moves_the_generation_only_when_a_string_changed() {
        let (tabs, mut state) = setup(None);
        state.data.now = at(0);
        assert_eq!(tick(&mut state, &tabs, at(1)), vec![]);
        assert_eq!(state.generation, 0, "no frame was built yet");
        frame::frame_model_with(&mut state, &tabs);
        assert_eq!(tick(&mut state, &tabs, at(2)), vec![]);
        assert_eq!(state.generation, 0, "the same minute");
        state.ui.status = Some(crate::app::state::StatusMsg {
            text: "synced".into(),
            style: CellStyle::Normal,
            expires: at(600),
        });
        assert_eq!(tick(&mut state, &tabs, at(3)), vec![]);
        assert_eq!(state.generation, 1, "the status bar changed");
        frame::frame_model_with(&mut state, &tabs);
        tick(&mut state, &tabs, at(4));
        assert_eq!(state.generation, 1);
        tick(&mut state, &tabs, at(60));
        assert_eq!(state.generation, 2, "a new minute");
    }

    #[test]
    fn an_expired_message_leaves_on_its_tick_with_the_fade_event() {
        let (tabs, mut state) = setup(None);
        state.data.now = at(0);
        state.message("merged #101", CellStyle::Success, TimeDelta::seconds(5));
        let generation = state.generation;
        tick(&mut state, &tabs, at(4));
        assert!(state.ui.status.is_some());
        assert_eq!(state.ui.ui_events, vec![]);
        tick(&mut state, &tabs, at(5));
        assert_eq!(state.ui.status, None);
        assert_eq!(state.ui.ui_events, vec![UiEvent::StatusExpired]);
        assert_eq!(state.generation, generation + 1);
    }

    #[test]
    fn sync_failed_sets_the_failure_until_a_key_or_a_success() {
        let (tabs, mut state) = setup(None);
        state.data.now = at(0);
        state.data.sync.pending.insert(Scope::Mine);
        let failed = Msg::SyncFailed {
            source: "sync",
            scope: Some(Scope::Mine),
            err: SyncError::Offline,
        };
        assert_eq!(run(&mut state, &tabs, failed.clone()), vec![]);
        assert_eq!(state.data.sync.pending.len(), 0);
        let failure = state.data.sync.failure.clone().unwrap();
        assert_eq!(failure.reason, "sync failed: offline");
        assert_eq!(failure.at, at(0));
        let msg = state.ui.status.clone().unwrap();
        assert_eq!(
            (msg.text.as_str(), msg.style),
            ("offline", CellStyle::Danger)
        );
        run(
            &mut state,
            &tabs,
            press(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(state.data.sync.failure, None, "a key press clears it");
        run(&mut state, &tabs, failed);
        run(&mut state, &tabs, synced(Vec::new(), None, 0));
        assert_eq!(state.data.sync.failure, None, "a success clears it");
    }

    #[test]
    fn other_workers_and_rate_limits() {
        let (tabs, mut state) = setup(None);
        let ci = Msg::SyncFailed {
            source: "ci",
            scope: None,
            err: SyncError::Other("502".into()),
        };
        run(&mut state, &tabs, ci);
        assert_eq!(
            state.data.sync.failure, None,
            "a ci failure is a message only"
        );
        assert_eq!(state.ui.status.clone().unwrap().text, "ci failed: 502");
        let until = at(600);
        let limited = Msg::SyncFailed {
            source: "ci",
            scope: None,
            err: SyncError::RateLimited { until },
        };
        run(&mut state, &tabs, limited);
        let failure = state.data.sync.failure.clone().unwrap();
        assert_eq!(failure.rate_limited_until, Some(until));
        assert_eq!(
            failure.reason,
            error_text(&SyncError::RateLimited { until })
        );
        assert!(failure.reason.starts_with("rate limited until "));
        let died = Msg::WorkerDied {
            name: "sync",
            restart_in: Duration::from_secs(2),
        };
        run(&mut state, &tabs, died);
        assert_eq!(
            state.data.sync.failure.clone().unwrap().reason,
            "sync worker died"
        );
        assert_eq!(
            state.ui.status.clone().unwrap().text,
            "sync worker restarting in 2s"
        );
    }

    /// The text and the style of the status bar message.
    fn status_of(state: &AppState) -> Option<(String, CellStyle)> {
        state
            .ui
            .status
            .as_ref()
            .map(|msg| (msg.text.clone(), msg.style))
    }

    #[test]
    fn rate_limited_stays_until_the_reset_time_and_then_fades() {
        let (tabs, mut state) = setup(None);
        state.data.now = at(0);
        let until = at(600);
        let limited = Msg::SyncFailed {
            source: "sync",
            scope: None,
            err: SyncError::RateLimited { until },
        };
        run(&mut state, &tabs, limited);
        let text = error_text(&SyncError::RateLimited { until });
        assert_eq!(status_of(&state), Some((text.clone(), CellStyle::Warn)));
        run(
            &mut state,
            &tabs,
            press(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        tick(&mut state, &tabs, at(599));
        assert_eq!(
            status_of(&state),
            Some((text, CellStyle::Warn)),
            "a key press and the error time do not clear it"
        );
        assert_eq!(state.ui.ui_events, vec![]);
        tick(&mut state, &tabs, at(600));
        assert_eq!(state.ui.status, None);
        assert_eq!(state.data.sync.condition, None);
        assert_eq!(state.ui.ui_events, vec![UiEvent::StatusExpired]);
    }

    #[test]
    fn offline_stays_until_the_next_success_and_then_fades() {
        let (tabs, mut state) = setup(None);
        state.data.now = at(0);
        let offline = Msg::SyncFailed {
            source: "ci",
            scope: None,
            err: SyncError::Offline,
        };
        run(&mut state, &tabs, offline);
        let shown = Some(("offline".to_owned(), CellStyle::Danger));
        assert_eq!(status_of(&state), shown);
        tick(&mut state, &tabs, at(3600));
        assert_eq!(status_of(&state), shown, "no time limit");
        run(&mut state, &tabs, synced(Vec::new(), None, 3600));
        assert_eq!(state.data.sync.condition, None);
        assert_eq!(status_of(&state), shown, "the message leaves on a tick");
        assert_eq!(state.ui.ui_events, vec![]);
        tick(&mut state, &tabs, at(3601));
        assert_eq!(state.ui.status, None);
        assert_eq!(state.ui.ui_events, vec![UiEvent::StatusExpired]);
    }

    #[test]
    fn a_worker_restart_stays_until_the_restart() {
        let (tabs, mut state) = setup(None);
        state.data.now = at(0);
        let died = Msg::WorkerDied {
            name: "ci",
            restart_in: Duration::from_secs(30),
        };
        run(&mut state, &tabs, died);
        let shown = Some(("ci worker restarting in 30s".to_owned(), CellStyle::Warn));
        assert_eq!(status_of(&state), shown);
        tick(&mut state, &tabs, at(29));
        assert_eq!(status_of(&state), shown);
        tick(&mut state, &tabs, at(30));
        assert_eq!(state.ui.status, None);
        assert_eq!(state.ui.ui_events, vec![UiEvent::StatusExpired]);
    }

    #[test]
    fn a_lasting_state_comes_back_after_a_message_on_top() {
        let (tabs, mut state) = setup(None);
        state.data.now = at(0);
        let offline = Msg::SyncFailed {
            source: "sync",
            scope: None,
            err: SyncError::Offline,
        };
        run(&mut state, &tabs, offline);
        state.message("copied", CellStyle::Info, TimeDelta::seconds(5));
        tick(&mut state, &tabs, at(5));
        assert_eq!(
            status_of(&state),
            Some(("offline".to_owned(), CellStyle::Danger))
        );
        assert_eq!(state.ui.ui_events, vec![UiEvent::StatusExpired]);
    }

    #[test]
    fn three_lasting_states_are_distinct() {
        let until = at(600);
        let rate = Condition::RateLimited { until }.message();
        let offline = Condition::Offline.message();
        let restart = Condition::Restarting {
            name: "sync",
            secs: 4,
            at: at(4),
        }
        .message();
        assert_eq!(
            (rate.style, rate.expires),
            (CellStyle::Warn, until),
            "{}",
            rate.text
        );
        assert!(rate.text.starts_with("rate limited until "));
        assert_eq!(
            (offline.text.as_str(), offline.style),
            ("offline", CellStyle::Danger)
        );
        assert_eq!(
            (restart.text.as_str(), restart.style, restart.expires),
            ("sync worker restarting in 4s", CellStyle::Warn, at(4))
        );
    }

    #[test]
    fn prefetch_batch_is_stored_until_done_equals_total() {
        let mut state = AppState::new(SIZE);
        let batch = |done, total| Msg::PrefetchBatch { done, total };
        let generation = state.generation;
        assert_eq!(update(&mut state, &Keymap::default(), batch(0, 3)), vec![]);
        assert_eq!(
            state.data.prefetch,
            Some(PrefetchBatch { done: 0, total: 3 })
        );
        assert!(state.generation > generation);
        update(&mut state, &Keymap::default(), batch(2, 3));
        assert_eq!(
            state.data.prefetch,
            Some(PrefetchBatch { done: 2, total: 3 })
        );
        let term = term_status(&state);
        assert_eq!(term.msg.as_deref(), Some("prefetching"));
        let generation = state.generation;
        update(&mut state, &Keymap::default(), batch(2, 3));
        assert_eq!(state.generation, generation, "no change, no bump");
        update(&mut state, &Keymap::default(), batch(3, 3));
        assert_eq!(state.data.prefetch, None);
        update(&mut state, &Keymap::default(), batch(0, 0));
        assert_eq!(state.data.prefetch, None);
    }

    #[test]
    fn refresh_syncs_every_tab_scope_once() {
        let mine = FakeTab::new("one");
        let tabs: Vec<Box<dyn Tab>> = vec![Box::new(mine)];
        let mut state = AppState::with_tabs(SIZE, &tabs);
        let r = press(KeyCode::Char('r'), KeyModifiers::NONE);
        assert_eq!(
            run(&mut state, &tabs, r.clone()),
            vec![fake_interest()],
            "no scope"
        );
        let real = &TABS;
        let mut state = AppState::new(SIZE);
        let cmds = update(&mut state, &Keymap::default(), r.clone());
        let mut scopes: Vec<_> = real.iter().filter_map(|tab| tab.scope()).collect();
        scopes.dedup();
        let syncs: Vec<_> = cmds
            .iter()
            .filter(|cmd| matches!(cmd, Cmd::SyncNow(_)))
            .cloned()
            .collect();
        assert_eq!(
            syncs,
            scopes.into_iter().map(Cmd::SyncNow).collect::<Vec<_>>()
        );
        let again = update(&mut state, &Keymap::default(), r);
        assert!(
            !again.iter().any(|cmd| matches!(cmd, Cmd::SyncNow(_))),
            "a pending sync is not asked again"
        );
    }

    #[test]
    fn question_mark_opens_the_help_and_closes_it_again() {
        let (tabs, mut state) = setup(None);
        let help = press(KeyCode::Char('?'), KeyModifiers::NONE);
        assert_eq!(run(&mut state, &tabs, help.clone()), vec![]);
        let expected = help_model(&Keymap::default(), &tabs, &REGISTRY);
        let overlays = &frame::frame_model_with(&mut state, &tabs).overlays;
        assert_eq!(
            overlays.last().map(|o| (o.kind, &o.content)),
            Some((
                OverlayKind::Help,
                &OverlayContent::Help {
                    help: expected,
                    scroll: 0,
                }
            ))
        );
        assert_eq!(
            state.ui.ui_events,
            [UiEvent::OverlayOpened(OverlayKind::Help)]
        );
        assert_eq!(run(&mut state, &tabs, help.clone()), vec![]);
        assert!(state.ui.overlays.iter().all(|o| o.closing));
        run(&mut state, &tabs, Msg::TransitionsDone);
        assert!(state.ui.overlays.is_empty());
        run(&mut state, &tabs, help);
        let esc = press(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(run(&mut state, &tabs, esc), vec![]);
        assert!(state.ui.overlays.iter().all(|o| o.closing));
        assert_eq!(
            state.ui.ui_events.last(),
            Some(&UiEvent::OverlayClosed(OverlayKind::Help))
        );
    }

    #[test]
    fn transitions_done_drops_the_closing_overlays() {
        let (tabs, mut state) = setup(None);
        let overlay = |kind, closing| Overlay {
            kind,
            closing,
            state: OverlayState::Authors(AuthorPicker {
                tab: 0,
                authors: Vec::new(),
                cursor: 0,
            }),
        };
        state.ui.overlays = vec![
            overlay(OverlayKind::Popup, false),
            overlay(OverlayKind::Confirm, true),
        ];
        assert_eq!(run(&mut state, &tabs, Msg::TransitionsDone), vec![]);
        assert_eq!(state.ui.overlays.len(), 1);
        assert_eq!(state.ui.overlays[0].kind, OverlayKind::Popup);
        assert_eq!(state.generation, 1);
        assert_eq!(run(&mut state, &tabs, Msg::TransitionsDone), vec![]);
        assert_eq!(state.generation, 1, "nothing left to drop");
    }

    fn at(secs: i64) -> DateTime<Utc> {
        test_time() + chrono::TimeDelta::seconds(secs)
    }

    /// A pull request with every group observed at `secs`.
    fn pr_at(number: u64, title: &str, secs: i64) -> PullRequest {
        let mut pr = testdata::pr("chaoshub", number, title);
        pr.head.observed_at = at(secs);
        pr.ci_state.observed_at = at(secs);
        pr.checks.observed_at = at(secs);
        pr
    }

    fn fake_interest() -> Cmd {
        Cmd::SetInterest(Interest {
            tab: "one",
            scope: None,
            wants_local: false,
            selected: None,
            visible: Vec::new(),
        })
    }

    fn synced(prs: Vec<PullRequest>, seen: Option<Vec<PrKey>>, started: i64) -> Msg {
        Msg::PrsSynced {
            scope: Scope::Mine,
            prs,
            seen,
            started_at: at(started),
            last: true,
        }
    }

    #[test]
    fn prs_synced_stores_the_prs_and_scans_each_repo_with_a_path_once() {
        let (tabs, mut state) = setup(None);
        let repo = |name: &str, path: Option<&str>| Repo {
            key: testdata::repo_key(name),
            path: path.map(Into::into),
            settings: None,
        };
        state.load(
            crate::store::Snapshot::default(),
            vec![
                repo("chaoshub", Some("/code/chaoshub")),
                repo("deploy", Some("/code/deploy")),
                repo("codeview", None),
            ],
        );
        let mut other = pr_at(3, "Other repo", 0);
        other.key.repo = testdata::repo_key("codeview");
        let mut unknown = pr_at(4, "Repo outside the session", 0);
        unknown.key.repo = testdata::repo_key("elsewhere");
        let prs = vec![pr_at(1, "One", 0), pr_at(2, "Two", 0), other, unknown];
        assert_eq!(
            run(&mut state, &tabs, synced(prs, None, 0)),
            vec![Cmd::ScanJj(testdata::repo_key("chaoshub")), fake_interest()],
            "codeview and elsewhere have no path, deploy has no pull request"
        );
        assert_eq!(state.data.prs.len(), 4);
        assert_eq!(
            state.data.scopes.get(&testdata::pr_key("chaoshub", 1)),
            Some(&Scope::Mine)
        );
        assert_eq!(state.data.synced.get(&Scope::Mine), Some(&at(0)));
        assert_eq!(state.data.revision, 1);
        assert_eq!(state.generation, 2, "one for the load, one for the sync");
        let settings = state.data.repos[&testdata::repo_key("chaoshub")]
            .settings
            .clone();
        assert_eq!(settings, Some(pr_at(1, "One", 0).head.value.repo_settings));
    }

    #[test]
    fn a_page_shows_its_rows_and_only_the_end_finishes_the_sync() {
        let (tabs, mut state) = setup(None);
        state.data.sync.pending.insert(Scope::Mine);
        let page = Msg::PrsSynced {
            scope: Scope::Mine,
            prs: vec![pr_at(1, "One", 0)],
            seen: None,
            started_at: at(0),
            last: false,
        };
        run(&mut state, &tabs, page);
        assert_eq!(state.data.prs.len(), 1, "the page shows at once");
        assert!(state.data.sync.pending.contains(&Scope::Mine));
        assert_eq!(state.data.synced.get(&Scope::Mine), None);

        let page = Msg::PrsSynced {
            scope: Scope::Mine,
            prs: vec![pr_at(2, "Two", 1)],
            seen: None,
            started_at: at(0),
            last: false,
        };
        run(&mut state, &tabs, page);
        let seen = vec![
            testdata::pr_key("chaoshub", 1),
            testdata::pr_key("chaoshub", 2),
        ];
        run(&mut state, &tabs, synced(Vec::new(), Some(seen), 0));
        let keys: Vec<_> = state.data.prs.keys().map(|key| key.number).collect();
        assert_eq!(keys, [1, 2], "the rows of the earlier pages stay");
        assert!(state.data.sync.pending.is_empty());
        assert_eq!(state.data.synced.get(&Scope::Mine), Some(&at(0)));
    }

    #[test]
    fn a_started_sync_runs_until_its_end_or_a_failure() {
        let (tabs, mut state) = setup(None);
        let started = |scope| Msg::SyncStarted { scope };
        run(&mut state, &tabs, started(Scope::Mine));
        run(&mut state, &tabs, started(Scope::Team));
        assert!(state.data.busy());
        run(&mut state, &tabs, synced(Vec::new(), None, 0));
        assert_eq!(
            state.data.sync.running,
            std::collections::HashSet::from([Scope::Team])
        );
        let failed = Msg::SyncFailed {
            source: "sync",
            scope: Some(Scope::Team),
            err: SyncError::Offline,
        };
        run(&mut state, &tabs, failed);
        assert!(state.data.sync.running.is_empty());

        run(&mut state, &tabs, started(Scope::Mine));
        let died = Msg::WorkerDied {
            name: "sync",
            restart_in: Duration::from_secs(1),
        };
        run(&mut state, &tabs, died);
        assert!(!state.data.busy(), "a dead sync worker runs no sync");
    }

    #[test]
    fn older_groups_are_dropped_one_by_one() {
        let (tabs, mut state) = setup(None);
        run(
            &mut state,
            &tabs,
            synced(vec![pr_at(1, "New", 10)], None, 10),
        );
        let mut older = pr_at(1, "Old", 5);
        older.ci_state = Observed {
            value: crate::domain::pr::CiState::Success,
            observed_at: at(20),
        };
        run(&mut state, &tabs, synced(vec![older], None, 5));
        let pr = &state.data.prs[&testdata::pr_key("chaoshub", 1)];
        assert_eq!(pr.head.value.title, "New", "the older head is dropped");
        assert_eq!(pr.head.observed_at, at(10));
        assert_eq!(
            pr.ci_state.value,
            crate::domain::pr::CiState::Success,
            "the newer CI state applies"
        );
        assert_eq!(state.data.revision, 2);
    }

    #[test]
    fn the_same_data_again_does_not_move_the_revision() {
        let (tabs, mut state) = setup(None);
        run(&mut state, &tabs, synced(vec![pr_at(1, "One", 0)], None, 0));
        run(&mut state, &tabs, synced(vec![pr_at(1, "One", 5)], None, 5));
        assert_eq!(state.data.revision, 1);
        assert_eq!(
            state.data.prs[&testdata::pr_key("chaoshub", 1)]
                .head
                .observed_at,
            at(5)
        );
    }

    #[test]
    fn gone_drops_only_candidates_seen_before_the_sync_started() {
        let (tabs, mut state) = setup(None);
        let prs = vec![pr_at(1, "Old", 0), pr_at(2, "Refreshed", 30)];
        run(&mut state, &tabs, synced(prs, None, 0));
        let seen = vec![testdata::pr_key("chaoshub", 9)];
        let cmds = run(&mut state, &tabs, synced(Vec::new(), Some(seen), 20));
        assert_eq!(cmds, vec![]);
        let keys: Vec<_> = state.data.prs.keys().map(|key| key.number).collect();
        assert_eq!(keys, [2], "#2 was refreshed after the sync started");
        assert_eq!(state.data.scopes.len(), 1);
    }

    #[test]
    fn gone_drops_a_candidate_observed_at_the_start_time() {
        let (tabs, mut state) = setup(None);
        run(
            &mut state,
            &tabs,
            synced(vec![pr_at(1, "One", 20)], None, 0),
        );
        run(&mut state, &tabs, synced(Vec::new(), Some(Vec::new()), 20));
        assert_eq!(
            state.data.prs.len(),
            0,
            "observed_at == started_at counts as not seen"
        );
    }

    #[test]
    fn a_complete_full_sync_drops_the_cached_prs_that_it_did_not_return() {
        let (tabs, mut state) = setup(None);
        let mut merged = pr_at(5, "Merged before", 0);
        merged.head.value.state = PrState::Merged;
        let mut theirs = pr_at(3, "Team scope", 0);
        theirs.head.value.author = "alice".into();
        let snapshot = crate::store::Snapshot {
            prs: vec![
                (Scope::Mine, pr_at(1, "Closed while the app was off", 0)),
                (Scope::Mine, pr_at(2, "Refreshed after the start", 30)),
                (Scope::Team, theirs),
                (Scope::Mine, merged),
            ],
            ..crate::store::Snapshot::default()
        };
        state.load(snapshot, Vec::new());
        let partial = run(&mut state, &tabs, synced(Vec::new(), None, 20));
        assert_eq!(partial, vec![fake_interest()]);
        assert_eq!(state.data.prs.len(), 4, "a partial sync drops nothing");
        let open = pr_at(4, "Still open", 20);
        run(&mut state, &tabs, synced(vec![open], Some(Vec::new()), 20));
        let keys: Vec<_> = state.data.prs.keys().map(|key| key.number).collect();
        assert_eq!(
            keys,
            [2, 3, 4, 5],
            "#1 is gone; #2 is newer, #3 is in another scope, #5 is not open"
        );
        assert!(
            !state
                .data
                .scopes
                .contains_key(&testdata::pr_key("chaoshub", 1))
        );
    }

    #[test]
    fn pr_refreshed_merges_and_picks_a_scope() {
        let (tabs, mut state) = setup(None);
        state.data.session.login = Some("chaosteil".into());
        let mut colleague = pr_at(2, "Theirs", 0);
        colleague.head.value.author = "alice".into();
        let cmds = run(&mut state, &tabs, Msg::PrRefreshed(Box::new(colleague)));
        assert_eq!(cmds, vec![fake_interest()]);
        let cmds = run(
            &mut state,
            &tabs,
            Msg::PrRefreshed(Box::new(pr_at(1, "Mine", 0))),
        );
        assert_eq!(cmds, vec![], "the interest did not change");
        let scope = |n| state.data.scopes[&testdata::pr_key("chaoshub", n)];
        assert_eq!(scope(1), Scope::Mine);
        assert_eq!(scope(2), Scope::Team);
        let generation = state.generation;
        let older = pr_at(1, "Older", -10);
        assert_eq!(
            run(&mut state, &tabs, Msg::PrRefreshed(Box::new(older))),
            vec![]
        );
        assert_eq!(state.generation, generation, "older data is not visible");
    }

    fn stacked(number: u64, position: u32) -> PullRequest {
        let mut pr = pr_at(number, "Stacked", 0);
        pr.head.value.stack = Some(crate::domain::stack::StackRef {
            id: "S_1".into(),
            number: 7,
            size: 3,
            position,
            base_ref: "main".into(),
        });
        pr
    }

    #[test]
    fn follow_up_refreshes_the_stack_group_and_scans_the_repo() {
        let (tabs, mut state) = setup(None);
        let prs = vec![
            stacked(13, 3),
            stacked(11, 1),
            stacked(12, 2),
            pr_at(5, "Single", 0),
        ];
        run(&mut state, &tabs, synced(prs.clone(), None, 0));
        let done = |key| Msg::ActionEvent {
            id: "approve",
            key,
            event: ActionEvent::Done(Ok("approved".into())),
        };
        let repo = testdata::repo_key("chaoshub");
        let refs = |numbers: &[u64]| {
            numbers
                .iter()
                .map(|&n| pr_ref(&state.data.prs[&testdata::pr_key("chaoshub", n)]))
                .collect::<Vec<_>>()
        };
        let stack_refs = refs(&[11, 12, 13]);
        let single_refs = refs(&[5]);
        let key = |n| TargetKey::Pr(testdata::pr_key("chaoshub", n));
        assert_eq!(
            run(&mut state, &tabs, done(key(12))),
            vec![Cmd::ScanJj(repo.clone()), Cmd::RefreshPr(stack_refs)]
        );
        assert_eq!(
            run(
                &mut state,
                &tabs,
                Msg::PaneEnded {
                    id: "review",
                    key: key(5)
                }
            ),
            vec![
                Cmd::ScanJj(repo.clone()),
                Cmd::RefreshPr(single_refs),
                Cmd::LoadCodeview(testdata::pr_key("chaoshub", 5)),
            ]
        );
        assert_eq!(
            run(&mut state, &tabs, done(key(99))),
            vec![Cmd::ScanJj(repo)],
            "an unknown pull request has no refs"
        );
        let progress = Msg::ActionEvent {
            id: "approve",
            key: key(5),
            event: ActionEvent::Progress("working".into()),
        };
        assert_eq!(run(&mut state, &tabs, progress), vec![]);
    }

    /// The real tabs with the mine pull requests and no jj scan, on a
    /// screen with a list of `height` rows.
    /// The mine state with `height` list rows: the screen also has the
    /// tab bar, the header row, the key hint bar and the status bar.
    fn mine_state(height: u16) -> AppState {
        let mut state = AppState::new(Size {
            width: 100,
            height: height + 4,
        });
        let snapshot = crate::store::Snapshot {
            local: Vec::new(),
            ..testdata::mine_snapshot()
        };
        state.load(snapshot, Vec::new());
        state
    }

    fn key(state: &mut AppState, c: char) -> Vec<Cmd> {
        update(
            state,
            &Keymap::default(),
            press(KeyCode::Char(c), KeyModifiers::NONE),
        )
    }

    fn selected_number(state: &mut AppState) -> Option<u64> {
        let model = frame::active_list(state, &TABS);
        match &model.rows[model.cursor].item {
            crate::app::models::RowItem::Pr(pr) => Some(pr.key.number),
            _ => None,
        }
    }

    #[test]
    fn j_and_k_move_over_the_rows_and_skip_headers() {
        // Rows: #99, header, #103, #102, #101, #7.
        let mut state = mine_state(10);
        assert_eq!(selected_number(&mut state), Some(99));
        let cmds = key(&mut state, 'j');
        assert_eq!(selected_number(&mut state), Some(103));
        let selected = state.data.prs[&testdata::pr_key("chaoshub", 103)].clone();
        assert!(matches!(
            cmds.as_slice(),
            [Cmd::SetInterest(interest)] if interest.selected == Some(pr_ref(&selected))
                && interest.visible.len() == 5
                && interest.tab == "mine"
        ));
        key(&mut state, 'G');
        assert_eq!(selected_number(&mut state), Some(7));
        key(&mut state, 'k');
        assert_eq!(selected_number(&mut state), Some(101));
        key(&mut state, 'g');
        assert_eq!(selected_number(&mut state), Some(99));
        let generation = state.generation;
        assert_eq!(key(&mut state, 'k'), vec![], "no move at the top");
        assert_eq!(state.generation, generation);
        assert_eq!(state.ui.ui_events, vec![], "no scroll in a tall list");
    }

    #[test]
    fn the_header_key_toggles_the_row_and_writes_the_cache() {
        let mut state = mine_state(4);
        assert!(state.ui.column_headers, "on by default");
        assert_eq!(frame::list_size(&state).height, 4);
        let generation = state.generation;
        let cmds = key(&mut state, 'H');
        assert!(!state.ui.column_headers);
        assert!(state.generation > generation);
        assert_eq!(
            cmds[0],
            Cmd::Persist(Persist::UiState {
                key: "column_headers".into(),
                value: "false".into(),
            })
        );
        assert_eq!(frame::list_size(&state).height, 5, "the list takes the row");
        assert!(!frame::frame_model(&mut state).column_headers);
        let cmds = key(&mut state, 'H');
        assert!(state.ui.column_headers);
        assert_eq!(
            cmds[0],
            Cmd::Persist(Persist::UiState {
                key: "column_headers".into(),
                value: "true".into(),
            })
        );
        assert!(frame::frame_model(&mut state).column_headers);
    }

    #[test]
    fn the_header_row_takes_a_row_from_the_scroll_window() {
        // Rows: #99, header, #103, #102, #101, #7. The window has 5 rows
        // without the header row and 4 with it.
        let mut state = mine_state(4);
        state.ui.column_headers = false;
        state.bump();
        for _ in 0..3 {
            key(&mut state, 'j');
        }
        assert_eq!(selected_number(&mut state), Some(101));
        assert_eq!(state.ui.tabs[0].scroll, 0, "#101 is the fifth row");
        key(&mut state, 'H');
        assert!(state.ui.column_headers);
        assert_eq!(
            state.ui.tabs[0].scroll, 1,
            "the cursor stays visible in the shorter window"
        );
        assert_eq!(selected_number(&mut state), Some(101));
        key(&mut state, 'H');
        assert_eq!(
            state.ui.tabs[0].scroll, 1,
            "a taller window keeps the scroll"
        );
        let mut state = mine_state(4);
        for _ in 0..3 {
            key(&mut state, 'j');
        }
        assert_eq!(
            state.ui.tabs[0].scroll, 1,
            "with the header row, the fourth move scrolls"
        );
    }

    #[test]
    fn a_move_out_of_the_window_scrolls_with_an_event() {
        let mut state = mine_state(4);
        key(&mut state, 'j');
        key(&mut state, 'j');
        assert_eq!(state.ui.ui_events, vec![], "#103 and #102 are visible");
        key(&mut state, 'j');
        assert_eq!(
            state.ui.ui_events,
            vec![UiEvent::ScrollTo {
                tab: "mine",
                from: 0,
                to: 1,
            }]
        );
        assert_eq!(frame::active_list(&mut state, &TABS).scroll, 1);
        let ctrl_u = press(KeyCode::Char('u'), KeyModifiers::CONTROL);
        update(&mut state, &Keymap::default(), ctrl_u);
        assert_eq!(selected_number(&mut state), Some(103));
    }

    #[test]
    fn the_selection_survives_a_sync() {
        let mut state = mine_state(10);
        key(&mut state, 'j');
        key(&mut state, 'j');
        assert_eq!(selected_number(&mut state), Some(102));
        let mut newer = testdata::pr("chaoshub", 200, "A new one on top");
        newer.head.value.updated_at += TimeDelta::days(1);
        newer.head.observed_at += TimeDelta::seconds(1);
        let mut moved = state.data.prs[&testdata::pr_key("chaoshub", 102)].clone();
        moved.head.value.head_oid = "pushed".into();
        moved.head.observed_at += TimeDelta::seconds(1);
        let msg = Msg::PrsSynced {
            scope: Scope::Mine,
            prs: vec![newer, moved.clone()],
            seen: None,
            started_at: at(1),
            last: true,
        };
        let cmds = update(&mut state, &Keymap::default(), msg);
        assert_eq!(selected_number(&mut state), Some(102));
        assert_eq!(frame::active_list(&mut state, &TABS).cursor, 4);
        let interest = cmds.iter().find_map(|cmd| match cmd {
            Cmd::SetInterest(interest) => Some(interest.clone()),
            _ => None,
        });
        assert_eq!(
            interest.and_then(|interest| interest.selected),
            Some(pr_ref(&moved)),
            "the interest carries the new head"
        );
    }

    #[test]
    fn tab_keys_switch_and_keep_the_previous_list() {
        let tabs: Vec<Box<dyn Tab>> =
            vec![Box::new(FakeTab::new("one")), Box::new(FakeTab::new("two"))];
        let mut state = AppState::with_tabs(SIZE, &tabs);
        let interest = |tab| {
            Cmd::SetInterest(Interest {
                tab,
                scope: None,
                wants_local: false,
                selected: None,
                visible: Vec::new(),
            })
        };
        let two = press(KeyCode::Char('2'), KeyModifiers::NONE);
        assert_eq!(run(&mut state, &tabs, two.clone()), vec![interest("two")]);
        assert_eq!(state.ui.active, 1);
        assert_eq!(
            state.ui.ui_events,
            vec![UiEvent::TabChanged { from: 0, to: 1 }]
        );
        assert_eq!(state.generation, 1);
        assert_eq!(run(&mut state, &tabs, two), vec![], "already active");
        assert_eq!(state.generation, 1);
        let tab = press(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(run(&mut state, &tabs, tab), vec![interest("one")]);
        assert_eq!(state.ui.active, 0);
        assert_eq!(
            state.ui.ui_events.last(),
            Some(&UiEvent::TabChanged { from: 1, to: 0 })
        );
        let nine = press(KeyCode::Char('9'), KeyModifiers::NONE);
        assert_eq!(run(&mut state, &tabs, nine), vec![], "no ninth tab");
    }

    #[test]
    fn the_frame_shows_the_new_tab_after_a_switch() {
        let tabs: Vec<Box<dyn Tab>> =
            vec![Box::new(FakeTab::new("one")), Box::new(FakeTab::new("two"))];
        let mut state = AppState::with_tabs(SIZE, &tabs);
        run(
            &mut state,
            &tabs,
            press(KeyCode::Char('2'), KeyModifiers::NONE),
        );
        let frame = frame::frame_model_with(&mut state, &tabs).clone();
        assert_eq!(frame.tab_bar.active, 1);
    }

    fn typed(state: &mut AppState, text: &str) {
        for c in text.chars() {
            key(state, c);
        }
    }

    #[test]
    fn slash_types_a_filter_that_narrows_the_list() {
        let mut state = mine_state(10);
        assert_eq!(frame::active_list(&mut state, &TABS).rows.len(), 6);
        key(&mut state, '/');
        assert!(state.ui.filter_edit);
        typed(&mut state, "quit");
        assert_eq!(state.ui.tabs[0].filter, "quit", "q types, it does not quit");
        let back = press(KeyCode::Backspace, KeyModifiers::NONE);
        for _ in 0..4 {
            update(&mut state, &Keymap::default(), back.clone());
        }
        typed(&mut state, "pane tab");
        let model = frame::active_list(&mut state, &TABS);
        let titles: Vec<_> = model.rows.iter().map(|r| r.cells[0].text.clone()).collect();
        assert_eq!(
            titles,
            [
                "stack #12 -> main · 3 PRs",
                "feat(ui): popup frame with pane tabs",
            ]
        );
        assert_eq!(
            frame::frame_model(&mut state).filter,
            Some(crate::app::models::FilterModel {
                text: "pane tab".into(),
                editing: true,
            })
        );
        assert_eq!(
            frame::list_size(&state).height,
            9,
            "the filter line takes a row"
        );
        let enter = press(KeyCode::Enter, KeyModifiers::NONE);
        let cmds = update(&mut state, &Keymap::default(), enter);
        assert_eq!(cmds, vec![persist("filter", "mine", "pane tab".into())]);
        assert!(!state.ui.filter_edit);
        assert_eq!(selected_number(&mut state), Some(103));
        key(&mut state, 'k');
        assert_eq!(state.ui.tabs[0].filter, "pane tab", "k navigates again");
        key(&mut state, '/');
        let esc = press(KeyCode::Esc, KeyModifiers::NONE);
        let cmds = update(&mut state, &Keymap::default(), esc);
        assert_eq!(cmds[0], persist("filter", "mine", String::new()));
        assert_eq!(state.ui.tabs[0].filter, "");
        assert_eq!(frame::frame_model(&mut state).filter, None);
        assert_eq!(frame::active_list(&mut state, &TABS).rows.len(), 6);
        assert_eq!(
            selected_number(&mut state),
            Some(103),
            "the selection stays"
        );
    }

    #[test]
    fn ctrl_c_quits_while_typing_a_filter() {
        let mut state = mine_state(10);
        key(&mut state, '/');
        let ctrl_c = press(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            update(&mut state, &Keymap::default(), ctrl_c),
            vec![Cmd::Quit]
        );
    }

    #[test]
    fn the_filter_scores_again_after_a_sync() {
        let mut state = mine_state(10);
        key(&mut state, '/');
        typed(&mut state, "quokka");
        assert_eq!(frame::active_list(&mut state, &TABS).rows.len(), 0);
        let mut newer = testdata::pr("chaoshub", 300, "Quokka support");
        newer.head.observed_at += TimeDelta::seconds(1);
        let msg = Msg::PrsSynced {
            scope: Scope::Mine,
            prs: vec![newer],
            seen: None,
            started_at: at(1),
            last: true,
        };
        update(&mut state, &Keymap::default(), msg);
        assert_eq!(frame::active_list(&mut state, &TABS).rows.len(), 1);
    }

    #[test]
    fn jj_scans_follow_the_freshness_rule() {
        let mut state = mine_state(10);
        let (repo, scan) = testdata::mine_local();
        let mut newer = scan.clone();
        newer.observed_at = at(10);
        newer.value.changes.truncate(1);
        let msg = |scan| Msg::JjScanned {
            repo: repo.clone(),
            state: scan,
        };
        let revision = state.data.revision;
        update(&mut state, &Keymap::default(), msg(newer.clone()));
        assert_eq!(state.data.local[&repo], newer);
        assert_eq!(state.data.revision, revision + 1);
        let generation = state.generation;
        assert_eq!(update(&mut state, &Keymap::default(), msg(scan)), vec![]);
        assert_eq!(state.data.local[&repo], newer, "the older scan is dropped");
        assert_eq!(state.generation, generation);
    }

    #[test]
    fn t_on_the_team_tab_shows_all_open_and_keeps_the_selection() {
        let mut state = AppState::new(SIZE);
        state.load(testdata::team_snapshot(), Vec::new());
        key(&mut state, '3');
        key(&mut state, 'j');
        assert_eq!(selected_number(&mut state), Some(86));
        let generation = state.generation;
        key(&mut state, 't');
        assert_eq!(state.generation, generation + 1);
        assert_eq!(frame::active_list(&mut state, &TABS).rows.len(), 4);
        assert_eq!(selected_number(&mut state), Some(86));
        key(&mut state, '1');
        let generation = state.generation;
        key(&mut state, 't');
        assert_eq!(state.generation, generation, "the mine tab has no toggle");
    }

    fn team_state() -> AppState {
        let mut state = AppState::new(SIZE);
        state.load(testdata::team_snapshot(), Vec::new());
        key(&mut state, '3');
        state
    }

    fn picker(state: &AppState) -> Option<&AuthorPicker> {
        state
            .ui
            .overlays
            .iter()
            .rev()
            .find(|o| !o.closing)
            .and_then(|o| match &o.state {
                OverlayState::Authors(picker) => Some(picker),
                OverlayState::Popup(_)
                | OverlayState::Confirm { .. }
                | OverlayState::Prompt { .. }
                | OverlayState::Help { .. }
                | OverlayState::Output { .. } => None,
            })
    }

    #[test]
    fn the_author_picker_filters_the_team_tab() {
        let mut state = team_state();
        key(&mut state, 't');
        state.ui.ui_events.clear();
        key(&mut state, 'a');
        assert_eq!(
            picker(&state).map(AuthorPicker::items),
            Some(vec![
                "all authors".to_owned(),
                "bob".to_owned(),
                "dave".to_owned(),
            ])
        );
        assert_eq!(
            state.ui.ui_events,
            vec![UiEvent::OverlayOpened(OverlayKind::Picker)]
        );
        key(&mut state, 'j');
        assert_eq!(picker(&state).map(|p| p.cursor), Some(1));
        assert_eq!(state.ui.active, 2, "j moves the picker, not the list");
        let enter = press(KeyCode::Enter, KeyModifiers::NONE);
        let cmds = update(&mut state, &Keymap::default(), enter);
        assert_eq!(cmds[0], persist("author", "team", "bob".into()));
        assert!(matches!(cmds[1..], [Cmd::SetInterest(_)]));
        assert_eq!(picker(&state), None);
        assert!(state.ui.overlays[0].closing, "it slides out");
        assert_eq!(
            state.ui.ui_events.last(),
            Some(&UiEvent::OverlayClosed(OverlayKind::Picker))
        );
        assert_eq!(state.ui.tabs[2].author.as_deref(), Some("bob"));
        let model = frame::active_list(&mut state, &TABS);
        assert_eq!(
            model.rows.len(),
            3,
            "the mode row and the two pull requests of bob"
        );
        key(&mut state, 'a');
        assert_eq!(
            picker(&state).map(|p| p.cursor),
            Some(1),
            "the cursor on bob"
        );
        key(&mut state, 'g');
        update(
            &mut state,
            &Keymap::default(),
            press(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(state.ui.tabs[2].author, None);
    }

    #[test]
    fn esc_and_q_close_the_picker_and_ctrl_c_quits() {
        let mut state = team_state();
        key(&mut state, 'a');
        assert_eq!(key(&mut state, 'q'), vec![], "q closes, it does not quit");
        assert_eq!(picker(&state), None);
        key(&mut state, 'a');
        update(
            &mut state,
            &Keymap::default(),
            press(KeyCode::Esc, KeyModifiers::NONE),
        );
        assert_eq!(picker(&state), None);
        assert_eq!(state.ui.tabs[2].author, None);
        key(&mut state, 'a');
        let ctrl_c = press(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            update(&mut state, &Keymap::default(), ctrl_c),
            vec![Cmd::Quit]
        );
        update(&mut state, &Keymap::default(), Msg::TransitionsDone);
        assert_eq!(state.ui.overlays.len(), 1, "the closed pickers are gone");
    }

    #[test]
    fn the_mine_tab_has_no_author_picker() {
        let mut state = mine_state(10);
        key(&mut state, 'a');
        assert_eq!(state.ui.overlays.len(), 0);
        assert_eq!(
            state.ui.status.as_ref().map(|m| m.text.as_str()),
            Some("no authors to pick here")
        );
    }

    #[test]
    fn a_restored_filter_applies_from_the_first_frame() {
        let mut snapshot = testdata::mine_snapshot();
        snapshot
            .ui_state
            .insert("filter.mine".into(), "overlay".into());
        let mut state = AppState::new(SIZE);
        state.load(snapshot, Vec::new());
        start(&mut state);
        let model = frame::active_list(&mut state, &TABS);
        let titles: Vec<_> = model.rows.iter().map(|r| r.cells[0].text.clone()).collect();
        assert_eq!(
            titles,
            [
                "stack #12 -> main · 3 PRs",
                "feat(app): Pane trait, registry and overlay stack",
            ]
        );
        assert_eq!(
            frame::frame_model(&mut state)
                .filter
                .clone()
                .map(|f| f.text),
            Some("overlay".to_owned())
        );
    }

    fn fake_panes() -> Vec<Box<dyn Pane>> {
        use crate::app::panes::fake::FakePane;
        vec![
            Box::new(FakePane {
                id: "a",
                key: '1',
                lines: 3,
                only: None,
            }),
            Box::new(FakePane {
                id: "b",
                key: '2',
                lines: 20,
                only: None,
            }),
            Box::new(FakePane {
                id: "c",
                key: '3',
                lines: 1,
                only: Some(999),
            }),
        ]
    }

    /// The popup content of the top overlay.
    fn popup_content(state: &mut AppState, panes: &[Box<dyn Pane>]) -> OverlayContent {
        frame::frame_model_full(state, &TABS, panes)
            .overlays
            .last()
            .unwrap()
            .content
            .clone()
    }

    #[test]
    fn enter_opens_the_popup_with_the_panes_that_apply() {
        let panes = fake_panes();
        let mut state = mine_state(10);
        let send = |state: &mut AppState, msg: Msg| {
            update_full(state, &Keymap::default(), &TABS, &panes, &REGISTRY, msg)
        };
        let enter = press(KeyCode::Enter, KeyModifiers::NONE);
        let j = press(KeyCode::Char('j'), KeyModifiers::NONE);
        send(&mut state, j.clone());
        state.ui.ui_events.clear();
        assert_eq!(send(&mut state, enter), vec![]);
        assert_eq!(state.ui.overlays.len(), 1);
        assert_eq!(
            state.ui.ui_events,
            vec![UiEvent::OverlayOpened(OverlayKind::Popup)]
        );
        let OverlayContent::Popup {
            title,
            subtitle,
            tabs,
            active,
            cursor,
            ..
        } = popup_content(&mut state, &panes)
        else {
            panic!("not a popup");
        };
        assert_eq!(title, "#103 feat(ui): popup frame with pane tabs");
        assert_eq!(subtitle, "chaosteil · chaosteil/103 -> main · draft");
        let keys: Vec<(char, &str)> = tabs.iter().map(|t| (t.key, t.title)).collect();
        assert_eq!(keys, [('1', "a"), ('2', "b")], "pane c does not apply");
        assert_eq!((active, cursor), (0, 0));
        send(&mut state, j.clone());
        send(&mut state, j.clone());
        send(&mut state, j);
        assert!(
            matches!(
                popup_content(&mut state, &panes),
                OverlayContent::Popup { cursor: 2, .. }
            ),
            "j moves the pane cursor, up to the last item"
        );
        let x = press(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(
            send(&mut state, x.clone()),
            vec![Cmd::OpenUrl("a 2".into())]
        );
        send(&mut state, press(KeyCode::Char('2'), KeyModifiers::NONE));
        assert!(matches!(
            popup_content(&mut state, &panes),
            OverlayContent::Popup {
                active: 1,
                cursor: 0,
                ..
            }
        ));
        let generation = state.generation;
        send(&mut state, press(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(state.generation, generation, "pane c does not apply");
        send(&mut state, press(KeyCode::Tab, KeyModifiers::NONE));
        assert!(
            matches!(
                popup_content(&mut state, &panes),
                OverlayContent::Popup {
                    active: 0,
                    cursor: 2,
                    ..
                }
            ),
            "Tab cycles, each pane keeps its cursor"
        );
        assert_eq!(state.ui.active, 0, "the tab keys go to the popup");
        send(&mut state, press(KeyCode::Esc, KeyModifiers::NONE));
        assert!(state.ui.overlays[0].closing);
        assert_eq!(
            state.ui.ui_events.last(),
            Some(&UiEvent::OverlayClosed(OverlayKind::Popup))
        );
        assert_eq!(send(&mut state, x), vec![], "a closing popup takes no keys");
    }

    #[test]
    fn the_popup_says_when_its_pull_request_is_gone() {
        let mut state = mine_state(10);
        assert_eq!(selected_number(&mut state), Some(99));
        let gone = testdata::pr_key("chaoshub", 99);
        update(
            &mut state,
            &Keymap::default(),
            press(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(state.ui.overlays.len(), 1);
        state.data.prs.remove(&gone);
        state.bump();
        assert!(matches!(
            popup_content(&mut state, &PANES),
            OverlayContent::Popup { subtitle, .. } if subtitle == "this pull request is gone"
        ));
    }

    #[test]
    fn ci_updates_apply_to_the_same_head_only() {
        let (tabs, mut state) = setup(None);
        run(
            &mut state,
            &tabs,
            synced(vec![pr_at(1, "One", 0), pr_at(2, "Two", 0)], None, 0),
        );
        let ci = |secs| Observed {
            value: CiState::Failure,
            observed_at: at(secs),
        };
        let check = crate::domain::check::Check {
            name: "build".into(),
            kind: crate::domain::check::CheckKind::CheckRun,
            status: crate::domain::check::CheckStatus::InProgress,
            conclusion: None,
            started: None,
            completed: None,
            url: None,
            run_id: None,
            suite_id: None,
        };
        let msg = Msg::CiUpdated {
            states: vec![
                (testdata::pr_key("chaoshub", 1), "oid1".into(), ci(5)),
                (testdata::pr_key("chaoshub", 2), "an old head".into(), ci(5)),
                (testdata::pr_key("chaoshub", 9), "oid9".into(), ci(5)),
            ],
            selected_checks: Some((
                testdata::pr_key("chaoshub", 2),
                Observed {
                    value: vec![check],
                    observed_at: at(5),
                },
            )),
        };
        let revision = state.data.revision;
        run(&mut state, &tabs, msg);
        let pr = |state: &AppState, n| state.data.prs[&testdata::pr_key("chaoshub", n)].clone();
        assert_eq!(pr(&state, 1).ci_state, ci(5));
        assert_eq!(
            pr(&state, 2).ci_state.value,
            CiState::None,
            "the head moved"
        );
        assert_eq!(pr(&state, 2).checks.value.len(), 1);
        assert_eq!(state.data.revision, revision + 1);
        let older = Msg::CiUpdated {
            states: vec![(
                testdata::pr_key("chaoshub", 1),
                "oid1".into(),
                Observed {
                    value: CiState::Success,
                    observed_at: at(1),
                },
            )],
            selected_checks: None,
        };
        assert_eq!(run(&mut state, &tabs, older), vec![]);
        assert_eq!(pr(&state, 1).ci_state, ci(5));
    }

    #[test]
    fn a_sync_or_a_poll_that_changes_a_visible_row_glows_it() {
        // Rows: #99, header, #103 in the window; #102, #101, #7 below it.
        let mut state = mine_state(3);
        start(&mut state);
        let later = |minutes| crate::domain::test_time() + chrono::TimeDelta::minutes(minutes);
        let with_ci = |minutes, states: &[(u64, CiState)]| {
            let mut prs = testdata::mine_prs();
            for pr in &mut prs {
                pr.ci_state.observed_at = later(minutes);
                if let Some(&(_, ci)) = states.iter().find(|(n, _)| *n == pr.key.number) {
                    pr.ci_state.value = ci;
                }
            }
            Msg::PrsSynced {
                scope: Scope::Mine,
                prs,
                seen: None,
                started_at: later(minutes),
                last: true,
            }
        };
        let send = |state: &mut AppState, msg| {
            state.ui.ui_events.clear();
            update(state, &Keymap::default(), msg);
            state.ui.ui_events.clone()
        };
        let glow = |n| UiEvent::RowGlow(testdata::pr_key("chaoshub", n));
        assert_eq!(
            send(&mut state, with_ci(1, &[(103, CiState::Success)])),
            vec![],
            "the first sync after start does not glow"
        );
        assert_eq!(
            send(
                &mut state,
                with_ci(2, &[(103, CiState::Failure), (102, CiState::Failure)])
            ),
            vec![glow(103)],
            "#102 is below the window"
        );
        assert_eq!(
            send(
                &mut state,
                with_ci(3, &[(103, CiState::Failure), (102, CiState::Failure)])
            ),
            vec![],
            "no change, no glow"
        );
        let poll = Msg::CiUpdated {
            states: vec![(
                testdata::pr_key("chaoshub", 103),
                "oid103".into(),
                Observed {
                    value: CiState::Pending,
                    observed_at: later(4),
                },
            )],
            selected_checks: None,
        };
        assert_eq!(send(&mut state, poll), vec![glow(103)]);
    }

    #[test]
    fn a_conversation_load_runs_until_its_result_or_a_dead_worker() {
        let (tabs, mut state) = setup(None);
        let one = testdata::pr_key("chaoshub", 1);
        let two = testdata::pr_key("chaoshub", 2);
        run(&mut state, &tabs, Msg::DetailLoading(one.clone()));
        run(&mut state, &tabs, Msg::DetailLoading(two.clone()));
        assert!(state.data.busy());
        run(
            &mut state,
            &tabs,
            Msg::DetailLoaded(one, Err(SyncError::Offline)),
        );
        assert_eq!(state.data.details_loading, BTreeSet::from([two]));
        let died = Msg::WorkerDied {
            name: "detail",
            restart_in: Duration::from_secs(1),
        };
        run(&mut state, &tabs, died);
        assert!(!state.data.busy(), "a dead detail worker loads nothing");
    }

    #[test]
    fn detail_loads_follow_the_freshness_rule_and_keep_errors() {
        let (tabs, mut state) = setup(None);
        let key = testdata::pr_key("chaoshub", 7);
        let at_secs = |secs| Observed {
            value: testdata::detail(),
            observed_at: at(secs),
        };
        let loaded = |result| Msg::DetailLoaded(key.clone(), result);
        run(&mut state, &tabs, loaded(Err(SyncError::Offline)));
        assert_eq!(
            state.data.detail_errors.get(&key).map(String::as_str),
            Some("offline")
        );
        let generation = state.generation;
        run(&mut state, &tabs, loaded(Err(SyncError::Offline)));
        assert_eq!(state.generation, generation, "the same error again");
        run(&mut state, &tabs, loaded(Ok(at_secs(10))));
        assert_eq!(state.data.details.get(&key), Some(&at_secs(10)));
        assert_eq!(state.data.detail_errors.get(&key), None);
        let mut older = at_secs(5);
        older.value.comments.clear();
        let generation = state.generation;
        assert_eq!(run(&mut state, &tabs, loaded(Ok(older))), vec![]);
        assert_eq!(state.data.details.get(&key), Some(&at_secs(10)));
        assert_eq!(state.generation, generation);
    }

    #[test]
    fn a_files_load_runs_until_its_result_or_a_dead_worker() {
        let (tabs, mut state) = setup(None);
        let one = testdata::pr_key("chaoshub", 1);
        let two = testdata::pr_key("chaoshub", 2);
        run(&mut state, &tabs, Msg::FilesLoading(one.clone()));
        run(&mut state, &tabs, Msg::FilesLoading(two.clone()));
        assert!(state.data.busy());
        run(
            &mut state,
            &tabs,
            Msg::FilesLoaded(one, Err(SyncError::Offline)),
        );
        assert_eq!(state.data.files_loading, BTreeSet::from([two]));
        let died = Msg::WorkerDied {
            name: "detail",
            restart_in: Duration::from_secs(1),
        };
        run(&mut state, &tabs, died);
        assert!(!state.data.busy(), "a dead detail worker loads nothing");
    }

    #[test]
    fn files_loads_follow_the_freshness_rule_and_keep_errors() {
        use crate::domain::files::{ChangedFile, FileStatus};

        let (tabs, mut state) = setup(None);
        let key = testdata::pr_key("chaoshub", 7);
        let at_secs = |secs| Observed {
            value: PrFiles {
                files: vec![ChangedFile {
                    path: "src/lib.rs".into(),
                    previous_path: None,
                    status: FileStatus::Modified,
                    additions: 1,
                    deletions: 0,
                    patch: Some("@@ -0,0 +1 @@\n+x".into()),
                }],
                truncated: false,
                head_oid: "oid7".into(),
            },
            observed_at: at(secs),
        };
        let loaded = |result| Msg::FilesLoaded(key.clone(), result);
        run(&mut state, &tabs, loaded(Err(SyncError::Offline)));
        assert_eq!(
            state.data.files_errors.get(&key).map(String::as_str),
            Some("offline")
        );
        let generation = state.generation;
        run(&mut state, &tabs, loaded(Err(SyncError::Offline)));
        assert_eq!(state.generation, generation, "the same error again");
        run(&mut state, &tabs, loaded(Ok(at_secs(10))));
        assert_eq!(state.data.files.get(&key), Some(&at_secs(10)));
        assert_eq!(state.data.files_errors.get(&key), None);
        let mut older = at_secs(5);
        older.value.files.clear();
        let generation = state.generation;
        assert_eq!(run(&mut state, &tabs, loaded(Ok(older))), vec![]);
        assert_eq!(state.data.files.get(&key), Some(&at_secs(10)));
        assert_eq!(state.generation, generation);
    }

    #[test]
    fn startup_done_sets_the_session_syncs_and_scans() {
        let mut state = AppState::new(SIZE);
        let repo = |name: &str, path: Option<&str>| Repo {
            key: testdata::repo_key(name),
            path: path.map(Into::into),
            settings: None,
        };
        state.load(
            crate::store::Snapshot::default(),
            vec![
                repo("chaoshub", Some("/code/chaoshub")),
                repo("deploy", None),
            ],
        );
        let msg = Msg::StartupDone {
            login: "chaosteil".into(),
            cwd_repo: Some(repo("codeview", Some("/code/codeview"))),
        };
        let cmds = update(&mut state, &Keymap::default(), msg);
        let session = Session {
            login: Some("chaosteil".into()),
            repos: vec![
                repo("chaoshub", Some("/code/chaoshub")),
                repo("deploy", None),
                repo("codeview", Some("/code/codeview")),
            ],
        };
        assert_eq!(
            cmds[..5],
            [
                Cmd::SetSession(session),
                Cmd::SyncNow(Scope::Mine),
                Cmd::SyncNow(Scope::Team),
                Cmd::ScanJj(testdata::repo_key("chaoshub")),
                Cmd::ScanJj(testdata::repo_key("codeview")),
            ]
        );
        assert!(matches!(cmds[5..], [Cmd::SetInterest(_)]));
        assert_eq!(state.data.sync.pending.len(), 2);
        assert_eq!(
            state.data.repos[&testdata::repo_key("codeview")].path,
            Some("/code/codeview".into())
        );
        assert_eq!(
            crate::app::term::term_status(&state).msg.as_deref(),
            Some("syncing 3 repos")
        );
    }

    #[test]
    fn a_known_cwd_repo_gets_its_path() {
        let mut state = AppState::new(SIZE);
        let deploy = Repo {
            key: testdata::repo_key("deploy"),
            path: None,
            settings: None,
        };
        state.load(crate::store::Snapshot::default(), vec![deploy.clone()]);
        let msg = Msg::StartupDone {
            login: "chaosteil".into(),
            cwd_repo: Some(Repo {
                path: Some("/code/deploy".into()),
                ..deploy
            }),
        };
        let cmds = update(&mut state, &Keymap::default(), msg);
        assert!(cmds.contains(&Cmd::ScanJj(testdata::repo_key("deploy"))));
        assert_eq!(state.data.session.repos.len(), 1);
        assert_eq!(
            state.data.session.repos[0].path,
            Some("/code/deploy".into())
        );
    }

    #[test]
    fn rate_info_is_stored_without_a_redraw() {
        let (tabs, mut state) = setup(None);
        let info = RateInfo {
            remaining: 4000,
            reset_at: test_time(),
            last_cost: 1,
        };
        assert_eq!(run(&mut state, &tabs, Msg::Rate(info.clone())), vec![]);
        assert_eq!(state.data.sync.rate, Some(info));
        assert_eq!(state.generation, 0);
    }

    /// The real tabs with the mine fixtures and the jj scan, the login
    /// `chaosteil`, and the cursor on the row of `item`.
    fn action_state(item: &dyn Fn(&AppData) -> RowItem) -> AppState {
        let mut state = AppState::new(SIZE);
        state.load(testdata::mine_snapshot(), Vec::new());
        state.data.session.login = Some("chaosteil".into());
        let item = item(&state.data);
        state.ui.tabs[0].selected = Some(item);
        state.bump();
        state
    }

    /// The row item of the pull request `#number` of `chaoshub`.
    fn pr_row(number: u64) -> impl Fn(&AppData) -> RowItem {
        move |data: &AppData| RowItem::Pr(pr_ref(&data.prs[&testdata::pr_key("chaoshub", number)]))
    }

    /// A key press with the fake `actions` and their default keys.
    fn act(state: &mut AppState, actions: &[Box<dyn Action>], code: KeyCode) -> Vec<Cmd> {
        let keymap = Keymap::with_actions(actions, &std::collections::BTreeMap::new()).unwrap();
        act_with(state, actions, &keymap, code)
    }

    fn act_with(
        state: &mut AppState,
        actions: &[Box<dyn Action>],
        keymap: &Keymap,
        code: KeyCode,
    ) -> Vec<Cmd> {
        update_full(
            state,
            keymap,
            &TABS,
            &PANES,
            actions,
            press(code, KeyModifiers::NONE),
        )
    }

    /// The selection of `#number` of `chaoshub` for the login
    /// `chaosteil`.
    fn pr_selection(state: &AppState, number: u64) -> Box<Selection> {
        Box::new(Selection {
            target: pr_target(&state.data, &testdata::pr_key("chaoshub", number)).unwrap(),
            me: "chaosteil".into(),
            action_config: None,
        })
    }

    fn fake(action: FakeAction) -> Vec<Box<dyn Action>> {
        vec![Box::new(action)]
    }

    #[test]
    fn a_run_action_becomes_run_action_with_the_selection() {
        let mut state = action_state(&pr_row(102));
        let actions = fake(FakeAction::new("approve", 'A'));
        let table: toml::Table = toml::from_str("body = \"lgtm\"").unwrap();
        state
            .data
            .action_config
            .insert("approve".into(), table.clone());
        let mut selection = pr_selection(&state, 102);
        selection.action_config = Some(table);
        assert_eq!(
            act(&mut state, &actions, KeyCode::Char('A')),
            vec![Cmd::RunAction {
                id: "approve",
                selection,
                input: ActionInput::None,
            }]
        );
        assert!(state.ui.overlays.is_empty());
    }

    #[test]
    fn a_terminal_and_a_command_route_without_execute() {
        let mut state = action_state(&pr_row(99));
        let request = crate::bus::LaunchRequest {
            argv: vec!["nvim".into()],
            cwd: "/code/chaoshub".into(),
            env: Vec::new(),
            label: "chaoshub-review".into(),
            zoom: true,
            split: None,
            ratio: None,
        };

        let mut review = FakeAction::new("review", 'v');
        review.prepare = Prepare::Terminal(request.clone());
        let mut browser = FakeAction::new("browser", 'o');
        browser.prepare = Prepare::Command(Cmd::OpenUrl("https://example.com".into()));
        let actions: Vec<Box<dyn Action>> = vec![Box::new(review), Box::new(browser)];
        assert_eq!(
            act(&mut state, &actions, KeyCode::Char('v')),
            vec![Cmd::RunTerminal {
                id: "review",
                key: TargetKey::Pr(testdata::pr_key("chaoshub", 99)),
                request,
            }]
        );
        assert_eq!(
            act(&mut state, &actions, KeyCode::Char('o')),
            vec![Cmd::OpenUrl("https://example.com".into())]
        );
    }

    #[test]
    fn disabled_shows_the_reason_and_hidden_passes_the_key_on() {
        let mut state = action_state(&pr_row(102));
        let mut merge = FakeAction::new("merge", 'm');
        merge.applies = Applicability::Disabled("stack not loaded");
        let mut hidden = FakeAction::new("hidden", 'h');
        hidden.applies = Applicability::Hidden;
        let actions: Vec<Box<dyn Action>> = vec![Box::new(merge), Box::new(hidden)];
        assert_eq!(act(&mut state, &actions, KeyCode::Char('m')), vec![]);
        assert_eq!(
            state.ui.status.as_ref().map(|msg| msg.text.as_str()),
            Some("stack not loaded")
        );
        state.ui.status = None;
        assert_eq!(act(&mut state, &actions, KeyCode::Char('h')), vec![]);
        assert_eq!(state.ui.status, None, "a hidden action says nothing");
    }

    #[test]
    fn a_header_row_runs_no_action() {
        let mut state = action_state(&|_| RowItem::StackHeader {
            stack_id: "S_1".into(),
        });
        state.ui.tabs[0].cursor = 0;
        let actions = fake(FakeAction::new("approve", 'A'));
        let model = frame::active_list(&mut state, &TABS);
        assert!(matches!(
            model.rows[model.cursor].item,
            RowItem::StackHeader { .. }
        ));
        assert_eq!(act(&mut state, &actions, KeyCode::Char('A')), vec![]);
    }

    #[test]
    fn a_confirm_opens_the_popup_and_y_or_enter_runs() {
        let mut state = action_state(&pr_row(102));
        let spec = crate::bus::ConfirmSpec {
            title: "merge #102?".into(),
            lines: vec!["merges 2 pull requests atomically".into()],
            danger: true,
        };
        let mut merge = FakeAction::new("merge", 'm');
        merge.prepare = Prepare::Confirm(spec.clone());
        let actions = fake(merge);
        for confirm in [KeyCode::Char('y'), KeyCode::Enter] {
            assert_eq!(act(&mut state, &actions, KeyCode::Char('m')), vec![]);
            let overlays = &frame::frame_model(&mut state).overlays;
            assert_eq!(
                overlays.last().map(|o| &o.content),
                Some(&OverlayContent::Confirm(spec.clone()))
            );
            assert_eq!(act(&mut state, &actions, KeyCode::Char('j')), vec![]);
            assert_eq!(
                act(&mut state, &actions, confirm),
                vec![Cmd::RunAction {
                    id: "merge",
                    selection: pr_selection(&state, 102),
                    input: ActionInput::Confirmed,
                }]
            );
            assert!(state.ui.overlays.iter().all(|o| o.closing));
            run_msg(&mut state, &actions, Msg::TransitionsDone);
        }
        act(&mut state, &actions, KeyCode::Char('m'));
        assert_eq!(act(&mut state, &actions, KeyCode::Esc), vec![]);
        assert!(state.ui.overlays.iter().all(|o| o.closing));
    }

    /// The action state on `#number`, with the sidebar open on it.
    fn sidebar_state(number: u64, actions: &[Box<dyn Action>]) -> AppState {
        let mut state = action_state(&pr_row(number));
        assert_eq!(act(&mut state, actions, KeyCode::Enter), vec![]);
        assert!(sidebar(&state).is_some());
        state
    }

    /// The pull request popup on top of the overlay stack, when it is not
    /// closing.
    fn sidebar(state: &AppState) -> Option<&PopupState> {
        match state.ui.overlays.iter().rev().find(|o| !o.closing) {
            Some(Overlay {
                state: OverlayState::Popup(popup),
                ..
            }) => Some(popup),
            _ => None,
        }
    }

    #[test]
    fn v_in_the_sidebar_runs_the_terminal_action() {
        let request = crate::bus::LaunchRequest {
            argv: vec!["nvim".into()],
            cwd: "/code/chaoshub".into(),
            env: Vec::new(),
            label: "chaoshub-review".into(),
            zoom: true,
            split: None,
            ratio: None,
        };

        let mut review = FakeAction::new("review", 'v');
        review.prepare = Prepare::Terminal(request.clone());
        let actions = fake(review);
        let mut state = sidebar_state(99, &actions);
        assert_eq!(
            act(&mut state, &actions, KeyCode::Char('v')),
            vec![Cmd::RunTerminal {
                id: "review",
                key: TargetKey::Pr(testdata::pr_key("chaoshub", 99)),
                request,
            }]
        );
        assert!(sidebar(&state).is_some(), "the sidebar stays open");
    }

    #[test]
    fn a_confirm_in_the_sidebar_opens_on_top_of_it() {
        let spec = crate::bus::ConfirmSpec {
            title: "merge #102?".into(),
            lines: Vec::new(),
            danger: true,
        };
        let mut merge = FakeAction::new("merge", 'm');
        merge.prepare = Prepare::Confirm(spec.clone());
        let actions = fake(merge);
        let mut state = sidebar_state(102, &actions);
        assert_eq!(act(&mut state, &actions, KeyCode::Char('m')), vec![]);
        let kinds: Vec<(OverlayKind, bool)> = state
            .ui
            .overlays
            .iter()
            .map(|o| (o.kind, o.closing))
            .collect();
        assert_eq!(
            kinds,
            [(OverlayKind::Popup, false), (OverlayKind::Confirm, false)]
        );
        assert_eq!(
            frame::frame_model(&mut state)
                .overlays
                .last()
                .map(|o| &o.content),
            Some(&OverlayContent::Confirm(spec))
        );
        assert_eq!(
            act(&mut state, &actions, KeyCode::Char('y')),
            vec![Cmd::RunAction {
                id: "merge",
                selection: pr_selection(&state, 102),
                input: ActionInput::Confirmed,
            }]
        );
        assert!(sidebar(&state).is_some(), "the sidebar stays open");
    }

    #[test]
    fn a_key_without_an_action_does_nothing_in_the_sidebar() {
        let actions = fake(FakeAction::new("approve", 'A'));
        let mut state = sidebar_state(102, &actions);
        let generation = state.generation;
        assert_eq!(act(&mut state, &actions, KeyCode::Char('z')), vec![]);
        assert_eq!(state.generation, generation);
        assert_eq!(state.ui.overlays.len(), 1);
        assert!(sidebar(&state).is_some());
    }

    #[test]
    fn a_pane_key_in_the_sidebar_selects_the_pane_before_an_action() {
        let actions = fake(FakeAction::new("approve", '2'));
        let mut state = sidebar_state(102, &actions);
        assert_eq!(sidebar(&state).map(|popup| popup.active), Some(0));
        state.ui.ui_events.clear();
        assert_eq!(act(&mut state, &actions, KeyCode::Char('2')), vec![]);
        assert_eq!(sidebar(&state).map(|popup| popup.active), Some(1));
        assert_eq!(
            state.ui.ui_events,
            vec![UiEvent::PaneTabChanged { from: 0, to: 1 }]
        );
        state.ui.ui_events.clear();
        act(&mut state, &actions, KeyCode::Char('2'));
        assert_eq!(state.ui.ui_events, vec![], "the same pane: no event");
        act(&mut state, &actions, KeyCode::Tab);
        assert!(
            matches!(
                state.ui.ui_events.as_slice(),
                [UiEvent::PaneTabChanged { from: 1, .. }]
            ),
            "next_tab: {:?}",
            state.ui.ui_events
        );
    }

    #[test]
    fn n_cancels_a_confirm() {
        let mut state = action_state(&pr_row(102));
        let mut close = FakeAction::new("close", 'X');
        close.prepare = Prepare::Confirm(crate::bus::ConfirmSpec {
            title: "close #102?".into(),
            lines: Vec::new(),
            danger: true,
        });
        let actions = fake(close);
        assert_eq!(act(&mut state, &actions, KeyCode::Char('X')), vec![]);
        assert!(state.ui.overlays.iter().any(|o| !o.closing));
        assert_eq!(act(&mut state, &actions, KeyCode::Char('n')), vec![]);
        assert!(state.ui.overlays.iter().all(|o| o.closing));
    }

    #[test]
    fn n_is_text_in_a_prompt() {
        let mut state = action_state(&pr_row(102));
        let spec = crate::bus::PromptSpec {
            title: "request changes".into(),
            placeholder: "why".into(),
        };
        let mut request = FakeAction::new("request_changes", 'R');
        request.prepare = Prepare::Prompt(spec.clone());
        let actions = fake(request);
        act(&mut state, &actions, KeyCode::Char('R'));
        assert_eq!(act(&mut state, &actions, KeyCode::Char('n')), vec![]);
        let overlays = &frame::frame_model(&mut state).overlays;
        assert_eq!(
            overlays.last().map(|o| &o.content),
            Some(&OverlayContent::Prompt {
                spec,
                text: "n".into(),
            })
        );
        assert_eq!(act(&mut state, &actions, KeyCode::Esc), vec![]);
        assert!(state.ui.overlays.iter().all(|o| o.closing));
    }

    fn run_msg(state: &mut AppState, actions: &[Box<dyn Action>], msg: Msg) -> Vec<Cmd> {
        update_full(state, &Keymap::default(), &TABS, &PANES, actions, msg)
    }

    #[test]
    fn a_prompt_collects_text_and_enter_runs() {
        let mut state = action_state(&pr_row(102));
        let spec = crate::bus::PromptSpec {
            title: "request changes".into(),
            placeholder: "why".into(),
        };
        let mut request = FakeAction::new("request_changes", 'R');
        request.prepare = Prepare::Prompt(spec.clone());
        let actions = fake(request);
        assert_eq!(act(&mut state, &actions, KeyCode::Char('R')), vec![]);
        for c in "fix qx".chars() {
            assert_eq!(act(&mut state, &actions, KeyCode::Char(c)), vec![]);
        }
        assert_eq!(act(&mut state, &actions, KeyCode::Backspace), vec![]);
        let overlays = &frame::frame_model(&mut state).overlays;
        assert_eq!(
            overlays.last().map(|o| &o.content),
            Some(&OverlayContent::Prompt {
                spec: spec.clone(),
                text: "fix q".into(),
            })
        );
        assert_eq!(
            act(&mut state, &actions, KeyCode::Enter),
            vec![Cmd::RunAction {
                id: "request_changes",
                selection: pr_selection(&state, 102),
                input: ActionInput::Text("fix q".into()),
            }]
        );
        run_msg(&mut state, &actions, Msg::TransitionsDone);
        act(&mut state, &actions, KeyCode::Char('R'));
        assert_eq!(act(&mut state, &actions, KeyCode::Esc), vec![]);
        assert!(state.ui.overlays.iter().all(|o| o.closing));
    }

    #[test]
    fn keys_override_the_default_key_of_an_action() {
        let mut state = action_state(&pr_row(102));
        let actions = fake(FakeAction::new("approve", 'A'));
        let overrides =
            std::collections::BTreeMap::from([("approve".to_owned(), "ctrl-a".to_owned())]);
        let keymap = Keymap::with_actions(&actions, &overrides).unwrap();
        assert_eq!(
            act_with(&mut state, &actions, &keymap, KeyCode::Char('A')),
            vec![]
        );
        let ctrl_a = press(KeyCode::Char('a'), KeyModifiers::CONTROL);
        let cmds = update_full(&mut state, &keymap, &TABS, &PANES, &actions, ctrl_a);
        assert!(matches!(
            cmds.as_slice(),
            [Cmd::RunAction { id: "approve", .. }]
        ));
    }

    /// The flags cell of the row of `#number` of `chaoshub`.
    fn flags_of(state: &mut AppState, number: u64) -> String {
        let model = frame::active_list(state, &TABS);
        let row = model
            .rows
            .iter()
            .find(|row| matches!(&row.item, RowItem::Pr(pr) if pr.key.number == number))
            .unwrap();
        row.cells.last().unwrap().text.clone()
    }

    #[test]
    fn one_pane_per_key_and_a_second_press_focuses_it() {
        let mut state = action_state(&pr_row(99));
        let request = crate::bus::LaunchRequest {
            argv: vec!["nvim".into()],
            cwd: "/code/chaoshub".into(),
            env: Vec::new(),
            label: "chaoshub-review".into(),
            zoom: true,
            split: None,
            ratio: None,
        };

        let mut review = FakeAction::new("review", 'v');
        review.prepare = Prepare::Terminal(request.clone());
        let actions = fake(review);
        let key = TargetKey::Pr(testdata::pr_key("chaoshub", 99));
        let run_terminal = Cmd::RunTerminal {
            id: "review",
            key: key.clone(),
            request,
        };
        assert_eq!(
            act(&mut state, &actions, KeyCode::Char('v')),
            vec![run_terminal.clone()]
        );
        let pane = ("review", key.clone());
        assert_eq!(
            state.data.panes.get(&pane),
            Some(&PaneHandleState::Starting)
        );
        assert_eq!(flags_of(&mut state, 99), "▶");
        let focus = Cmd::FocusPane {
            id: "review",
            key: key.clone(),
        };
        assert_eq!(
            act(&mut state, &actions, KeyCode::Char('v')),
            vec![focus.clone()]
        );
        let started = Msg::PaneStarted {
            id: "review",
            key: key.clone(),
        };
        assert_eq!(run_msg(&mut state, &actions, started), vec![]);
        assert_eq!(state.data.panes.get(&pane), Some(&PaneHandleState::Running));
        assert_eq!(act(&mut state, &actions, KeyCode::Char('v')), vec![focus]);
        let ended = Msg::PaneEnded {
            id: "review",
            key: key.clone(),
        };
        let refs = vec![pr_ref(&state.data.prs[&testdata::pr_key("chaoshub", 99)])];
        assert_eq!(
            run_msg(&mut state, &actions, ended),
            vec![
                Cmd::ScanJj(testdata::repo_key("chaoshub")),
                Cmd::RefreshPr(refs),
                Cmd::LoadCodeview(testdata::pr_key("chaoshub", 99)),
            ]
        );
        assert!(state.data.panes.is_empty());
        assert_eq!(flags_of(&mut state, 99), "");
        assert_eq!(
            act(&mut state, &actions, KeyCode::Char('v')),
            vec![run_terminal]
        );
    }

    fn event(id: &'static str, number: u64, event: ActionEvent) -> Msg {
        Msg::ActionEvent {
            id,
            key: TargetKey::Pr(testdata::pr_key("chaoshub", number)),
            event,
        }
    }

    fn lines(range: std::ops::Range<usize>) -> Vec<String> {
        range.map(|i| format!("line {i}")).collect()
    }

    /// The title and the lines of each output popup that is not closing.
    fn outputs(state: &AppState) -> Vec<(String, Vec<String>)> {
        state
            .ui
            .overlays
            .iter()
            .filter(|o| !o.closing)
            .filter_map(|o| match &o.state {
                OverlayState::Output { title, lines, .. } => Some((title.clone(), lines.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn output_lines_open_one_popup_per_action_and_target() {
        let mut state = action_state(&pr_row(102));
        let actions = fake(FakeAction::new("agent", 'g'));
        let out = |range| ActionEvent::Output(lines(range));
        assert_eq!(
            run_msg(&mut state, &actions, event("agent", 102, out(0..2))),
            vec![]
        );
        assert_eq!(
            state.ui.ui_events.last(),
            Some(&UiEvent::OverlayOpened(OverlayKind::Output))
        );
        let generation = state.generation;
        run_msg(&mut state, &actions, event("agent", 102, out(2..3)));
        assert!(state.generation > generation);
        assert_eq!(outputs(&state), [("agent output".to_owned(), lines(0..3))]);
        let model = frame::frame_model(&mut state).overlays.last().cloned();
        assert_eq!(
            model.map(|o| o.content),
            Some(OverlayContent::Output {
                title: "agent output".into(),
                lines: lines(0..3),
                scroll: 2,
            })
        );
        run_msg(&mut state, &actions, event("agent", 101, out(0..1)));
        run_msg(&mut state, &actions, event("other", 102, out(0..1)));
        assert_eq!(
            outputs(&state)
                .iter()
                .map(|(t, _)| t.as_str())
                .collect::<Vec<_>>(),
            ["agent output", "agent output", "other output"]
        );
    }

    #[test]
    fn an_output_popup_keeps_the_last_2000_lines() {
        let mut state = action_state(&pr_row(102));
        let actions = fake(FakeAction::new("agent", 'g'));
        let out = |range| event("agent", 102, ActionEvent::Output(lines(range)));
        run_msg(&mut state, &actions, out(0..1500));
        run_msg(&mut state, &actions, out(1500..2600));
        assert_eq!(
            outputs(&state),
            [("agent output".to_owned(), lines(600..2600))]
        );
        let mut fresh = action_state(&pr_row(102));
        run_msg(&mut fresh, &actions, out(0..2001));
        assert_eq!(outputs(&fresh)[0].1, lines(1..2001));
    }

    #[test]
    fn a_closed_output_popup_opens_again_on_new_lines() {
        let mut state = action_state(&pr_row(102));
        let actions = fake(FakeAction::new("agent", 'g'));
        let out = |range| event("agent", 102, ActionEvent::Output(lines(range)));
        run_msg(&mut state, &actions, out(0..2));
        assert_eq!(act(&mut state, &actions, KeyCode::Esc), vec![]);
        assert_eq!(outputs(&state), []);
        run_msg(&mut state, &actions, out(2..3));
        assert_eq!(outputs(&state), [("agent output".to_owned(), lines(2..3))]);
    }

    #[test]
    fn progress_and_the_result_show_in_the_status_line() {
        let mut state = action_state(&pr_row(102));
        let actions = fake(FakeAction::new("rerun_checks", 'x'));
        let status = |state: &AppState| {
            state
                .ui
                .status
                .as_ref()
                .map(|msg| (msg.text.clone(), msg.style))
        };
        let progress = event("rerun_checks", 102, ActionEvent::Progress("2 runs".into()));
        assert_eq!(run_msg(&mut state, &actions, progress), vec![]);
        assert_eq!(status(&state), Some(("2 runs".into(), CellStyle::Info)));
        let follow = vec![
            Cmd::ScanJj(testdata::repo_key("chaoshub")),
            Cmd::RefreshPr(stack_group_refs(
                &state.data,
                &testdata::pr_key("chaoshub", 102),
            )),
        ];
        let ok = event(
            "rerun_checks",
            102,
            ActionEvent::Done(Ok("re-running".into())),
        );
        assert_eq!(run_msg(&mut state, &actions, ok), follow);
        assert_eq!(
            status(&state),
            Some(("re-running".into(), CellStyle::Success))
        );
        let err = event(
            "rerun_checks",
            102,
            ActionEvent::Done(Err("offline".into())),
        );
        assert_eq!(run_msg(&mut state, &actions, err), follow);
        assert_eq!(
            status(&state),
            Some(("rerun_checks failed: offline".into(), CellStyle::Danger))
        );
        state.ui.status = None;
        let quiet = event("rerun_checks", 102, ActionEvent::Done(Ok(String::new())));
        assert_eq!(run_msg(&mut state, &actions, quiet), follow);
        assert_eq!(status(&state), None, "an empty result says nothing");
    }

    #[test]
    fn a_successful_action_flashes_its_row() {
        let mut state = action_state(&pr_row(102));
        let actions = fake(FakeAction::new("rerun_checks", 'x'));
        let key = testdata::pr_key("chaoshub", 102);
        state.ui.ui_events.clear();
        let err = event(
            "rerun_checks",
            102,
            ActionEvent::Done(Err("offline".into())),
        );
        run_msg(&mut state, &actions, err);
        assert_eq!(state.ui.ui_events, vec![], "a failure does not flash");
        let generation = state.generation;
        let quiet = event("rerun_checks", 102, ActionEvent::Done(Ok(String::new())));
        run_msg(&mut state, &actions, quiet);
        assert_eq!(state.ui.ui_events, vec![UiEvent::RowFlash(key)]);
        assert!(state.generation > generation, "the flash needs a frame");
    }

    #[test]
    fn a_failed_pane_launch_shows_its_error() {
        let mut state = action_state(&pr_row(99));
        let actions: Vec<Box<dyn Action>> = Vec::new();
        let key = TargetKey::Pr(testdata::pr_key("chaoshub", 99));
        state
            .data
            .panes
            .insert(("review", key.clone()), PaneHandleState::Starting);
        let failed = event(
            "review",
            99,
            ActionEvent::Done(Err("cannot open the pane: cannot start rex: gone".into())),
        );
        let cmds = run_msg(&mut state, &actions, failed);
        assert_eq!(
            state.ui.status.as_ref().map(|msg| msg.text.as_str()),
            Some("review failed: cannot open the pane: cannot start rex: gone")
        );
        assert_eq!(cmds, follow_up(&state.data, &key), "one follow-up");
        assert!(
            state.data.panes.is_empty(),
            "the pane that never started is gone"
        );
    }

    /// A mouse event of `kind` on `hit`.
    fn mouse_on(kind: MouseKind, hit: Option<Hit>) -> Msg {
        Msg::Mouse {
            kind,
            x: 0,
            y: 0,
            hit,
        }
    }

    fn click_on(hit: Hit) -> Msg {
        mouse_on(MouseKind::Click, Some(hit))
    }

    fn wheel_down() -> Msg {
        mouse_on(MouseKind::ScrollDown, Some(Hit::Inside))
    }

    fn wheel_up() -> Msg {
        mouse_on(MouseKind::ScrollUp, None)
    }

    /// Sends `msg` with the real tabs, the panes of `panes` and the
    /// default keys.
    fn send_with(state: &mut AppState, panes: &[Box<dyn Pane>], msg: Msg) -> Vec<Cmd> {
        update_full(state, &Keymap::default(), &TABS, panes, &REGISTRY, msg)
    }

    #[test]
    fn the_wheel_moves_the_list_cursor_three_rows() {
        // Rows: #99, header, #103, #102, #101, #7.
        let mut state = mine_state(10);
        let cmds = update(&mut state, &Keymap::default(), wheel_down());
        assert_eq!(selected_number(&mut state), Some(101));
        assert!(matches!(cmds.as_slice(), [Cmd::SetInterest(_)]));
        update(&mut state, &Keymap::default(), wheel_down());
        assert_eq!(selected_number(&mut state), Some(7), "the last row");
        update(&mut state, &Keymap::default(), wheel_up());
        assert_eq!(selected_number(&mut state), Some(103));
    }

    #[test]
    fn the_wheel_scrolls_a_pane_of_lines_in_the_sidebar() {
        let mut state = mine_state(10);
        let key99 = testdata::pr_key("chaoshub", 99);
        let body: Vec<String> = (0..40).map(|i| format!("paragraph {i}")).collect();
        state.data.prs.get_mut(&key99).unwrap().head.value.body = body.join("\n\n");
        state.bump();
        update(
            &mut state,
            &Keymap::default(),
            press(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(
            sidebar(&state).map(|p| p.active),
            Some(0),
            "the description"
        );
        update(&mut state, &Keymap::default(), wheel_down());
        assert!(matches!(
            popup_content(&mut state, &PANES),
            OverlayContent::Popup {
                scroll: 3,
                cursor: 0,
                ..
            }
        ));
        update(&mut state, &Keymap::default(), wheel_up());
        assert!(matches!(
            popup_content(&mut state, &PANES),
            OverlayContent::Popup { scroll: 0, .. }
        ));
        assert_eq!(selected_number(&mut state), Some(99), "the list stays");
    }

    #[test]
    fn the_wheel_moves_the_cursor_of_a_tree_pane_in_the_sidebar() {
        let panes = fake_panes();
        let mut state = mine_state(10);
        send_with(
            &mut state,
            &panes,
            press(KeyCode::Enter, KeyModifiers::NONE),
        );
        send_with(
            &mut state,
            &panes,
            press(KeyCode::Char('2'), KeyModifiers::NONE),
        );
        assert_eq!(send_with(&mut state, &panes, wheel_down()), vec![]);
        assert!(matches!(
            popup_content(&mut state, &panes),
            OverlayContent::Popup { cursor: 3, .. }
        ));
        send_with(
            &mut state,
            &panes,
            press(KeyCode::Char('1'), KeyModifiers::NONE),
        );
        send_with(&mut state, &panes, wheel_down());
        assert!(
            matches!(
                popup_content(&mut state, &panes),
                OverlayContent::Popup { cursor: 2, .. }
            ),
            "pane a has three nodes"
        );
    }

    /// The scroll of the top help or output overlay.
    fn text_scroll(state: &AppState) -> Option<usize> {
        match state.ui.overlays.iter().rev().find(|o| !o.closing)?.state {
            OverlayState::Help { scroll, .. } | OverlayState::Output { scroll, .. } => Some(scroll),
            _ => None,
        }
    }

    #[test]
    fn the_wheel_and_the_motion_keys_scroll_the_help() {
        let mut state = mine_state(10);
        key(&mut state, '?');
        assert_eq!(text_scroll(&state), Some(0));
        update(&mut state, &Keymap::default(), wheel_down());
        assert_eq!(text_scroll(&state), Some(3));
        key(&mut state, 'j');
        assert_eq!(text_scroll(&state), Some(4));
        key(&mut state, 'G');
        let OverlayState::Help { help, .. } = &state.ui.overlays[0].state else {
            panic!("not the help");
        };
        let last = help.line_count() - 1;
        assert_eq!(text_scroll(&state), Some(last));
        update(&mut state, &Keymap::default(), wheel_up());
        assert_eq!(text_scroll(&state), Some(last - 3));
        key(&mut state, 'g');
        assert_eq!(text_scroll(&state), Some(0));
        assert_eq!(selected_number(&mut state), Some(99), "the list stays");
    }

    #[test]
    fn an_output_popup_follows_the_tail_until_it_scrolls_up() {
        let mut state = action_state(&pr_row(102));
        let actions = fake(FakeAction::new("agent", 'g'));
        let out = |range| event("agent", 102, ActionEvent::Output(lines(range)));
        run_msg(&mut state, &actions, out(0..10));
        assert_eq!(text_scroll(&state), Some(9), "on the last line");
        run_msg(&mut state, &actions, out(10..12));
        assert_eq!(text_scroll(&state), Some(11), "it follows");
        run_msg(&mut state, &actions, wheel_up());
        assert_eq!(text_scroll(&state), Some(8));
        run_msg(&mut state, &actions, out(12..14));
        assert_eq!(text_scroll(&state), Some(8), "it stays");
        run_msg(
            &mut state,
            &actions,
            press(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        assert_eq!(text_scroll(&state), Some(7));
        run_msg(&mut state, &actions, wheel_down());
        assert_eq!(text_scroll(&state), Some(10));
    }

    #[test]
    fn a_click_on_a_tab_switches_to_it() {
        let mut state = mine_state(10);
        let cmds = update(&mut state, &Keymap::default(), click_on(Hit::Tab(1)));
        assert_eq!(state.ui.active, 1);
        assert!(matches!(cmds.as_slice(), [Cmd::SetInterest(_)]));
        assert_eq!(
            state.ui.ui_events.last(),
            Some(&UiEvent::TabChanged { from: 0, to: 1 })
        );
        let generation = state.generation;
        update(&mut state, &Keymap::default(), click_on(Hit::Tab(8)));
        assert_eq!(state.ui.active, 1, "no ninth tab");
        assert_eq!(state.generation, generation);
    }

    #[test]
    fn a_click_selects_a_row_and_a_second_click_opens_the_sidebar() {
        // Rows: #99, header, #103, #102, #101, #7.
        let mut state = mine_state(10);
        let generation = state.generation;
        assert_eq!(
            update(&mut state, &Keymap::default(), click_on(Hit::Row(1))),
            vec![]
        );
        assert_eq!(state.generation, generation, "a header is not selectable");
        assert_eq!(selected_number(&mut state), Some(99));
        let cmds = update(&mut state, &Keymap::default(), click_on(Hit::Row(3)));
        assert_eq!(selected_number(&mut state), Some(102));
        assert!(matches!(cmds.as_slice(), [Cmd::SetInterest(_)]));
        assert!(state.ui.overlays.is_empty());
        update(&mut state, &Keymap::default(), click_on(Hit::Row(3)));
        let popup = sidebar(&state).map(|p| p.key.clone());
        assert_eq!(popup, Some(testdata::pr_key("chaoshub", 102)));
        let generation = state.generation;
        update(&mut state, &Keymap::default(), click_on(Hit::Row(9)));
        assert_eq!(state.generation, generation, "no such row");
    }

    #[test]
    fn a_click_on_a_key_hint_presses_the_key() {
        let mut state = action_state(&pr_row(102));
        let actions = fake(FakeAction::new("approve", 'A'));
        let keymap = Keymap::with_actions(&actions, &std::collections::BTreeMap::new()).unwrap();
        let chord = |code| KeyChord {
            code,
            mods: KeyModifiers::NONE,
        };
        let msg = click_on(Hit::Key(chord(KeyCode::Char('A'))));
        let cmds = update_full(&mut state, &keymap, &TABS, &PANES, &actions, msg);
        assert_eq!(
            cmds,
            vec![Cmd::RunAction {
                id: "approve",
                selection: pr_selection(&state, 102),
                input: ActionInput::None,
            }]
        );
        let msg = click_on(Hit::Key(chord(KeyCode::Char('?'))));
        update_full(&mut state, &keymap, &TABS, &PANES, &actions, msg);
        assert_eq!(text_scroll(&state), Some(0), "the help opens");
        let msg = click_on(Hit::Key(chord(KeyCode::Esc)));
        assert_eq!(
            update_full(&mut state, &keymap, &TABS, &PANES, &actions, msg),
            vec![]
        );
        assert!(
            state.ui.overlays.iter().all(|o| o.closing),
            "Esc closes the help"
        );
    }

    #[test]
    fn a_click_on_a_pane_tab_makes_that_pane_active() {
        let panes = fake_panes();
        let mut state = mine_state(10);
        send_with(
            &mut state,
            &panes,
            press(KeyCode::Enter, KeyModifiers::NONE),
        );
        state.ui.ui_events.clear();
        send_with(&mut state, &panes, click_on(Hit::PaneTab(1)));
        assert_eq!(sidebar(&state).map(|p| p.active), Some(1));
        assert_eq!(
            state.ui.ui_events,
            vec![UiEvent::PaneTabChanged { from: 0, to: 1 }]
        );
        let generation = state.generation;
        send_with(&mut state, &panes, click_on(Hit::PaneTab(2)));
        assert_eq!(
            sidebar(&state).map(|p| p.active),
            Some(1),
            "pane c does not apply, so the sidebar has two tabs"
        );
        assert_eq!(state.generation, generation);
    }

    /// A tree pane of three nodes. The `open` key returns
    /// `Cmd::OpenUrl` with the cursor node.
    struct OpenPane;

    impl Pane for OpenPane {
        fn id(&self) -> &'static str {
            "open"
        }

        fn title(&self) -> &'static str {
            "open"
        }

        fn key(&self) -> char {
            '1'
        }

        fn applies(&self, _target: &Target) -> bool {
            true
        }

        fn init_state(&self) -> PaneState {
            PaneState::default()
        }

        fn model(&self, _cx: &KeyCx<'_>, _ps: &PaneState, _size: Size) -> PaneModel {
            PaneModel::Tree {
                nodes: (0..3)
                    .map(|_| crate::app::models::TreeNode {
                        depth: 0,
                        expanded: Some(false),
                        cells: Vec::new(),
                    })
                    .collect(),
            }
        }

        fn handle_key(&self, key: KeyChord, cx: &KeyCx<'_>, ps: &mut PaneState) -> Option<Handled> {
            cx.keys.matches("open", key).then(|| Handled {
                cmds: vec![Cmd::OpenUrl(format!("node {}", ps.cursor))],
                ..Handled::default()
            })
        }
    }

    #[test]
    fn a_click_on_a_pane_row_moves_the_cursor_and_a_second_click_expands() {
        let panes: Vec<Box<dyn Pane>> = vec![Box::new(OpenPane)];
        let mut state = mine_state(10);
        send_with(
            &mut state,
            &panes,
            press(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(
            send_with(&mut state, &panes, click_on(Hit::PaneRow(0))),
            vec![Cmd::OpenUrl("node 0".into())],
            "the cursor node opens"
        );
        assert_eq!(
            send_with(&mut state, &panes, click_on(Hit::PaneRow(2))),
            vec![]
        );
        assert!(matches!(
            popup_content(&mut state, &panes),
            OverlayContent::Popup { cursor: 2, .. }
        ));
        assert_eq!(
            send_with(&mut state, &panes, click_on(Hit::PaneRow(2))),
            vec![Cmd::OpenUrl("node 2".into())]
        );
        assert_eq!(
            send_with(&mut state, &panes, click_on(Hit::PaneRow(7))),
            vec![],
            "no such node"
        );
    }

    #[test]
    fn a_click_on_a_table_row_moves_the_pane_cursor() {
        let mut state = mine_state(10);
        let key99 = testdata::pr_key("chaoshub", 99);
        state.data.prs.get_mut(&key99).unwrap().checks.value = testdata::checks();
        state.bump();
        update(
            &mut state,
            &Keymap::default(),
            press(KeyCode::Enter, KeyModifiers::NONE),
        );
        update(
            &mut state,
            &Keymap::default(),
            press(KeyCode::Char('2'), KeyModifiers::NONE),
        );
        let OverlayContent::Popup {
            pane: PaneModel::Table { rows, .. },
            ..
        } = popup_content(&mut state, &PANES)
        else {
            panic!("the checks pane is a table");
        };
        assert!(rows.len() > 1, "the checks of the fixture");
        let last = rows.len() - 1;
        update(&mut state, &Keymap::default(), click_on(Hit::PaneRow(last)));
        assert!(matches!(
            popup_content(&mut state, &PANES),
            OverlayContent::Popup { cursor, .. } if cursor == last
        ));
    }

    #[test]
    fn a_click_on_a_picker_item_picks_it() {
        let mut state = team_state();
        key(&mut state, 'a');
        let cmds = update(&mut state, &Keymap::default(), click_on(Hit::Item(1)));
        assert_eq!(cmds[0], persist("author", "team", "bob".into()));
        assert_eq!(picker(&state), None, "the picker closes");
        assert_eq!(state.ui.tabs[2].author.as_deref(), Some("bob"));
        key(&mut state, 'a');
        update(&mut state, &Keymap::default(), click_on(Hit::Item(0)));
        assert_eq!(state.ui.tabs[2].author, None, "all authors");
        key(&mut state, 'a');
        update(&mut state, &Keymap::default(), click_on(Hit::Item(9)));
        assert!(picker(&state).is_some(), "no such item");
    }

    #[test]
    fn a_click_outside_closes_the_top_overlay_and_inside_does_nothing() {
        let mut state = mine_state(10);
        update(
            &mut state,
            &Keymap::default(),
            press(KeyCode::Enter, KeyModifiers::NONE),
        );
        let generation = state.generation;
        assert_eq!(
            update(&mut state, &Keymap::default(), click_on(Hit::Inside)),
            vec![]
        );
        assert_eq!(state.generation, generation);
        assert!(sidebar(&state).is_some());
        assert_eq!(
            update(&mut state, &Keymap::default(), click_on(Hit::Outside)),
            vec![]
        );
        assert!(sidebar(&state).is_none());
        assert_eq!(
            state.ui.ui_events.last(),
            Some(&UiEvent::OverlayClosed(OverlayKind::Popup))
        );
    }

    #[test]
    fn a_click_outside_a_confirm_cancels_it() {
        let mut state = action_state(&pr_row(102));
        let mut merge = FakeAction::new("merge", 'm');
        merge.prepare = Prepare::Confirm(crate::bus::ConfirmSpec {
            title: "merge #102?".into(),
            lines: Vec::new(),
            danger: true,
        });
        let actions = fake(merge);
        act(&mut state, &actions, KeyCode::Char('m'));
        assert_eq!(
            run_msg(&mut state, &actions, click_on(Hit::Outside)),
            vec![]
        );
        assert!(state.ui.overlays.iter().all(|o| o.closing));
    }

    /// The real tabs with the mine fixtures, the session `repos` and the
    /// login `chaosteil`, pinned to `key`.
    fn pinned_with(key: PrKey, repos: Vec<Repo>) -> AppState {
        let mut state = AppState::new(SIZE);
        state.load(testdata::mine_snapshot(), repos);
        state.data.session.login = Some("chaosteil".into());
        state.data.now = test_time() + TimeDelta::minutes(5);
        pin(&mut state, key);
        state
    }

    fn pinned(key: PrKey) -> AppState {
        pinned_with(key, Vec::new())
    }

    /// A pull request of `chaoshub` that is not in the mine fixtures.
    fn not_cached() -> PrKey {
        testdata::pr_key("chaoshub", 500)
    }

    #[test]
    fn the_pinned_app_has_one_popup_and_no_open_event() {
        let state = pinned(testdata::pr_key("codeview", 7));
        assert_eq!(state.ui.overlays.len(), 1);
        let overlay = &state.ui.overlays[0];
        assert_eq!(overlay.kind, OverlayKind::Popup);
        assert!(!overlay.closing);
        assert!(matches!(
            &overlay.state,
            OverlayState::Popup(popup) if popup.key == testdata::pr_key("codeview", 7)
                && popup.active == 0
                && popup.panes.len() == PANES.len()
        ));
        assert_eq!(state.ui.ui_events, vec![]);
    }

    #[test]
    fn esc_and_q_quit_the_pinned_popup() {
        for code in [KeyCode::Esc, KeyCode::Char('q')] {
            let mut state = pinned(testdata::pr_key("codeview", 7));
            let cmds = update(
                &mut state,
                &Keymap::default(),
                press(code, KeyModifiers::NONE),
            );
            assert_eq!(cmds, vec![Cmd::Quit], "{code:?}");
            assert!(!state.ui.overlays[0].closing, "the pinned popup stays");
        }
    }

    /// A fake merge action with a confirm on `m`.
    fn confirm_merge() -> Vec<Box<dyn Action>> {
        let mut merge = FakeAction::new("merge", 'm');
        merge.prepare = Prepare::Confirm(crate::bus::ConfirmSpec {
            title: "merge?".into(),
            lines: Vec::new(),
            danger: true,
        });
        fake(merge)
    }

    #[test]
    fn esc_in_a_confirm_on_the_pinned_popup_closes_the_confirm_only() {
        let mut state = pinned(testdata::pr_key("chaoshub", 101));
        let actions = confirm_merge();
        act(&mut state, &actions, KeyCode::Char('m'));
        assert_eq!(state.ui.overlays.len(), 2);
        assert_eq!(state.ui.overlays[1].kind, OverlayKind::Confirm);
        assert_eq!(act(&mut state, &actions, KeyCode::Esc), vec![]);
        assert!(state.ui.overlays[1].closing, "the confirm closes");
        assert!(!state.ui.overlays[0].closing, "the pinned popup stays");
        assert_eq!(
            act(&mut state, &actions, KeyCode::Esc),
            vec![Cmd::Quit],
            "the pinned popup is on top again"
        );
    }

    /// The repos `chaoshub` and `codeview`, both with a path.
    fn two_repos() -> Vec<Repo> {
        ["chaoshub", "codeview"]
            .into_iter()
            .map(|name| Repo {
                key: testdata::repo_key(name),
                path: Some(format!("/code/{name}").into()),
                settings: None,
            })
            .collect()
    }

    fn startup() -> Msg {
        Msg::StartupDone {
            login: "chaosteil".into(),
            cwd_repo: None,
        }
    }

    #[test]
    fn the_pinned_startup_scans_the_pinned_repo_and_loads_the_pull_request() {
        let mut state = pinned_with(not_cached(), two_repos());
        let cmds = update(&mut state, &Keymap::default(), startup());
        assert_eq!(
            cmds,
            vec![
                Cmd::SetSession(state.data.session.clone()),
                Cmd::ScanJj(testdata::repo_key("chaoshub")),
                Cmd::LoadPr(not_cached()),
                Cmd::LoadAgent(not_cached()),
                Cmd::LoadCodeview(not_cached()),
            ]
        );
    }

    #[test]
    fn the_pinned_startup_refreshes_a_cached_pull_request() {
        let key = testdata::pr_key("chaoshub", 101);
        let mut state = pinned_with(key.clone(), two_repos());
        let cmds = update(&mut state, &Keymap::default(), startup());
        let pr = pr_ref(&state.data.prs[&key]);
        assert_eq!(
            cmds,
            vec![
                Cmd::SetSession(state.data.session.clone()),
                Cmd::ScanJj(testdata::repo_key("chaoshub")),
                Cmd::RefreshPr(vec![pr]),
                Cmd::LoadAgent(key.clone()),
                Cmd::LoadCodeview(key),
            ]
        );
    }

    #[test]
    fn the_pinned_interest_selects_the_loaded_pull_request() {
        let mut state = pinned(not_cached());
        let interest = state.ui.interest.clone().unwrap();
        assert_eq!(interest.tab, "pr");
        assert_eq!(interest.selected, None);
        let pr = testdata::pr("chaoshub", 500, "Loaded");
        let cmds = update(
            &mut state,
            &Keymap::default(),
            Msg::PrRefreshed(Box::new(pr.clone())),
        );
        assert_eq!(
            cmds,
            vec![Cmd::SetInterest(Interest {
                tab: "pr",
                scope: None,
                wants_local: false,
                selected: Some(pr_ref(&pr)),
                visible: Vec::new(),
            })]
        );
    }

    #[test]
    fn the_pinned_interest_shows_the_other_entries_of_the_stack() {
        let state = pinned(testdata::pr_key("chaoshub", 102));
        let interest = state.ui.interest.clone().unwrap();
        let numbers: Vec<u64> = interest.visible.iter().map(|pr| pr.key.number).collect();
        assert_eq!(numbers, [101, 103]);
        assert_eq!(
            interest.selected.map(|pr| pr.key.number),
            Some(102),
            "the pinned pull request is selected"
        );
        assert!(!interest.wants_local);
    }

    #[test]
    fn r_in_the_pinned_popup_loads_then_refreshes() {
        let mut state = pinned(not_cached());
        assert_eq!(key(&mut state, 'r'), vec![Cmd::LoadPr(not_cached())]);
        let pr = testdata::pr("chaoshub", 500, "Loaded");
        update(
            &mut state,
            &Keymap::default(),
            Msg::PrRefreshed(Box::new(pr.clone())),
        );
        assert_eq!(
            key(&mut state, 'r'),
            vec![
                Cmd::RefreshPr(vec![pr_ref(&pr)]),
                Cmd::LoadCodeview(pr.key.clone()),
                Cmd::ScanJj(testdata::repo_key("chaoshub")),
            ]
        );
    }

    #[test]
    fn r_in_the_sidebar_refreshes_the_stack_group() {
        let mut state = mine_state(10);
        key(&mut state, 'j');
        update(
            &mut state,
            &Keymap::default(),
            press(KeyCode::Enter, KeyModifiers::NONE),
        );
        let refs = stack_group_refs(&state.data, &testdata::pr_key("chaoshub", 103));
        assert_eq!(refs.len(), 3);
        assert_eq!(
            key(&mut state, 'r'),
            vec![
                Cmd::RefreshPr(refs),
                Cmd::LoadCodeview(testdata::pr_key("chaoshub", 103)),
                Cmd::ScanJj(testdata::repo_key("chaoshub")),
            ]
        );
    }

    #[test]
    fn help_opens_on_top_of_the_pinned_popup() {
        let mut state = pinned(testdata::pr_key("codeview", 7));
        assert_eq!(key(&mut state, '?'), vec![]);
        assert_eq!(state.ui.overlays.len(), 2);
        assert_eq!(state.ui.overlays[1].kind, OverlayKind::Help);
        assert!(!state.ui.overlays[0].closing);
    }

    #[test]
    fn a_click_outside_the_pinned_popup_does_nothing() {
        let mut state = pinned(testdata::pr_key("chaoshub", 101));
        let generation = state.generation;
        assert_eq!(
            update(&mut state, &Keymap::default(), click_on(Hit::Outside)),
            vec![]
        );
        assert!(!state.ui.overlays[0].closing);
        assert_eq!(state.generation, generation);
        let actions = confirm_merge();
        act(&mut state, &actions, KeyCode::Char('m'));
        assert_eq!(
            run_msg(&mut state, &actions, click_on(Hit::Outside)),
            vec![]
        );
        assert!(state.ui.overlays[1].closing, "the confirm closes");
        assert!(!state.ui.overlays[0].closing, "the pinned popup stays");
    }

    #[test]
    fn the_pinned_frame_has_no_tabs_and_no_list() {
        let mut state = pinned(testdata::pr_key("codeview", 7));
        let frame = frame::frame_model(&mut state).clone();
        assert_eq!(frame.tab_bar.titles, [""; 0]);
        assert_eq!(frame.list, empty_list());
        assert_eq!(frame.status.left[0].text, "[#7]");
        assert_eq!(
            frame.status.left[1].text,
            " chaosteil/codeview · updated 5m ago"
        );

        let mut state = pinned(not_cached());
        let frame = frame::frame_model(&mut state).clone();
        assert_eq!(
            frame.status.left[1].text,
            " chaosteil/chaoshub · not loaded yet"
        );
        assert!(matches!(
            &frame.overlays[0].content,
            OverlayContent::Popup { subtitle, .. } if subtitle == "not loaded yet"
        ));
    }

    /// The detail pane command of the tests.
    fn detail_pane() -> crate::app::state::DetailPane {
        crate::app::state::DetailPane {
            argv: vec!["/bin/chaoshub".into(), "--config".into(), "/c.toml".into()],
            cwd: "/home/me".into(),
            split: Some("left".into()),
            ratio: Some(40),
        }
    }

    /// The mine state with the detail pane, the repo `chaoshub` at
    /// `/code/chaoshub`, and the cursor on `#99`.
    fn detail_state() -> AppState {
        let mut state = AppState::new(SIZE);
        let repos = vec![Repo {
            key: testdata::repo_key("chaoshub"),
            path: Some("/code/chaoshub".into()),
            settings: None,
        }];
        state.load(testdata::mine_snapshot(), repos);
        state.data.session.login = Some("chaosteil".into());
        state.data.detail = Some(detail_pane());
        state
    }

    fn enter(state: &mut AppState) -> Vec<Cmd> {
        update(
            state,
            &Keymap::default(),
            press(KeyCode::Enter, KeyModifiers::NONE),
        )
    }

    /// The key and the request of a `RunTerminal` of the detail pane.
    fn detail_run(cmd: &Cmd) -> (TargetKey, LaunchRequest) {
        match cmd {
            Cmd::RunTerminal { id, key, request } if *id == DETAIL_ID => {
                (key.clone(), request.clone())
            }
            other => panic!("not a detail pane: {other:?}"),
        }
    }

    #[test]
    fn enter_with_a_detail_pane_runs_chaoshub_pr_in_a_pane() {
        let mut state = detail_state();
        assert_eq!(selected_number(&mut state), Some(99));
        let cmds = enter(&mut state);
        assert_eq!(cmds.len(), 1, "{cmds:?}");
        let (key, request) = detail_run(&cmds[0]);
        let pr = testdata::pr_key("chaoshub", 99);
        assert_eq!(key, TargetKey::Pr(pr.clone()));
        assert_eq!(
            request.argv,
            [
                "/bin/chaoshub",
                "--config",
                "/c.toml",
                "pr",
                "chaosteil/chaoshub",
                "99"
            ]
        );
        assert_eq!(request.cwd, std::path::PathBuf::from("/code/chaoshub"));
        assert_eq!(request.env, []);
        assert!(!request.zoom);
        assert_eq!(request.split.as_deref(), Some("left"));
        assert_eq!(request.ratio, Some(40));
        let suffix = request
            .label
            .strip_prefix("chaoshub-detail-chaosteil-chaoshub-99-")
            .unwrap();
        assert_eq!(suffix.len(), 4, "{}", request.label);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(state.ui.overlays.is_empty(), "no sidebar");
        assert_eq!(
            state.data.panes.get(&(DETAIL_ID, key.clone())),
            Some(&PaneHandleState::Starting)
        );
        update(
            &mut state,
            &Keymap::default(),
            Msg::PaneStarted { id: DETAIL_ID, key },
        );
        assert_eq!(flags_of(&mut state, 99), "▶");
    }

    #[test]
    fn a_repo_without_a_path_runs_in_the_directory_of_the_list() {
        let mut state = detail_state();
        key(&mut state, 'G');
        assert_eq!(selected_number(&mut state), Some(7));
        let (_, request) = detail_run(&enter(&mut state)[0]);
        assert_eq!(request.cwd, std::path::PathBuf::from("/home/me"));
        assert_eq!(request.argv[3..], ["pr", "chaosteil/codeview", "7"]);
    }

    #[test]
    fn enter_on_the_open_detail_pane_focuses_it() {
        let mut state = detail_state();
        enter(&mut state);
        assert_eq!(
            enter(&mut state),
            vec![Cmd::FocusPane {
                id: DETAIL_ID,
                key: TargetKey::Pr(testdata::pr_key("chaoshub", 99)),
            }]
        );
    }

    #[test]
    fn enter_on_another_pull_request_replaces_the_detail_pane() {
        let mut state = detail_state();
        enter(&mut state);
        key(&mut state, 'j');
        assert_eq!(selected_number(&mut state), Some(103));
        let cmds = enter(&mut state);
        assert_eq!(cmds.len(), 2, "{cmds:?}");
        assert_eq!(
            cmds[0],
            Cmd::CancelAction {
                id: DETAIL_ID,
                key: TargetKey::Pr(testdata::pr_key("chaoshub", 99)),
            }
        );
        let (key, _) = detail_run(&cmds[1]);
        assert_eq!(key, TargetKey::Pr(testdata::pr_key("chaoshub", 103)));
    }

    #[test]
    fn a_click_on_the_cursor_row_opens_the_detail_pane() {
        let mut state = detail_state();
        let cmds = update(&mut state, &Keymap::default(), click_on(Hit::Row(0)));
        assert_eq!(cmds.len(), 1, "{cmds:?}");
        let (key, _) = detail_run(&cmds[0]);
        assert_eq!(key, TargetKey::Pr(testdata::pr_key("chaoshub", 99)));
        assert!(state.ui.overlays.is_empty(), "no sidebar");
    }

    /// The mine state with a path for the repo of the cursor row, and the
    /// key of the cursor row.
    fn agent_state() -> (AppState, PrKey) {
        let mut state = mine_state(10);
        let model = frame::active_list(&mut state, &TABS);
        let crate::app::models::RowItem::Pr(pr) = &model.rows[model.cursor].item else {
            panic!("the cursor row is no pull request");
        };
        let key = pr.key.clone();
        state.data.repos.insert(
            key.repo.clone(),
            Repo {
                key: key.repo.clone(),
                path: Some("/code/chaoshub".into()),
                settings: None,
            },
        );
        (state, key)
    }

    /// The job of `tool` on `key` at the head of the pull request.
    fn agent_job(state: &AppState, key: &PrKey, tool: Tool) -> AgentJob {
        AgentJob {
            key: key.clone(),
            head_sha: state.data.prs[key].head.value.head_oid.clone(),
            repo_path: "/code/chaoshub".into(),
            tool,
        }
    }

    /// A finished review record with 12000 tokens.
    fn review_record() -> Record {
        Record {
            version: crate::domain::agent::VERSION,
            tool: "review".into(),
            pr: 7,
            head_sha: "oid7".into(),
            model: "m".into(),
            started_at: crate::domain::test_time(),
            finished_at: crate::domain::test_time(),
            usage: serde_json::json!({"total_tokens": 12000}),
            data: serde_json::json!({"review": {}}),
        }
    }

    fn status_text(state: &AppState) -> Option<&str> {
        state.ui.status.as_ref().map(|m| m.text.as_str())
    }

    #[test]
    fn shift_p_opens_the_popup_on_the_agent_pane_and_loads_the_results() {
        let (mut state, pr) = agent_state();
        assert_eq!(
            key(&mut state, 'P'),
            vec![Cmd::LoadAgent(pr.clone()), Cmd::LoadCodeview(pr.clone())]
        );
        let Some(Overlay {
            state: OverlayState::Popup(popup),
            ..
        }) = state.ui.overlays.last()
        else {
            panic!("no popup");
        };
        assert_eq!(popup.key, pr);
        assert_eq!(PANES[popup.active].id(), "agent");
        // In another pane, `P` moves the open popup to the Agent pane.
        key(&mut state, '1');
        assert_eq!(
            key(&mut state, 'P'),
            vec![Cmd::LoadAgent(pr.clone()), Cmd::LoadCodeview(pr)]
        );
        assert_eq!(state.ui.overlays.len(), 1);
    }

    #[test]
    fn a_tool_key_starts_one_run_per_tool_and_pull_request() {
        let (mut state, pr) = agent_state();
        key(&mut state, 'P');
        let review = agent_job(&state, &pr, Tool::Review);
        assert_eq!(key(&mut state, 'r'), vec![Cmd::RunAgent(review)]);
        assert!(
            state
                .data
                .agent_jobs
                .contains(&(pr.clone(), ToolKind::Review))
        );
        assert_eq!(key(&mut state, 'r'), Vec::new());
        assert_eq!(status_text(&state), Some("review runs already"));
        // Another tool on the same pull request runs at the same time.
        let improve = agent_job(&state, &pr, Tool::Improve);
        assert_eq!(key(&mut state, 'i'), vec![Cmd::RunAgent(improve)]);
        assert_eq!(state.data.agent_jobs.len(), 2);
    }

    #[test]
    fn the_end_of_a_run_stores_the_result_and_frees_the_tool() {
        let (mut state, pr) = agent_state();
        key(&mut state, 'P');
        key(&mut state, 'r');
        key(&mut state, 'i');
        let done = Msg::AgentFinished {
            key: pr.clone(),
            kind: ToolKind::Review,
            result: Ok(review_record()),
        };
        assert_eq!(
            update(&mut state, &Keymap::default(), done),
            vec![Cmd::LoadAgent(pr.clone())]
        );
        assert_eq!(status_text(&state), Some("review done (12k tokens)"));
        let failed = Msg::AgentFinished {
            key: pr.clone(),
            kind: ToolKind::Improve,
            result: Err("pr-agent failed (exit status: 3): no key".into()),
        };
        update(&mut state, &Keymap::default(), failed);
        assert_eq!(
            status_text(&state),
            Some("improve failed: pr-agent failed (exit status: 3): no key")
        );
        assert!(state.data.agent_jobs.is_empty());
        let results = &state.data.agent[&pr];
        assert_eq!(results.records[&ToolKind::Review], review_record());
        assert_eq!(
            results.errors[&ToolKind::Improve],
            "pr-agent failed (exit status: 3): no key"
        );
        // A load replaces the records and keeps the run errors.
        let loaded = Msg::AgentLoaded {
            key: pr.clone(),
            results: AgentResults::default(),
        };
        update(&mut state, &Keymap::default(), loaded);
        assert!(state.data.agent[&pr].records.is_empty());
        assert!(
            state.data.agent[&pr]
                .errors
                .contains_key(&ToolKind::Improve)
        );
        // The tool is free again.
        let review = agent_job(&state, &pr, Tool::Review);
        assert_eq!(key(&mut state, 'r'), vec![Cmd::RunAgent(review)]);
    }

    #[test]
    fn ask_opens_a_prompt_and_runs_with_the_question() {
        let (mut state, pr) = agent_state();
        key(&mut state, 'P');
        assert_eq!(key(&mut state, 'a'), Vec::new());
        assert!(matches!(
            state.ui.overlays.last().map(|o| &o.state),
            Some(OverlayState::Prompt { .. })
        ));
        for c in "why?".chars() {
            key(&mut state, c);
        }
        let enter = press(KeyCode::Enter, KeyModifiers::NONE);
        let ask = agent_job(&state, &pr, Tool::Ask("why?".into()));
        assert_eq!(
            update(&mut state, &Keymap::default(), enter),
            vec![Cmd::RunAgent(ask)]
        );
        assert!(state.data.agent_jobs.contains(&(pr, ToolKind::Ask)));
    }

    #[test]
    fn p_then_y_runs_the_agent_action_with_the_confirm() {
        let (mut state, pr) = agent_state();
        key(&mut state, 'P');
        let describe = Record {
            tool: "describe".into(),
            data: serde_json::json!({"title": "New title", "description": "- New"}),
            ..review_record()
        };
        state.data.agent.insert(
            pr.clone(),
            AgentResults {
                records: [(ToolKind::Describe, describe)].into(),
                errors: std::collections::BTreeMap::new(),
            },
        );
        assert_eq!(key(&mut state, 'p'), Vec::new());
        let Some(OverlayState::Confirm { action, spec }) =
            state.ui.overlays.last().map(|o| &o.state)
        else {
            panic!("no confirm");
        };
        assert_eq!(action.id, "agent");
        assert_eq!(spec.lines[2], "+ New title");
        let cmds = key(&mut state, 'y');
        let [Cmd::RunAction { id, input, .. }] = cmds.as_slice() else {
            panic!("{cmds:?}");
        };
        assert_eq!((*id, input), ("agent", &ActionInput::Confirmed));
    }

    #[test]
    fn agent_started_marks_the_job() {
        let (mut state, pr) = agent_state();
        let started = Msg::AgentStarted {
            key: pr.clone(),
            kind: ToolKind::Describe,
        };
        update(&mut state, &Keymap::default(), started);
        assert!(state.data.agent_jobs.contains(&(pr, ToolKind::Describe)));
    }
}
