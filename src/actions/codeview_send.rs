//! The codeview send action: sends chosen codeview comments as one review.
//!
//! The send dialog of the Description pane starts the action with
//! `ActionInput::Review`. The action has no default key. It never posts
//! to GitHub itself: it runs codeview in a headless nvim, and codeview
//! maps the comments to the lines of the diff, posts the review, and marks
//! the comments as sent. So the lock and the sent marks of codeview cover
//! each send.
//!
//! The action reads the comment files again first, and leaves out the
//! comments that went out from codeview in the meantime. It writes the
//! request of `codeview.bridge` to a temporary file in the state
//! directory, runs
//! `nvim --headless -c "lua require('codeview.bridge').submit_file('<path>')"`
//! in the repo directory, and reads the result file.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::actions::review::{NVIM, agent_env};
use crate::actions::{Action, Applicability, Hint, Prepare, Services};
use crate::bus::{ActionInput, KeyChord, ReviewKind, Selection, Target};
use crate::codeview::store;
use crate::domain::keys::PrKey;

/// The longest run of the headless nvim.
const TIMEOUT: Duration = Duration::from_secs(120);

/// The version of the request and the result of `codeview.bridge`.
const VERSION: u32 = 1;

/// The action that sends codeview comments through a headless codeview.
pub(crate) struct CodeviewSend;

impl CodeviewSend {
    /// The `[actions.codeview_send]` key of the comment directory. `run`
    /// fills it from `[codeview] comments_dir`.
    pub(crate) const COMMENTS_DIR: &'static str = "comments_dir";
}

impl Action for CodeviewSend {
    fn id(&self) -> &'static str {
        "codeview_send"
    }

    fn label(&self) -> &'static str {
        "send codeview comments"
    }

    /// The `S` key of the Description pane starts the send.
    fn default_key(&self) -> Option<KeyChord> {
        None
    }

    fn hint(&self) -> Hint {
        Hint::None
    }

    /// A pull request of a repo with a local path: codeview runs in it.
    fn applies(&self, sel: &Selection) -> Applicability {
        match &sel.target {
            Target::Pr { repo, .. } if repo.path.is_some() => Applicability::Enabled,
            Target::Pr { .. } => Applicability::Disabled("the repo has no local path"),
        }
    }

    /// A key of the action opens the Description pane, where you choose
    /// the comments.
    fn prepare(&self, _sel: &Selection) -> Prepare {
        Prepare::OpenPopup("description")
    }

    /// Sends the comments of `ActionInput::Review` with a headless
    /// codeview.
    fn execute(
        &self,
        sel: Selection,
        input: ActionInput,
        _svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let Target::Pr { pr, repo, .. } = sel.target;
            let ActionInput::Review { ids, event, body } = input else {
                return Err("choose the comments in the Description pane".to_owned());
            };
            let repo = repo
                .path
                .ok_or_else(|| "the repo has no local path".to_owned())?;
            let comments_dir = sel
                .action_config
                .as_ref()
                .and_then(|table| table.get(Self::COMMENTS_DIR))
                .and_then(toml::Value::as_str)
                .map(PathBuf::from)
                .or_else(|| store::comments_dir(None))
                .ok_or_else(|| "no directory of the codeview comments".to_owned())?;
            let work_dir = crate::config::paths::state_dir()
                .ok_or_else(|| "no state directory".to_owned())?
                .join("codeview");
            let job = Job {
                program: PathBuf::from(NVIM),
                env: agent_env(),
                repo,
                comments_dir,
                work_dir,
                key: pr.key,
                ids,
                event,
                body,
                timeout: TIMEOUT,
            };
            send(job).await
        })
    }
}

/// One send.
#[derive(Debug, Clone)]
struct Job {
    /// The nvim binary.
    program: PathBuf,
    env: Vec<(String, String)>,
    /// The local checkout of the repo. nvim runs in it.
    repo: PathBuf,
    comments_dir: PathBuf,
    /// The directory of the request and the result files.
    work_dir: PathBuf,
    key: PrKey,
    ids: Vec<String>,
    event: ReviewKind,
    body: String,
    timeout: Duration,
}

/// The request of `codeview.bridge`.
#[derive(Debug, Serialize)]
struct Request<'a> {
    version: u32,
    repo: String,
    number: u64,
    head: &'a str,
    ids: &'a [String],
    event: &'a str,
    body: &'a str,
    out: &'a Path,
}

/// The result of `codeview.bridge`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Outcome {
    ok: bool,
    posted: u64,
    skipped: Vec<Skipped>,
    already_sent: Vec<String>,
    error: String,
}

/// A comment that stays local, with the reason.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Skipped {
    id: String,
    reason: String,
}

