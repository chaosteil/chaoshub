//! One run of pr-agent.
//!
//! The run starts the `[agent] command` in the repo directory:
//!
//! ```text
//! <command> --pr_url https://github.com/<owner>/<repo>/pull/<n> --json-output <tmp> <tool>
//! ```
//!
//! pr-agent reads its settings from env variables with the
//! `SECTION__KEY` form. The run sets the `[agent] env` table, then
//! `GITHUB__USER_TOKEN` (from `gh auth token`), `CONFIG__MODEL`,
//! `CONFIG__FALLBACK_MODELS`, `CONFIG__PUBLISH_OUTPUT=false`,
//! `CONFIG__PROPAGATE_TOOL_ERRORS=true` and the extra instructions of the
//! tool. The values are TOML text, because pr-agent reads env values as
//! TOML. pr-agent refuses `config.fallback_models` as an argument, so the
//! env carries all settings.
//!
//! `ask` has no `--json-output`. The run starts the script
//! `chaoshub-agent-ask` with the Python of the command instead. The
//! script prints the answer to stdout. See `ask_argv` for how the run
//! finds that Python.
//!
//! Each line on stdout and stderr goes to `app` as
//! `ActionEvent::Progress`. At `[agent] timeout_secs`, the run kills the
//! process. A failed run writes no result file, so the old file stays.

use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use chrono::Utc;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::agent::ID;
use crate::agent::store::Store;
use crate::bus::{ActionEvent, Msg, TargetKey};
use crate::config::schema::AgentConfig;
use crate::domain::agent::{AskEntry, Record, Tool, ToolKind, VERSION};
use crate::domain::keys::PrKey;
use crate::github::TokenSource;

/// The text of the ask script.
pub(crate) const SHIM_TEXT: &str = include_str!("chaoshub-agent-ask.py");

/// One run: a tool on a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Job {
    pub key: PrKey,
    /// The head commit of the pull request when the run starts.
    pub head_sha: String,
    /// The local checkout of the repo. pr-agent runs in it.
    pub repo_path: PathBuf,
    pub tool: Tool,
}

/// A line reader: the last line and all the text of one stream.
type Reader = JoinHandle<(Option<String>, String)>;

/// What a finished process gave.
struct Finished {
    status: ExitStatus,
    /// The last line on stdout or stderr.
    last_line: Option<String>,
    /// All of stdout, for `ask` only.
    stdout: String,
}

/// Runs pr-agent for `job` and writes the result to `store`. It sends
/// the progress lines to `tx`. On an error, it writes nothing and returns
/// the text for the status line.
pub(crate) async fn run(
    job: &Job,
    config: &AgentConfig,
    store: &Store,
    token: &TokenSource,
    tx: &Sender<Msg>,
) -> Result<Record, String> {
    let started_at = Utc::now();
    let kind = job.tool.kind();
    let user_env = config
        .resolve_env(|name| std::env::var(name).ok())
        .await
        .map_err(|err| err.to_string())?;
    let token = token
        .token()
        .await
        .map_err(|err| format!("gh auth token: {err}"))?;
    let env = process_env(config, kind, user_env, &token);
    let output = store
        .output_path(&job.key, kind)
        .map_err(|err| format!("cannot write the result: {err}"))?;
    let result = match argv(config, store, job, &output) {
        Ok(argv) => spawn(&argv, &env, job, config.timeout_secs, tx).await,
        Err(err) => Err(err),
    };
    let data = result.and_then(|finished| read_output(job, &output, finished));
    let _ = std::fs::remove_file(&output);
    let (data, usage) = data?;
    let data = match &job.tool {
        Tool::Ask(question) => ask_history(store, job, question, &data),
        _ => data,
    };
    let record = Record {
        version: VERSION,
        tool: kind.name().to_owned(),
        pr: job.key.number,
        head_sha: job.head_sha.clone(),
        model: config.model.clone(),
        started_at,
        finished_at: Utc::now(),
        usage,
        data,
    };
    store
        .write(&job.key, kind, &record)
        .map_err(|err| format!("cannot write the result: {err}"))?;
    Ok(record)
}

