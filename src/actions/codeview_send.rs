//! The codeview send action: sends chosen codeview comments as one review.
//!
//! The key of the action (`r`) opens the Review popup. It works directly
//! in the list and in the pull request detail, and in the actions menu.
//! In the conversation pane, `r` is the reply, so there only the actions
//! menu opens the popup. A key of an event in the popup starts the action
//! with
//! `ActionInput::Review`. The popup is the only way to approve, to request
//! changes and to send codeview comments. It opens also for a repo
//! without a local path. With comments, it
//! does not post to GitHub itself: it runs codeview in a headless nvim,
//! and codeview maps the comments to the lines of the diff, posts the
//! review, and marks the comments as sent. So the lock and the sent marks
//! of codeview cover each send.
//!
//! The action reads the comment files again first, and leaves out the
//! comments that went out from codeview in the meantime. When every chosen
//! comment went out already, the action posts the review with only the
//! message, so an approval does not stay behind. It writes the
//! request of `codeview.bridge` to a temporary file in the state
//! directory, runs
//! `nvim --headless -c "lua require('codeview.bridge').submit_file('<path>')"`
//! in the repo directory, and reads the result file. When codeview posts
//! no comment, it sends no review, and the action fails with
//! "review not sent".
//!
//! The request names the chosen comments per push of the head (version
//! 2). codeview posts the comments of an older push first, in a review of
//! their own on the head commit of that push. So each comment stays on
//! the lines that it was written for. A codeview that knows only version
//! 1 gets the comments of one push again with version 1. For comments of
//! more than one push, the action then fails and asks for a new codeview.
//!
//! The text of the result names each chosen comment that stays local,
//! with the reason. The status then shows the text as a warning.
//!
//! Without comments, codeview has nothing to post. Then the action posts
//! a review with only the message, with `addPullRequestReview`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::actions::review::{NVIM, agent_env};
use crate::actions::{Action, Applicability, Group, Prepare, Services, own_part};
use crate::bus::{
    ActionInput, Expect, KeyChord, KeyCode, KeyModifiers, Pending, PendingPart, ReviewKind,
    Selection, Target,
};
use crate::codeview::store;
use crate::domain::keys::PrKey;
use crate::domain::plural;
use crate::domain::pr::PullRequest;
use crate::github::ReviewEvent;

/// The longest run of the headless nvim. codeview gives the post of a
/// review 120 s, and the open of the pull request and the reads of GitHub
/// come on top. A kill during the post can leave a review on GitHub
/// without the sent marks of codeview, so the limit is well above that.
/// codeview takes a lock older than 10 minutes as stale, so the limit
/// stays at that time.
const TIMEOUT: Duration = Duration::from_secs(600);

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
        "send review"
    }

    fn hint(&self) -> String {
        self.label().to_owned()
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('r'),
            mods: KeyModifiers::NONE,
        })
    }

    fn group(&self) -> Group {
        Group::Review
    }

    /// Every pull request. Without a local path, the review has no
    /// codeview comments.
    fn applies(&self, _sel: &Selection) -> Applicability {
        Applicability::Enabled
    }

    /// A key of the action opens the Review popup, where you choose the
    /// comments and the event.
    fn prepare(&self, _sel: &Selection) -> Prepare {
        Prepare::Review
    }

    /// The reviews of the pull request, with the event of the review.
    fn pending(&self, sel: &Selection, input: &ActionInput) -> Vec<Pending> {
        let ActionInput::Review { event, .. } = input else {
            return Vec::new();
        };
        own_part(sel, PendingPart::Review, Expect::Review(*event))
    }

    /// Sends the comments of `ActionInput::Review` with a headless
    /// codeview. Without comments, it posts a review with only the
    /// message.
    fn execute(
        &self,
        sel: Selection,
        input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let Target::Pr { pr, repo, .. } = sel.target;
            let ActionInput::Review { ids, event, body } = input else {
                return Err("choose the comments in the Review popup".to_owned());
            };
            if ids.is_empty() {
                return message_review(&svc, &pr, event, &body).await;
            }
            let message = body.clone();
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
                key: pr.key.clone(),
                ids,
                event,
                body,
                timeout: TIMEOUT,
            };
            match send(job).await? {
                Sent::Done(text) => Ok(text),
                Sent::Gone(note) => message_review(&svc, &pr, event, &message)
                    .await
                    .map(|text| format!("{text}{note}")),
            }
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