/// The text of the comments that went out from codeview already.
const SENT_ALREADY: &str = "the comments went out from codeview already";

/// Reads the comment files again, runs the headless codeview on the
/// comments that are still unsent, and returns the text of the result.
async fn send(job: Job) -> Result<String, String> {
    let (dir, repo, number) = (job.comments_dir.clone(), job.repo.clone(), job.key.number);
    let comments = tokio::task::spawn_blocking(move || store::load(&dir, &repo, number))
        .await
        .map_err(|err| err.to_string())??;
    let Some(push) = job.ids.iter().find_map(|id| comments.push_of(id)) else {
        return Ok(SENT_ALREADY.to_owned());
    };
    let places: BTreeMap<&str, String> = push
        .comments
        .iter()
        .map(|comment| (comment.id.as_str(), comment.place()))
        .collect();
    let ids: Vec<String> = job
        .ids
        .iter()
        .filter(|id| {
            push.comments
                .iter()
                .any(|comment| &comment.id == *id && comment.sendable())
        })
        .cloned()
        .collect();
    let gone = job.ids.len() - ids.len();
    let place = |id: &str| places.get(id).cloned().unwrap_or_else(|| id.to_owned());

    std::fs::create_dir_all(&job.work_dir)
        .map_err(|err| format!("cannot make {}: {err}", job.work_dir.display()))?;
    let stem = format!("{}-{:08x}", std::process::id(), rand::random::<u32>());
    let request_path = job.work_dir.join(format!("send-{stem}.json"));
    let result_path = job.work_dir.join(format!("result-{stem}.json"));
    let request = Request {
        version: VERSION,
        repo: format!("{}/{}", job.key.repo.owner, job.key.repo.name),
        number: job.key.number,
        head: &push.head,
        ids: &ids,
        event: job.event.name(),
        body: &job.body,
        out: &result_path,
    };
    let text = serde_json::to_string(&request).map_err(|err| err.to_string())?;
    std::fs::write(&request_path, text)
        .map_err(|err| format!("cannot write {}: {err}", request_path.display()))?;
    let outcome = run(&job, &request_path, &result_path).await;
    let _ = std::fs::remove_file(&request_path);
    let _ = std::fs::remove_file(&result_path);
    let outcome = outcome?;
    if !outcome.ok {
        return Err(outcome.error);
    }
    let mut text = format!(
        "posted {} as a {} review",
        noun(outcome.posted),
        job.event.name()
    );
    if !outcome.skipped.is_empty() {
        let names: Vec<String> = outcome
            .skipped
            .iter()
            .map(|skipped| format!("{}: {}", place(&skipped.id), skipped.reason))
            .collect();
        let _ = write!(text, " · stays local: {}", names.join(", "));
    }
    let sent = outcome.already_sent.len() + gone;
    if sent > 0 {
        let mut names: Vec<String> = outcome.already_sent.iter().map(|id| place(id)).collect();
        names.extend(
            job.ids
                .iter()
                .filter(|id| !ids.contains(id))
                .map(|id| place(id)),
        );
        let _ = write!(
            text,
            " · went out from codeview already: {}",
            names.join(", ")
        );
    }
    Ok(text)
}

/// `1 comment` or `N comments`.
fn noun(count: u64) -> String {
    if count == 1 {
        "1 comment".to_owned()
    } else {
        format!("{count} comments")
    }
}

/// Runs the headless codeview on the request file and reads the result
/// file. A run past the timeout is killed.
async fn run(job: &Job, request: &Path, result: &Path) -> Result<Outcome, String> {
    let script = format!(
        "lua require('codeview.bridge').submit_file('{}')",
        lua_quote(&request.to_string_lossy())
    );
    let child = tokio::process::Command::new(&job.program)
        .args(["--headless", "-c", &script])
        .current_dir(&job.repo)
        .envs(job.env.iter().map(|(name, value)| (name, value)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| format!("cannot run {}: {err}", job.program.display()))?;
    let output = tokio::time::timeout(job.timeout, child.wait_with_output())
        .await
        .map_err(|_| format!("nvim did not end in {} s", job.timeout.as_secs()))?
        .map_err(|err| err.to_string())?;
    if let Ok(text) = std::fs::read_to_string(result) {
        return serde_json::from_str(&text)
            .map_err(|err| format!("cannot read the result of codeview: {err}"));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stderr.lines().find(|line| !line.trim().is_empty());
    Err(match line {
        Some(line) => format!("nvim ended with {}: {}", output.status, line.trim()),
        None => format!("nvim ended with {} and wrote no result", output.status),
    })
}

/// `text` as the content of a Lua string in single quotes.
fn lua_quote(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'")
}
