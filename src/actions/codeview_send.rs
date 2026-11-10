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
use crate::actions::{Action, Applicability, Prepare, Services};
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::domain::keys::RepoKey;

    /// A comment file of #14 at the head `h1` for the repo at `repo`:
    /// `c1` is unsent, `c2` went out already.
    fn comment_file(repo: &Path) -> String {
        format!(
            "---\nversion: 1\nrepo: {}\nrange: pr-14-h1\n---\n\n\
             ## codeview-comment c1\n\n- file: src/a.rs\n- start_line: 2\n- end_line: 2\n\
             - side: new\n- synced_at:\n- reply_to: 123\n\nFirst note\n\n\
             ## codeview-comment c2\n\n- file: src/b.rs\n- start_line: 5\n- end_line: 5\n\
             - side: new\n- synced_at: 2026-01-01T00:00:00Z\n\nSecond note\n",
            repo.display()
        )
    }

    /// A temporary setup: a repo, a comment directory, a work directory
    /// and a fake nvim that runs `body` after it copies the request to
    /// `request.json`. `$out` is the result path of the request.
    struct Setup {
        root: tempfile::TempDir,
    }

    impl Setup {
        fn new(body: &str) -> Self {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path();
            std::fs::create_dir_all(dir.join("repo")).unwrap();
            let comments = dir.join("comments").join("repo-1a2b3c4d");
            std::fs::create_dir_all(&comments).unwrap();
            std::fs::write(comments.join("k.md"), comment_file(&dir.join("repo"))).unwrap();
            let script = format!(
                "#!/bin/sh\n\
                 req=$(printf '%s' \"$3\" | sed \"s/.*submit_file('\\(.*\\)').*/\\1/\")\n\
                 cp \"$req\" '{copy}'\n\
                 out=$(sed -n 's/.*\"out\":\"\\([^\"]*\\)\".*/\\1/p' \"$req\")\n\
                 {body}\n",
                copy = dir.join("request.json").display(),
            );
            let nvim = dir.join("nvim");
            std::fs::write(&nvim, script).unwrap();
            std::fs::set_permissions(&nvim, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self { root }
        }

        fn job(&self, ids: &[&str], timeout: Duration) -> Job {
            let dir = self.root.path();
            Job {
                program: dir.join("nvim"),
                env: Vec::new(),
                repo: dir.join("repo"),
                comments_dir: dir.join("comments"),
                work_dir: dir.join("work"),
                key: PrKey {
                    repo: RepoKey {
                        owner: "chaosteil".into(),
                        name: "chaoshub-test".into(),
                    },
                    number: 14,
                },
                ids: ids.iter().map(|id| (*id).to_owned()).collect(),
                event: ReviewKind::Comment,
                body: "line one\nline two".into(),
                timeout,
            }
        }

        /// The request that the fake nvim got.
        fn request(&self) -> serde_json::Value {
            let text = std::fs::read_to_string(self.root.path().join("request.json")).unwrap();
            serde_json::from_str(&text).unwrap()
        }

        /// The files that stay in the work directory.
        fn leftovers(&self) -> usize {
            std::fs::read_dir(self.root.path().join("work")).map_or(0, Iterator::count)
        }
    }

    const OK: &str = r#"printf '%s' '{"version":1,"ok":true,"posted":1,"skipped":[],"already_sent":[],"url":"u","error":""}' > "$out""#;

    #[tokio::test]
    async fn an_ok_result_names_the_posted_and_the_sent_comments() {
        let setup = Setup::new(OK);
        let text = send(setup.job(&["c1", "c2"], TIMEOUT)).await.unwrap();
        assert_eq!(
            text,
            "posted 1 comment as a comment review · went out from codeview already: c2"
        );
        let request = setup.request();
        assert_eq!(request["version"], 1);
        assert_eq!(request["repo"], "chaosteil/chaoshub-test");
        assert_eq!(request["number"], 14);
        assert_eq!(request["head"], "h1");
        assert_eq!(request["ids"], serde_json::json!(["c1"]));
        assert_eq!(request["event"], "comment");
        assert_eq!(
            request["body"], "line one\nline two",
            "the line breaks stay"
        );
        assert_eq!(setup.leftovers(), 0, "the request and the result go away");
    }

    #[tokio::test]
    async fn the_result_names_the_skipped_comments() {
        let body = r#"printf '%s' '{"version":1,"ok":true,"posted":0,"skipped":[{"id":"c1","reason":"the diff does not hold new line 2"}],"already_sent":[],"url":"","error":""}' > "$out""#;
        let setup = Setup::new(body);
        let text = send(setup.job(&["c1"], TIMEOUT)).await.unwrap();
        assert_eq!(
            text,
            "posted 0 comments as a comment review · stays local: src/a.rs:L2 (new): the diff does not hold new line 2"
        );
    }

    #[tokio::test]
    async fn a_failed_result_is_the_error() {
        let body = r#"printf '%s' '{"version":1,"ok":false,"posted":0,"skipped":[],"already_sent":[],"url":"","error":"HTTP 422: Validation Failed"}' > "$out"; exit 1"#;
        let setup = Setup::new(body);
        let err = send(setup.job(&["c1"], TIMEOUT)).await.unwrap_err();
        assert_eq!(err, "HTTP 422: Validation Failed");
        assert_eq!(setup.leftovers(), 0);
    }

    #[tokio::test]
    async fn a_run_without_a_result_reports_the_exit_and_stderr() {
        let setup = Setup::new("echo 'E5108: no codeview' >&2; exit 1");
        let err = send(setup.job(&["c1"], TIMEOUT)).await.unwrap_err();
        assert!(err.starts_with("nvim ended with exit status: 1"), "{err}");
        assert!(err.ends_with("E5108: no codeview"), "{err}");
    }

    #[tokio::test]
    async fn a_run_past_the_timeout_is_killed() {
        let setup = Setup::new("sleep 10");
        let started = std::time::Instant::now();
        let err = send(setup.job(&["c1"], Duration::from_millis(300)))
            .await
            .unwrap_err();
        assert!(err.starts_with("nvim did not end in"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(setup.leftovers(), 0);
    }

    #[tokio::test]
    async fn comments_that_went_out_already_post_nothing() {
        let setup = Setup::new(OK);
        let text = send(setup.job(&["c2"], TIMEOUT)).await.unwrap();
        assert_eq!(text, SENT_ALREADY);
        assert!(
            !setup.root.path().join("request.json").exists(),
            "nvim did not run"
        );
    }

    #[test]
    fn lua_quote_escapes_quotes_and_backslashes() {
        assert_eq!(lua_quote(r"/tmp/it's\x"), r"/tmp/it\'s\\x");
    }
}