/// The newest version of the request and the result of
/// `codeview.bridge`. Version 2 names the chosen comments per push.
const VERSION: u32 = 2;

/// The version of the request of a codeview without version 2. It takes
/// the comments of one push.
const VERSION_ONE: u32 = 1;

/// The request of `codeview.bridge`. Version 2 has `pushes`. Version 1
/// has `head` and `ids` of one push.
#[derive(Debug, Serialize)]
struct Request<'a> {
    version: u32,
    repo: String,
    number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ids: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pushes: Option<&'a [PushIds<'a>]>,
    event: &'a str,
    body: &'a str,
    out: &'a Path,
}

/// The chosen comments of one push in a request of version 2.
#[derive(Debug, Serialize)]
struct PushIds<'a> {
    /// The head commit of the push.
    head: &'a str,
    ids: Vec<String>,
}

/// The result of `codeview.bridge`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Outcome {
    ok: bool,
    posted: u64,
    /// The part of `posted` that went out on older pushes. Version 2 only.
    older: u64,
    skipped: Vec<Skipped>,
    already_sent: Vec<String>,
    /// True when the review with the event went out. Newer codeview only.
    reviewed: bool,
    error: String,
}

/// What a send did.
#[derive(Debug, PartialEq, Eq)]
enum Sent {
    /// codeview ran. The text of the result.
    Done(String),
    /// Every chosen comment went out already, so codeview did not run.
    /// The review with only the message goes out. The text names the
    /// comments that went out already.
    Gone(String),
}

/// A comment that stays local, with the reason.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Skipped {
    id: String,
    reason: String,
}

/// The error of the comments that went out from codeview already.
const SENT_ALREADY: &str = "review not sent: codeview already sent these comments";

/// The error of a send that posted no comment: codeview then sends no
/// review.
const NOT_OUT: &str = "review not sent";

/// The text before the comments that stay local, in the result of a
/// send. The status shows such a result as a warning.
pub(crate) const STAYS_LOCAL: &str = " · not sent: ";

/// The start of the error of a codeview that does not know the version of
/// the request.
const UNKNOWN_VERSION: &str = "the request has version";

/// The error of a codeview without version 2, for comments of more than
/// one push.
const OLD_CODEVIEW: &str = "review not sent: update codeview to send comments of older pushes";

/// Posts a review of `pr` with only the message `body`, with
/// `addPullRequestReview`. A comment review and a change request need a
/// message. An approval without a message has no body.
async fn message_review(
    svc: &Services,
    pr: &PullRequest,
    event: ReviewKind,
    body: &str,
) -> Result<String, String> {
    let (event, done) = verb(event);
    let empty = body.trim().is_empty();
    if empty && event != ReviewEvent::Approve {
        return Err("the review needs a message".to_owned());
    }
    svc.client
        .add_review(&pr.node_id, event, (!empty).then_some(body))
        .await
        .map_err(|err| err.to_string())?;
    Ok(format!("{done} #{}", pr.key.number))
}

/// The event of GitHub of `event`, and the verb of the result text.
fn verb(event: ReviewKind) -> (ReviewEvent, &'static str) {
    match event {
        ReviewKind::Comment => (ReviewEvent::Comment, "commented on"),
        ReviewKind::Approve => (ReviewEvent::Approve, "approved"),
        ReviewKind::RequestChanges => (ReviewEvent::RequestChanges, "requested changes on"),
    }
}