/// The URL of a pull request on github.com.
fn pr_url(key: &PrKey) -> String {
    format!(
        "https://github.com/{}/{}/pull/{}",
        key.repo.owner, key.repo.name, key.number
    )
}

/// The env of the process: the `[agent] env` table, then the settings
/// of chaoshub. A setting of chaoshub replaces a value of the table with
/// the same name.
fn process_env(
    config: &AgentConfig,
    kind: ToolKind,
    user_env: std::collections::BTreeMap<String, String>,
    token: &str,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = user_env.into_iter().collect();
    let fallback = toml::Value::Array(
        config
            .fallback_models
            .iter()
            .map(|model| toml::Value::String(model.clone()))
            .collect(),
    );
    env.extend([
        ("GITHUB__USER_TOKEN".to_owned(), toml_string(token)),
        ("CONFIG__MODEL".to_owned(), toml_string(&config.model)),
        ("CONFIG__FALLBACK_MODELS".to_owned(), fallback.to_string()),
        ("CONFIG__PUBLISH_OUTPUT".to_owned(), "false".to_owned()),
        (
            "CONFIG__PROPAGATE_TOOL_ERRORS".to_owned(),
            "true".to_owned(),
        ),
    ]);
    let extra = match kind {
        ToolKind::Review => &config.extra_instructions.review,
        ToolKind::Improve => &config.extra_instructions.improve,
        ToolKind::Describe => &config.extra_instructions.describe,
        ToolKind::Ask => &config.extra_instructions.ask,
    };
    if !extra.is_empty() {
        let name = format!("{}__EXTRA_INSTRUCTIONS", kind.section().to_uppercase());
        env.push((name, toml_string(extra)));
    }
    env
}

/// `text` as a TOML string, so that pr-agent reads it as text and not as
/// a number, a list or a boolean.
fn toml_string(text: &str) -> String {
    toml::Value::String(text.to_owned()).to_string()
}

/// The argv of the run.
fn argv(
    config: &AgentConfig,
    store: &Store,
    job: &Job,
    output: &Path,
) -> Result<Vec<String>, String> {
    if config.command.is_empty() {
        return Err("[agent] command is empty".to_owned());
    }
    let url = pr_url(&job.key);
    match &job.tool {
        Tool::Ask(question) => {
            let shim = store
                .write_shim(SHIM_TEXT)
                .map_err(|err| format!("cannot write the ask script: {err}"))?;
            let mut argv = ask_argv(&config.command, &shim, lookup_path)?;
            argv.extend([url, question.clone()]);
            Ok(argv)
        }
        tool => {
            let mut argv = config.command.clone();
            argv.extend([
                "--pr_url".to_owned(),
                url,
                "--json-output".to_owned(),
                output.to_string_lossy().into_owned(),
                tool.kind().name().to_owned(),
            ]);
            Ok(argv)
        }
    }
}

/// The argv that runs the ask script `shim` with the Python of
/// `command`:
///
/// - One word, for example `["pr-agent"]` of pip or pipx: the run finds
///   the program in `PATH` (`find` does it) and reads its first line. A
///   `#!.../python` line gives that Python. A `#!/usr/bin/env python3`
///   line gives `/usr/bin/env python3`. A `#!/bin/sh` line with an
///   `'''exec' <python>` second line (the form of pip for long paths)
///   gives that Python.
/// - More than one word, for example `["uvx", "--from", "pr-agent",
///   "pr-agent"]`: the last word changes to `python`. So
///   `uvx --from pr-agent python <shim>` runs the script in the env of
///   the tool.
///
/// The `docker run` form cannot work: the last word is the image, and
/// the container cannot see the script.
fn ask_argv(
    command: &[String],
    shim: &Path,
    find: impl Fn(&str) -> Option<PathBuf>,
) -> Result<Vec<String>, String> {
    let mut argv = match command {
        [] => return Err("[agent] command is empty".to_owned()),
        [program] => {
            let path = find(program).ok_or_else(|| format!("cannot find {program}"))?;
            python_of(&path).ok_or_else(|| {
                format!("cannot find the Python of {program}: its first line names no Python")
            })?
        }
        [prefix @ .., _] => {
            let mut argv = prefix.to_vec();
            argv.push("python".to_owned());
            argv
        }
    };
    argv.push(shim.to_string_lossy().into_owned());
    Ok(argv)
}