/// Reads the comment files again, runs the headless codeview on the
/// comments that are still unsent, and returns the text of the result.
/// A send that posted no comment and no review is an error. When every
/// chosen comment went out already, codeview does not run, and the result
/// is `Sent::Gone`. A comment review without a message then has nothing
/// to post, so it is an error.
///
/// The request names the chosen comments per push, so codeview posts the
/// comments of an older push on the head commit of that push. A codeview
/// that knows only version 1 gets the comments of one push again with
/// version 1.
async fn send(job: Job) -> Result<Sent, String> {
    let (dir, repo, number) = (job.comments_dir.clone(), job.repo.clone(), job.key.number);
    let comments = tokio::task::spawn_blocking(move || store::load(&dir, &repo, number))
        .await
        .map_err(|err| err.to_string())??;
    let mut places: BTreeMap<&str, String> = BTreeMap::new();
    let mut pushes: Vec<PushIds<'_>> = Vec::new();
    for id in &job.ids {
        let Some(push) = comments.push_of(id) else {
            continue;
        };
        let Some(comment) = push
            .comments
            .iter()
            .find(|comment| &comment.id == id && comment.sendable())
        else {
            continue;
        };
        places.insert(id, comment.place());
        match pushes.iter_mut().find(|chosen| chosen.head == push.head) {
            Some(chosen) => chosen.ids.push(id.clone()),
            None => pushes.push(PushIds {
                head: &push.head,
                ids: vec![id.clone()],
            }),
        }
    }
    if pushes.is_empty() {
        if job.event == ReviewKind::Comment && job.body.trim().is_empty() {
            return Err(SENT_ALREADY.to_owned());
        }
        return Ok(Sent::Gone(format!(
            " · already sent from codeview: {}",
            job.ids.join(", ")
        )));
    }
    let gone: Vec<&String> = job
        .ids
        .iter()
        .filter(|id| !places.contains_key(id.as_str()))
        .collect();
    let place = |id: &str| places.get(id).cloned().unwrap_or_else(|| id.to_owned());

    let mut outcome = request(&job, VERSION, &pushes).await?;
    if !outcome.ok && outcome.error.starts_with(UNKNOWN_VERSION) {
        if pushes.len() > 1 {
            return Err(OLD_CODEVIEW.to_owned());
        }
        outcome = request(&job, VERSION_ONE, &pushes).await?;
    }
    if !outcome.ok {
        return Err(outcome.error);
    }
    let mut text = String::new();
    if !outcome.skipped.is_empty() {
        let names: Vec<String> = outcome
            .skipped
            .iter()
            .map(|skipped| format!("{}: {}", place(&skipped.id), skipped.reason))
            .collect();
        let _ = write!(text, "{STAYS_LOCAL}{}", names.join(", "));
    }
    if !outcome.already_sent.is_empty() || !gone.is_empty() {
        let mut names: Vec<String> = outcome.already_sent.iter().map(|id| place(id)).collect();
        names.extend(gone.iter().map(|id| place(id)));
        let _ = write!(text, " · already sent from codeview: {}", names.join(", "));
    }
    if outcome.posted == 0 {
        if outcome.reviewed {
            let (_, done) = verb(job.event);
            return Ok(Sent::Done(format!("{done} #{}{text}", job.key.number)));
        }
        return Err(format!("{NOT_OUT}{text}"));
    }
    let older = if outcome.older > 0 {
        format!(" ({} on an older push)", outcome.older)
    } else {
        String::new()
    };
    let name = job.event.name();
    let article = if name.starts_with('a') { "an" } else { "a" };
    Ok(Sent::Done(format!(
        "posted {} on #{} as {article} {name} review{older}{text}",
        noun(outcome.posted),
        job.key.number
    )))
}