/// The interpreter words of the script at `path`. `None` when the
/// script names no Python.
fn python_of(path: &Path) -> Option<Vec<String>> {
    let text = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&text[..text.len().min(4096)]).into_owned();
    let mut lines = text.lines();
    let words: Vec<String> = lines
        .next()?
        .strip_prefix("#!")?
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let is_python = |word: &str| {
        Path::new(word)
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains("python"))
    };
    if words.iter().any(|word| is_python(word)) {
        return Some(words);
    }
    let python = lines
        .next()?
        .strip_prefix("'''exec' ")?
        .split_whitespace()
        .next()?;
    is_python(python).then(|| vec![python.to_owned()])
}

/// The path of `program`: as it is when it has a `/`, else the first
/// match in `PATH`.
fn lookup_path(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        return Some(PathBuf::from(program));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

/// Starts `argv` in the repo directory, sends its output lines as
/// progress, and waits for it until `timeout_secs`.
async fn spawn(
    argv: &[String],
    env: &[(String, String)],
    job: &Job,
    timeout_secs: u64,
    tx: &Sender<Msg>,
) -> Result<Finished, String> {
    let program = &argv[0];
    let mut child = Command::new(program)
        .args(&argv[1..])
        .envs(env.iter().map(|(name, value)| (name, value)))
        .current_dir(&job.repo_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| format!("cannot start {program}: {err}"))?;
    let key = TargetKey::Pr(job.key.clone());
    let is_ask = matches!(job.tool, Tool::Ask(_));
    let stdout = child.stdout.take().map(|out| {
        // The answer of ask comes on stdout, so it is no progress.
        let tx = (!is_ask).then(|| tx.clone());
        tokio::spawn(read_lines(out, tx, key.clone()))
    });
    let stderr = child
        .stderr
        .take()
        .map(|err| tokio::spawn(read_lines(err, Some(tx.clone()), key.clone())));
    let readers = [stdout, stderr];
    let waited = tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await;
    let status = match waited {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => {
            abort(&readers);
            return Err(format!("pr-agent failed: {err}"));
        }
        Err(_) => {
            let _ = child.kill().await;
            abort(&readers);
            return Err(format!("pr-agent timed out after {timeout_secs}s"));
        }
    };
    let [stdout, stderr] = readers;
    let (out_last, out_text) = join(stdout).await;
    let (err_last, _) = join(stderr).await;
    Ok(Finished {
        status,
        last_line: err_last.or(out_last),
        stdout: out_text,
    })
}

/// Stops the line readers. A child of a killed process can keep a pipe
/// open, so the run does not wait for the end of the pipes.
fn abort(readers: &[Option<Reader>]) {
    for reader in readers.iter().flatten() {
        reader.abort();
    }
}

/// The result of a line reader. An aborted reader gives nothing.
async fn join(reader: Option<Reader>) -> (Option<String>, String) {
    match reader {
        Some(reader) => reader.await.unwrap_or_default(),
        None => (None, String::new()),
    }
}

/// Reads `stream` line by line. With `tx`, it sends each line as
/// progress. It returns the last line and all the text.
async fn read_lines<R: AsyncRead + Unpin>(
    stream: R,
    tx: Option<Sender<Msg>>,
    key: TargetKey,
) -> (Option<String>, String) {
    let mut lines = BufReader::new(stream).lines();
    let mut last = None;
    let mut text = String::new();
    while let Ok(Some(line)) = lines.next_line().await {
        text.push_str(&line);
        text.push('\n');
        let line = progress_text(&line);
        if line.is_empty() {
            continue;
        }
        if let Some(tx) = &tx {
            let msg = Msg::ActionEvent {
                id: ID,
                key: key.clone(),
                event: ActionEvent::Progress(line.clone()),
            };
            if tx.send(msg).await.is_err() {
                tracing::debug!("app is gone, agent progress dropped");
            }
        }
        last = Some(line);
    }
    (last, text)
}

/// The text of a log line: without the color codes, and without the
/// time, level and source of the log format of pr-agent
/// (`<time> | <level> | <source> - <message>`).
fn progress_text(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // A CSI sequence ends with a letter.
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            plain.push(c);
        }
    }
    let plain = plain.trim();
    match plain.split_once(" | ") {
        Some((_, rest)) => rest
            .split_once(" - ")
            .map_or(plain, |(_, message)| message)
            .trim()
            .to_owned(),
        None => plain.to_owned(),
    }
}