/// Writes the request of `version` for `pushes` to a temporary file in
/// the work directory, runs the headless codeview on it, and reads the
/// result. Version 1 takes the first push only. The request and the
/// result files go away at the end.
async fn request(job: &Job, version: u32, pushes: &[PushIds<'_>]) -> Result<Outcome, String> {
    std::fs::create_dir_all(&job.work_dir)
        .map_err(|err| format!("cannot make {}: {err}", job.work_dir.display()))?;
    let stem = format!("{}-{:08x}", std::process::id(), rand::random::<u32>());
    let request_path = job.work_dir.join(format!("send-{stem}.json"));
    let result_path = job.work_dir.join(format!("result-{stem}.json"));
    let one = (version == VERSION_ONE).then(|| &pushes[0]);
    let request = Request {
        version,
        repo: format!("{}/{}", job.key.repo.owner, job.key.repo.name),
        number: job.key.number,
        head: one.map(|push| push.head),
        ids: one.map(|push| push.ids.as_slice()),
        pushes: (version != VERSION_ONE).then_some(pushes),
        event: job.event.name(),
        body: &job.body,
        out: &result_path,
    };
    let text = serde_json::to_string(&request).map_err(|err| err.to_string())?;
    std::fs::write(&request_path, text)
        .map_err(|err| format!("cannot write {}: {err}", request_path.display()))?;
    let outcome = run(job, &request_path, &result_path).await;
    let _ = std::fs::remove_file(&request_path);
    let _ = std::fs::remove_file(&result_path);
    outcome
}

/// `1 comment` or `N comments`.
fn noun(count: u64) -> String {
    plural::count(count, "comment", "comments")
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

    /// A comment file of #14 at the older head `h0` for the repo at
    /// `repo`: `c3` is unsent.
    fn older_file(repo: &Path) -> String {
        format!(
            "---\nversion: 1\nrepo: {}\nrange: pr-14-h0\n---\n\n\
             ## codeview-comment c3\n\n- file: src/c.rs\n- start_line: 7\n- end_line: 8\n\
             - side: old\n- synced_at:\n\nOld note\n",
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

        /// Adds the comment file of the older push `h0`.
        fn with_older(self) -> Self {
            let dir = self.root.path();
            let file = dir.join("comments").join("repo-1a2b3c4d").join("old.md");
            std::fs::write(file, older_file(&dir.join("repo"))).unwrap();
            self
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
            Sent::Done(
                "posted 1 comment on #14 as a comment review · already sent from codeview: c2"
                    .into()
            )
        );
        let request = setup.request();
        assert_eq!(request["version"], 2);
        assert_eq!(request["repo"], "chaosteil/chaoshub-test");
        assert_eq!(request["number"], 14);
        assert_eq!(
            request["pushes"],
            serde_json::json!([{ "head": "h1", "ids": ["c1"] }])
        );
        assert_eq!(request.get("head"), None, "version 1 fields");
        assert_eq!(request.get("ids"), None);
        assert_eq!(request["event"], "comment");
        assert_eq!(
            request["body"], "line one\nline two",
            "the line breaks stay"
        );
        assert_eq!(setup.leftovers(), 0, "the request and the result go away");
    }

    #[tokio::test]
    async fn the_request_names_the_comments_of_each_push() {
        let body = r#"printf '%s' '{"version":2,"ok":true,"posted":2,"older":1,"skipped":[],"already_sent":[],"url":"u","error":""}' > "$out""#;
        let setup = Setup::new(body).with_older();
        let mut job = setup.job(&["c3", "c1", "c2"], TIMEOUT);
        job.event = ReviewKind::Approve;
        let text = send(job).await.unwrap();
        assert_eq!(
            text,
            Sent::Done("posted 2 comments on #14 as an approve review (1 on an older push) · already sent from codeview: c2".into())
        );
        assert_eq!(
            setup.request()["pushes"],
            serde_json::json!([
                { "head": "h0", "ids": ["c3"] },
                { "head": "h1", "ids": ["c1"] },
            ]),
            "one entry per push, in the order of the ids"
        );
        assert_eq!(setup.leftovers(), 0);
    }

    #[tokio::test]
    async fn a_comment_of_an_older_push_that_stays_local_is_in_the_text() {
        let body = r#"printf '%s' '{"version":2,"ok":true,"posted":1,"older":0,"skipped":[{"id":"c3","reason":"cannot read the diff of the older push h0"}],"already_sent":[],"url":"u","error":""}' > "$out""#;
        let setup = Setup::new(body).with_older();
        let text = send(setup.job(&["c1", "c3"], TIMEOUT)).await.unwrap();
        assert_eq!(
            text,
            Sent::Done(format!(
                "posted 1 comment on #14 as a comment review{STAYS_LOCAL}src/c.rs:L7-8 (old): cannot read the diff of the older push h0"
            ))
        );
    }

    /// The body of a fake nvim of a codeview that knows only version 1.
    const OLD_CODEVIEW_BODY: &str = r#"if grep -q '"version":2' "$req"; then printf '%s' '{"version":1,"ok":false,"posted":0,"skipped":[],"already_sent":[],"url":"","error":"the request has version 2, not 1"}' > "$out"; exit 1; fi
printf '%s' '{"version":1,"ok":true,"posted":1,"skipped":[],"already_sent":[],"url":"u","error":""}' > "$out""#;

    #[tokio::test]
    async fn an_older_codeview_gets_the_comments_of_one_push_with_version_1() {
        let setup = Setup::new(OLD_CODEVIEW_BODY);
        let text = send(setup.job(&["c1"], TIMEOUT)).await.unwrap();
        assert_eq!(
            text,
            Sent::Done("posted 1 comment on #14 as a comment review".into())
        );
        let request = setup.request();
        assert_eq!(request["version"], 1, "the second request");
        assert_eq!(request["head"], "h1");
        assert_eq!(request["ids"], serde_json::json!(["c1"]));
        assert_eq!(request.get("pushes"), None);
        assert_eq!(setup.leftovers(), 0);
    }

    #[tokio::test]
    async fn an_older_codeview_cannot_send_the_comments_of_two_pushes() {
        let setup = Setup::new(OLD_CODEVIEW_BODY).with_older();
        let err = send(setup.job(&["c1", "c3"], TIMEOUT)).await.unwrap_err();
        assert_eq!(err, OLD_CODEVIEW);
        assert_eq!(setup.request()["version"], 2, "no second request");
    }

    #[tokio::test]
    async fn the_result_names_the_skipped_comments() {
        let body = r#"printf '%s' '{"version":1,"ok":true,"posted":0,"skipped":[{"id":"c1","reason":"the diff does not hold new line 2"}],"already_sent":[],"url":"","error":""}' > "$out""#;
        let setup = Setup::new(body);
        let err = send(setup.job(&["c1"], TIMEOUT)).await.unwrap_err();
        assert_eq!(
            err,
            "review not sent · not sent: src/a.rs:L2 (new): the diff does not hold new line 2"
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
    async fn comments_that_went_out_already_leave_only_the_message() {
        let setup = Setup::new(OK);
        let sent = send(setup.job(&["c2"], TIMEOUT)).await.unwrap();
        assert_eq!(sent, Sent::Gone(" · already sent from codeview: c2".into()));
        assert!(
            !setup.root.path().join("request.json").exists(),
            "nvim did not run"
        );
    }

    #[tokio::test]
    async fn comments_that_went_out_already_post_no_comment_review_without_a_message() {
        let setup = Setup::new(OK);
        let mut job = setup.job(&["c2"], TIMEOUT);
        job.body = " \n".into();
        let err = send(job).await.unwrap_err();
        assert_eq!(err, SENT_ALREADY);
        assert!(!setup.root.path().join("request.json").exists());
    }

    #[tokio::test]
    async fn a_review_without_new_comments_is_a_result() {
        let body = r#"printf '%s' '{"version":2,"ok":true,"posted":0,"older":0,"skipped":[],"already_sent":["c1"],"reviewed":true,"url":"u","error":""}' > "$out""#;
        let setup = Setup::new(body);
        let mut job = setup.job(&["c1"], TIMEOUT);
        job.event = ReviewKind::Approve;
        let sent = send(job).await.unwrap();
        assert_eq!(
            sent,
            Sent::Done("approved #14 · already sent from codeview: src/a.rs:L2 (new)".into())
        );
    }

    #[tokio::test]
    async fn an_approval_of_comments_that_went_out_already_posts_the_approval_alone() {
        let setup = Setup::new(OK);
        let dir = setup.root.path();
        let mut selection = selection();
        let Target::Pr { repo, .. } = &mut selection.target;
        repo.path = Some(dir.join("repo"));
        let mut config = toml::Table::new();
        config.insert(
            CodeviewSend::COMMENTS_DIR.into(),
            toml::Value::String(dir.join("comments").to_string_lossy().into_owned()),
        );
        selection.action_config = Some(config);
        let input = ActionInput::Review {
            ids: vec!["c2".into()],
            event: ReviewKind::Approve,
            body: String::new(),
        };
        let server = server().await;
        let result = CodeviewSend
            .execute(selection, input, services(&server))
            .await;
        assert_eq!(
            result,
            Ok("approved #7 · already sent from codeview: c2".to_owned())
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "only the approval");
        let request: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(request["variables"]["event"], "APPROVE");
    }

    /// A selection of the pull request of `testdata/pr.json`, by
    /// `reviewer`.
    fn selection() -> Selection {
        let pr: PullRequest = serde_json::from_str(include_str!("testdata/pr.json")).unwrap();
        Selection {
            target: Target::Pr {
                repo: crate::domain::repo::Repo {
                    key: pr.key.repo.clone(),
                    path: None,
                    settings: None,
                },
                pr,
                stack: Vec::new(),
                local: crate::domain::local::LocalMatch::None,
            },
            me: "reviewer".into(),
            action_config: None,
        }
    }

    /// A GraphQL server that answers every request with one review.
    async fn server() -> wiremock::MockServer {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/graphql"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-remaining", "4990")
                    .insert_header("x-ratelimit-reset", "1790000000")
                    .set_body_json(serde_json::json!({ "data": {
                        "addPullRequestReview": { "pullRequestReview": { "id": "PRR_1", "state": "COMMENTED" } }
                    } })),
            )
            .mount(&server)
            .await;
        server
    }

    fn services(server: &wiremock::MockServer) -> Services {
        let client = crate::github::GithubClient::new(
            crate::github::TokenSource::command("echo", &["tok"]),
            &server.uri(),
        );
        Services {
            client: std::sync::Arc::new(client.unwrap()),
            locks: crate::vcs::lock::RepoLocks::default(),
            cancel: tokio_util::sync::CancellationToken::new(),
            tx: tokio::sync::mpsc::channel(8).0,
        }
    }

    /// The input of a review without comments.
    fn message(event: ReviewKind, body: &str) -> ActionInput {
        ActionInput::Review {
            ids: Vec::new(),
            event,
            body: body.into(),
        }
    }

    #[tokio::test]
    async fn without_comments_the_review_has_only_the_message() {
        let cases = [
            (ReviewKind::Approve, "", "approved #7", "APPROVE", None),
            (
                ReviewKind::Comment,
                "Thanks!",
                "commented on #7",
                "COMMENT",
                Some("Thanks!"),
            ),
            (
                ReviewKind::RequestChanges,
                "Split it.",
                "requested changes on #7",
                "REQUEST_CHANGES",
                Some("Split it."),
            ),
        ];
        for (event, body, text, name, sent) in cases {
            let server = server().await;
            let result = CodeviewSend
                .execute(selection(), message(event, body), services(&server))
                .await;
            assert_eq!(result, Ok(text.to_owned()));
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            let request: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert_eq!(request["operationName"], "AddReview");
            assert_eq!(
                request["variables"],
                serde_json::json!({ "id": "PR_kwDO7", "event": name, "body": sent })
            );
        }
    }

    #[tokio::test]
    async fn without_comments_a_comment_or_a_change_request_needs_a_message() {
        let server = server().await;
        for event in [ReviewKind::Comment, ReviewKind::RequestChanges] {
            let result = CodeviewSend
                .execute(selection(), message(event, " \n"), services(&server))
                .await;
            assert_eq!(result, Err("the review needs a message".to_owned()));
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 0);
    }

    #[test]
    fn lua_quote_escapes_quotes_and_backslashes() {
        assert_eq!(lua_quote(r"/tmp/it's\x"), r"/tmp/it\'s\\x");
    }
}