/// The data and the usage of a finished process. An exit code other than
/// 0, or no output, is an error.
fn read_output(
    job: &Job,
    output: &Path,
    finished: Finished,
) -> Result<(serde_json::Value, serde_json::Value), String> {
    if !finished.status.success() {
        let last = finished.last_line.unwrap_or_default();
        return Err(format!("pr-agent failed ({}): {last}", finished.status));
    }
    if matches!(job.tool, Tool::Ask(_)) {
        let answer = finished.stdout.trim();
        if answer.is_empty() {
            return Err("pr-agent gave no answer".to_owned());
        }
        return Ok((
            serde_json::Value::String(answer.to_owned()),
            serde_json::json!({}),
        ));
    }
    let text =
        std::fs::read_to_string(output).map_err(|_| "pr-agent wrote no result".to_owned())?;
    let data: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| format!("the result of pr-agent: {err}"))?;
    let usage = data
        .get("usage")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    Ok((data, usage))
}

/// The ask history with the new answer first. `answer` is a JSON string.
fn ask_history(
    store: &Store,
    job: &Job,
    question: &str,
    answer: &serde_json::Value,
) -> serde_json::Value {
    let old = match store.read(&job.key, ToolKind::Ask) {
        Ok(Some(record)) => record.ask_entries(),
        Ok(None) => Vec::new(),
        Err(err) => {
            tracing::warn!(%err, "the old ask history is unreadable");
            Vec::new()
        }
    };
    let entry = AskEntry {
        question: question.to_owned(),
        answer: answer.as_str().unwrap_or_default().to_owned(),
        head_sha: job.head_sha.clone(),
        at: Utc::now(),
    };
    let entries: Vec<AskEntry> = std::iter::once(entry).chain(old).collect();
    serde_json::to_value(entries).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use pretty_assertions::assert_eq;
    use tokio::sync::mpsc;

    use super::*;
    use crate::config::schema::{EnvValue, ExtraInstructions};
    use crate::domain::keys::RepoKey;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn key() -> PrKey {
        PrKey {
            repo: RepoKey {
                owner: "chaosteil".into(),
                name: "chaoshub-test".into(),
            },
            number: 7,
        }
    }

    /// A test setup: a state directory, a repo directory and the log
    /// prefix of the fakes.
    struct Setup {
        _dir: tempfile::TempDir,
        store: Store,
        repo: PathBuf,
        log: PathBuf,
    }

    fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        Setup {
            store: Store::new(dir.path().join("state")),
            repo,
            log: dir.path().join("fake"),
            _dir: dir,
        }
    }

    impl Setup {
        /// The config with the fake pr-agent. `mode` sets `FAKE_MODE`.
        fn config(&self, mode: &str) -> AgentConfig {
            AgentConfig {
                command: vec![fixture("fake-pr-agent").to_string_lossy().into_owned()],
                fallback_models: vec!["openai/gpt-6-luna".into()],
                env: BTreeMap::from([
                    (
                        "FAKE_LOG".to_owned(),
                        EnvValue::Literal(self.log.to_string_lossy().into_owned()),
                    ),
                    ("FAKE_MODE".to_owned(), EnvValue::Literal(mode.into())),
                    (
                        "CONFIG__PUBLISH_OUTPUT".to_owned(),
                        EnvValue::Literal("true".into()),
                    ),
                ]),
                extra_instructions: ExtraInstructions {
                    review: "Focus: on bugs".into(),
                    ..ExtraInstructions::default()
                },
                ..AgentConfig::default()
            }
        }

        fn job(&self, tool: Tool) -> Job {
            Job {
                key: key(),
                head_sha: "3a1f2c0".into(),
                repo_path: self.repo.clone(),
                tool,
            }
        }

        /// The lines that the fake wrote to `<log>.<suffix>`.
        fn logged(&self, suffix: &str) -> Vec<String> {
            let path = PathBuf::from(format!("{}.{suffix}", self.log.display()));
            std::fs::read_to_string(path)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        /// The value of `name` in the env that the fake saw.
        fn env_of(&self, name: &str) -> Option<String> {
            let prefix = format!("{name}=");
            self.logged("env")
                .into_iter()
                .find_map(|line| line.strip_prefix(&prefix).map(str::to_owned))
        }

        /// The names of the files in the directory of the pull request.
        fn files(&self) -> Vec<String> {
            let mut names: Vec<_> = std::fs::read_dir(self.store.pr_dir(&key()))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    fn token() -> TokenSource {
        TokenSource::command("echo", &["gho_test"])
    }

    /// The progress texts in `rx`.
    fn progress(rx: &mut mpsc::Receiver<Msg>) -> Vec<String> {
        let mut texts = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let Msg::ActionEvent {
                id,
                key: TargetKey::Pr(pr),
                event: ActionEvent::Progress(text),
            } = msg
            {
                assert_eq!((id, pr), (ID, key()));
                texts.push(text);
            }
        }
        texts
    }

    #[tokio::test]
    async fn review_passes_the_argv_and_the_env_and_stores_the_result() {
        let setup = setup();
        let config = setup.config("ok");
        let (tx, mut rx) = mpsc::channel(256);
        let record = run(
            &setup.job(Tool::Review),
            &config,
            &setup.store,
            &token(),
            &tx,
        )
        .await
        .unwrap();

        let argv = setup.logged("argv");
        assert_eq!(
            argv[..2],
            [
                "--pr_url",
                "https://github.com/chaosteil/chaoshub-test/pull/7"
            ]
        );
        assert_eq!(argv[2], "--json-output");
        assert!(argv[3].contains(".review.out-"), "{}", argv[3]);
        assert_eq!(argv[4..], ["review"]);
        assert_eq!(
            setup.logged("cwd"),
            [setup.repo.canonicalize().unwrap().to_string_lossy()]
        );
        assert_eq!(setup.env_of("GITHUB__USER_TOKEN").unwrap(), "\"gho_test\"");
        assert_eq!(
            setup.env_of("CONFIG__MODEL").unwrap(),
            "\"anthropic/claude-sonnet-5-5\""
        );
        assert_eq!(
            setup.env_of("CONFIG__FALLBACK_MODELS").unwrap(),
            "[\"openai/gpt-6-luna\"]"
        );
        // A value of chaoshub wins over the same name in `[agent] env`.
        assert_eq!(setup.env_of("CONFIG__PUBLISH_OUTPUT").unwrap(), "false");
        assert_eq!(
            setup.env_of("CONFIG__PROPAGATE_TOOL_ERRORS").unwrap(),
            "true"
        );
        assert_eq!(
            setup.env_of("PR_REVIEWER__EXTRA_INSTRUCTIONS").unwrap(),
            "\"Focus: on bugs\""
        );
        assert_eq!(setup.env_of("PR_QUESTIONS__EXTRA_INSTRUCTIONS"), None);

        assert_eq!(progress(&mut rx), ["starting review", "review done"]);
        assert_eq!(record.tool, "review");
        assert_eq!(record.pr, 7);
        assert_eq!(record.head_sha, "3a1f2c0");
        assert_eq!(record.total_tokens(), Some(12000));
        assert_eq!(
            record.data["review"]["key_issues_to_review"][0]["relevant_file"],
            "src/lib.rs"
        );
        assert_eq!(
            setup.store.read(&key(), ToolKind::Review).unwrap(),
            Some(record)
        );
        assert_eq!(setup.files(), ["review.json"]);
    }

    #[tokio::test]
    async fn improve_and_describe_store_their_json() {
        let setup = setup();
        let config = setup.config("ok");
        let (tx, _rx) = mpsc::channel(256);
        for tool in [Tool::Improve, Tool::Describe] {
            run(&setup.job(tool), &config, &setup.store, &token(), &tx)
                .await
                .unwrap();
        }
        let improve = setup
            .store
            .read(&key(), ToolKind::Improve)
            .unwrap()
            .unwrap();
        assert_eq!(
            improve.data["code_suggestions"][0]["label"],
            "possible issue"
        );
        let describe = setup
            .store
            .read(&key(), ToolKind::Describe)
            .unwrap()
            .unwrap();
        assert_eq!(describe.data["title"], "Add a greeting");
        assert_eq!(setup.files(), ["describe.json", "improve.json"]);
    }

    #[tokio::test]
    async fn a_failed_run_keeps_the_old_file() {
        let setup = setup();
        let (tx, _rx) = mpsc::channel(256);
        let job = setup.job(Tool::Review);
        let old = run(&job, &setup.config("ok"), &setup.store, &token(), &tx)
            .await
            .unwrap();
        let err = run(&job, &setup.config("fail"), &setup.store, &token(), &tx)
            .await
            .unwrap_err();
        assert!(err.starts_with("pr-agent failed ("), "{err}");
        assert!(err.ends_with("model error: no key"), "{err}");
        assert_eq!(
            setup.store.read(&key(), ToolKind::Review).unwrap(),
            Some(old)
        );
        assert_eq!(setup.files(), ["review.json"]);
    }

    #[tokio::test]
    async fn a_run_without_json_fails() {
        let setup = setup();
        let (tx, _rx) = mpsc::channel(256);
        let err = run(
            &setup.job(Tool::Review),
            &setup.config("nojson"),
            &setup.store,
            &token(),
            &tx,
        )
        .await
        .unwrap_err();
        assert_eq!(err, "pr-agent wrote no result");
        assert_eq!(setup.files(), [""; 0]);
    }

    #[tokio::test]
    async fn the_timeout_kills_the_process() {
        let setup = setup();
        let config = AgentConfig {
            timeout_secs: 1,
            ..setup.config("sleep")
        };
        let (tx, _rx) = mpsc::channel(256);
        let started = std::time::Instant::now();
        let err = run(
            &setup.job(Tool::Review),
            &config,
            &setup.store,
            &token(),
            &tx,
        )
        .await
        .unwrap_err();
        assert_eq!(err, "pr-agent timed out after 1s");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(setup.files(), [""; 0]);
    }

    #[tokio::test]
    async fn a_failed_env_command_stops_the_run() {
        let setup = setup();
        let mut config = setup.config("ok");
        config.env.insert(
            "KEY".into(),
            EnvValue::Command {
                command: "echo locked >&2; exit 1".into(),
            },
        );
        let (tx, _rx) = mpsc::channel(256);
        let err = run(
            &setup.job(Tool::Review),
            &config,
            &setup.store,
            &token(),
            &tx,
        )
        .await
        .unwrap_err();
        assert_eq!(err, "env KEY: the command failed: locked");
        assert!(!setup.log.with_extension("argv").exists());
    }

    /// A `pr-agent` script in `dir` whose first line names the fake
    /// Python.
    fn python_script(dir: &Path) -> PathBuf {
        let path = dir.join("pr-agent");
        let text = format!("#!{}\n", fixture("fake-python").display());
        std::fs::write(&path, text).unwrap();
        path
    }

    #[tokio::test]
    async fn ask_runs_the_script_and_keeps_a_history() {
        let setup = setup();
        let program = python_script(&setup.repo);
        let config = AgentConfig {
            command: vec![program.to_string_lossy().into_owned()],
            ..setup.config("ok")
        };
        let (tx, mut rx) = mpsc::channel(256);
        for question in ["Why?", "How?"] {
            run(
                &setup.job(Tool::Ask(question.into())),
                &config,
                &setup.store,
                &token(),
                &tx,
            )
            .await
            .unwrap();
        }
        let shim = setup.store.shim_path();
        assert_eq!(std::fs::read_to_string(&shim).unwrap(), SHIM_TEXT);
        assert_eq!(
            setup.logged("argv"),
            [
                shim.to_string_lossy().as_ref(),
                "https://github.com/chaosteil/chaoshub-test/pull/7",
                "How?"
            ]
        );
        assert_eq!(setup.env_of("CONFIG__PUBLISH_OUTPUT").unwrap(), "false");
        // stdout holds the answer, so only stderr gives progress.
        assert_eq!(progress(&mut rx), ["answering", "answering"]);
        let record = setup.store.read(&key(), ToolKind::Ask).unwrap().unwrap();
        let entries = record.ask_entries();
        let questions: Vec<_> = entries
            .iter()
            .map(|entry| entry.question.as_str())
            .collect();
        assert_eq!(questions, ["How?", "Why?"]);
        assert_eq!(entries[0].answer, "The answer to How? is 42.\nIt is short.");
        assert_eq!(entries[0].head_sha, "3a1f2c0");
    }

    #[tokio::test]
    async fn a_failed_ask_keeps_the_history() {
        let setup = setup();
        let program = python_script(&setup.repo);
        let config = AgentConfig {
            command: vec![program.to_string_lossy().into_owned()],
            ..setup.config("ok")
        };
        let (tx, _rx) = mpsc::channel(256);
        let job = setup.job(Tool::Ask("Why?".into()));
        run(&job, &config, &setup.store, &token(), &tx)
            .await
            .unwrap();
        let failing = AgentConfig {
            command: config.command.clone(),
            ..setup.config("fail")
        };
        let err = run(&job, &failing, &setup.store, &token(), &tx)
            .await
            .unwrap_err();
        assert!(err.ends_with("no answer"), "{err}");
        let record = setup.store.read(&key(), ToolKind::Ask).unwrap().unwrap();
        assert_eq!(record.ask_entries().len(), 1);
    }

    #[test]
    fn ask_argv_finds_the_python_of_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let shim = Path::new("/state/chaoshub-agent-ask");
        let script = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            path
        };
        let words = |argv: &[&str]| argv.iter().map(|&word| word.to_owned()).collect::<Vec<_>>();
        let cases = [
            (
                script("pip", "#!/venv/bin/python3.13\nimport sys\n"),
                words(&["/venv/bin/python3.13", "/state/chaoshub-agent-ask"]),
            ),
            (
                script("env", "#!/usr/bin/env python3\n"),
                words(&["/usr/bin/env", "python3", "/state/chaoshub-agent-ask"]),
            ),
            (
                script(
                    "long",
                    "#!/bin/sh\n'''exec' /very/long/venv/bin/python \"$0\" \"$@\"\n' '''\n",
                ),
                words(&["/very/long/venv/bin/python", "/state/chaoshub-agent-ask"]),
            ),
        ];
        for (path, expected) in cases {
            let find = |_: &str| Some(path.clone());
            assert_eq!(
                ask_argv(&words(&["pr-agent"]), shim, find).unwrap(),
                expected
            );
        }
        let shell = script("shell", "#!/bin/sh\necho hi\n");
        let err = ask_argv(&words(&["pr-agent"]), shim, |_| Some(shell.clone())).unwrap_err();
        assert_eq!(
            err,
            "cannot find the Python of pr-agent: its first line names no Python"
        );
        let err = ask_argv(&words(&["pr-agent"]), shim, |_| None).unwrap_err();
        assert_eq!(err, "cannot find pr-agent");
        assert_eq!(
            ask_argv(
                &words(&["uvx", "--from", "pr-agent", "pr-agent"]),
                shim,
                |_| None
            )
            .unwrap(),
            words(&[
                "uvx",
                "--from",
                "pr-agent",
                "python",
                "/state/chaoshub-agent-ask"
            ])
        );
    }

    #[test]
    fn progress_text_strips_the_log_format() {
        let cases = [
            (
                "\u{1b}[32m2026-11-05 17:00:00.000\u{1b}[0m | \u{1b}[1mINFO    \u{1b}[0m | pr_agent.tools.pr_reviewer:run:84 - Reviewing PR - #7",
                "Reviewing PR - #7",
            ),
            ("  plain line  ", "plain line"),
            ("a | b", "a | b"),
            ("", ""),
        ];
        for (line, expected) in cases {
            assert_eq!(progress_text(line), expected, "{line}");
        }
    }
}
