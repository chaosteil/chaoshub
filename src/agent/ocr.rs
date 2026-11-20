//! One run of Open Code Review, the `ocr` CLI.
//!
//! ocr reviews a diff of the local repo. It does not read the pull
//! request on GitHub, and it writes nothing to GitHub. So the run first
//! fetches the base branch and the head of the pull request from origin,
//! with the prefetch refs of `vcs::git`, under the lock of the repo. Then
//! it starts the `[agent.open_code_review] command` in the repo:
//!
//! ```text
//! <command> review --repo <repo> --from <base commit> --to <head commit>
//!     --format json --audience human --color never --output <tmp>
//!     [--background-file <tmp>] [--provider <p>] [--model <m>] [<args>]
//! ```
//!
//! ocr reviews the diff from the merge base of the two commits to the
//! head, the same diff as the pull request. `--background-file` holds the
//! title and the body of the pull request and the extra instructions.
//! With `--format json`, ocr writes its progress lines to stderr. Each
//! line goes to `app` as `ActionEvent::Progress`.
//!
//! ocr has only the review tool. Its JSON goes to `review.json` in the
//! form of a pr-agent review (see `domain::agent::ocr_review`). ocr exits
//! with 1 when a part of the review failed. That is an error of the run,
//! also when ocr has some comments. A failed run writes no result file,
//! so the old file stays.

use std::path::Path;

use chrono::Utc;
use tokio::sync::mpsc::Sender;

use crate::agent::run::{Finished, Job, Process, exit_error, plain_text, spawn};
use crate::agent::store::Store;
use crate::bus::Msg;
use crate::config::schema::{OcrConfig, resolve_env};
use crate::domain::agent::{Backend, Record, ToolKind, VERSION, ocr_review};
use crate::vcs::git;
use crate::vcs::lock::RepoLocks;

/// The base commit and the head commit of a review.
struct Range {
    base: String,
    head: String,
}

/// Runs ocr for `job` and writes the result to `store`, like
/// `run::run`.
pub(crate) async fn run(
    job: &Job,
    config: &OcrConfig,
    store: &Store,
    locks: &RepoLocks,
    tx: &Sender<Msg>,
) -> Result<Record, String> {
    let started_at = Utc::now();
    let kind = job.tool.kind();
    if kind != ToolKind::Review {
        return Err(format!("ocr has no {}", kind.name()));
    }
    if config.command.is_empty() {
        return Err("[agent.open_code_review] command is empty".to_owned());
    }
    let env: Vec<(String, String)> = resolve_env(&config.env, |name| std::env::var(name).ok())
        .await
        .map_err(|err| err.to_string())?
        .into_iter()
        .collect();
    let range = fetch(locks, job).await?;
    let output = store
        .output_path(&job.key, kind)
        .map_err(|err| format!("cannot write the result: {err}"))?;
    let background = background(job, config);
    let context = if background.is_empty() {
        None
    } else {
        let path = store
            .context_path(&job.key, kind)
            .and_then(|path| std::fs::write(&path, &background).map(|()| path))
            .map_err(|err| format!("cannot write the background: {err}"))?;
        Some(path)
    };
    let argv = argv(config, job, &range, &output, context.as_deref());
    let process = Process {
        argv: &argv,
        env: &env,
        cwd: &job.repo_path,
        timeout_secs: config.timeout_secs,
        backend: Backend::OpenCodeReview,
        stdout_progress: true,
        progress: progress_text,
    };
    let result = spawn(&process, &job.key, tx).await;
    let data = result.and_then(|finished| read_output(&output, &finished));
    let _ = std::fs::remove_file(&output);
    if let Some(context) = &context {
        let _ = std::fs::remove_file(context);
    }
    let data = data?;
    let record = Record {
        version: VERSION,
        backend: Backend::OpenCodeReview,
        tool: kind.name().to_owned(),
        pr: job.key.number,
        head_sha: range.head,
        model: model_of(&data),
        started_at,
        finished_at: Utc::now(),
        usage: usage_of(&data),
        data: ocr_review(&data),
    };
    store
        .write(&job.key, kind, &record)
        .map_err(|err| format!("cannot write the result: {err}"))?;
    Ok(record)
}

/// Fetches the base branch and the head of the pull request of `job`,
/// and reads their commits.
async fn fetch(locks: &RepoLocks, job: &Job) -> Result<Range, String> {
    let repo = &job.repo_path;
    git::prefetch_base(locks, repo, &job.base_ref)
        .await
        .map_err(|err| format!("cannot fetch {}: {err}", job.base_ref))?;
    git::prefetch_head(locks, repo, job.key.number)
        .await
        .map_err(|err| format!("cannot fetch the head: {err}"))?;
    let commit = |rev: String| async move {
        git::commit_of(repo, &rev)
            .await
            .map_err(|err| format!("cannot read {rev}: {err}"))
    };
    Ok(Range {
        base: commit(git::base_ref_name(&job.base_ref)).await?,
        head: commit(git::head_ref_name(job.key.number)).await?,
    })
}

/// The background text of the review: the context of the job, then the
/// extra instructions. Empty when both are empty.
fn background(job: &Job, config: &OcrConfig) -> String {
    [job.context.trim(), config.extra_instructions.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The argv of the run.
fn argv(
    config: &OcrConfig,
    job: &Job,
    range: &Range,
    output: &Path,
    context: Option<&Path>,
) -> Vec<String> {
    let mut argv = config.command.clone();
    argv.extend(
        [
            "review",
            "--repo",
            &job.repo_path.to_string_lossy(),
            "--from",
            &range.base,
            "--to",
            &range.head,
            "--format",
            "json",
            "--audience",
            "human",
            "--color",
            "never",
            "--output",
            &output.to_string_lossy(),
        ]
        .map(str::to_owned),
    );
    if let Some(context) = context {
        argv.push("--background-file".to_owned());
        argv.push(context.to_string_lossy().into_owned());
    }
    for (flag, value) in [("--provider", &config.provider), ("--model", &config.model)] {
        if !value.is_empty() {
            argv.push(flag.to_owned());
            argv.push(value.clone());
        }
    }
    argv.extend(config.args.iter().cloned());
    argv
}

/// The text of a progress line of ocr: the `plain_text`, without the
/// `[ocr]` prefix. A failed review also prints its JSON result to stderr.
/// A line of that JSON gives no text.
fn progress_text(line: &str) -> String {
    let plain = plain_text(line);
    if let Some(rest) = plain.strip_prefix("[ocr]") {
        return rest.trim().to_owned();
    }
    if plain.starts_with(['{', '}', '[', ']', '"']) {
        return String::new();
    }
    plain
}

/// The JSON output of a finished run. An exit code other than 0, or no
/// output, is an error.
fn read_output(output: &Path, finished: &Finished) -> Result<serde_json::Value, String> {
    if let Some(err) = exit_error(finished) {
        return Err(err);
    }
    let text = std::fs::read_to_string(output).map_err(|_| "ocr wrote no result".to_owned())?;
    serde_json::from_str(&text).map_err(|err| format!("the result of ocr: {err}"))
}

/// The model of the output: `<provider>/<model>`, or only the model when
/// ocr names no provider. Empty when ocr names no model.
fn model_of(data: &serde_json::Value) -> String {
    let field = |name: &str| {
        data.pointer(&format!("/llm/{name}"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
    };
    match (field("provider"), field("model")) {
        ("", model) => model.to_owned(),
        (provider, model) => format!("{provider}/{model}"),
    }
}

/// The token counts of the output: the `*_tokens` fields of its
/// `summary`, for example `total_tokens`.
fn usage_of(data: &serde_json::Value) -> serde_json::Value {
    let usage: serde_json::Map<String, serde_json::Value> = data
        .get("summary")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(name, value)| name.ends_with("_tokens") && value.is_u64())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    serde_json::Value::Object(usage)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::Duration;

    use pretty_assertions::assert_eq;
    use tokio::sync::mpsc;

    use super::*;
    use crate::bus::{ActionEvent, TargetKey};
    use crate::config::schema::EnvValue;
    use crate::domain::agent::Tool;
    use crate::domain::keys::{PrKey, RepoKey};
    use crate::vcs::testrepo::{git, git_missing, git_only_repo};

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

    /// A test setup: a git-only repo whose origin has pull request #7
    /// (the branch `me/feature` on `main`), a state directory and the log
    /// prefix of the fake.
    struct Setup {
        _dir: tempfile::TempDir,
        store: Store,
        repo: PathBuf,
        log: PathBuf,
        /// The commits of `main` and of `me/feature`.
        base: String,
        head: String,
    }

    /// The setup, or `None` without git.
    fn setup() -> Option<Setup> {
        if git_missing() {
            eprintln!("skipped: git is missing");
            return None;
        }
        let dir = tempfile::tempdir().unwrap();
        let repo = git_only_repo(dir.path());
        git(
            &repo,
            &["push", "--quiet", "origin", "me/feature:refs/pull/7/head"],
        );
        Some(Setup {
            store: Store::new(dir.path().join("state")),
            base: git(&repo, &["rev-parse", "main"]),
            head: git(&repo, &["rev-parse", "me/feature"]),
            repo,
            log: dir.path().join("fake"),
            _dir: dir,
        })
    }

    impl Setup {
        /// The config with the fake ocr. `mode` sets `FAKE_MODE`.
        fn config(&self, mode: &str) -> OcrConfig {
            OcrConfig {
                command: vec![fixture("fake-ocr").to_string_lossy().into_owned()],
                provider: "anthropic".into(),
                model: "claude-opus-4-6".into(),
                args: vec!["--effort".into(), "high".into()],
                env: BTreeMap::from([
                    (
                        "FAKE_LOG".to_owned(),
                        EnvValue::Literal(self.log.to_string_lossy().into_owned()),
                    ),
                    ("FAKE_MODE".to_owned(), EnvValue::Literal(mode.into())),
                ]),
                extra_instructions: "Focus on bugs.".into(),
                ..OcrConfig::default()
            }
        }

        fn job(&self, tool: Tool) -> Job {
            Job {
                key: key(),
                // The head of the job is older than the fetched head.
                head_sha: "3a1f2c0".into(),
                repo_path: self.repo.clone(),
                base_ref: "main".into(),
                context: "# Add a greeting\n\nThe body.".into(),
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

    /// The progress texts in `rx`.
    fn progress(rx: &mut mpsc::Receiver<Msg>) -> Vec<String> {
        let mut texts = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let Msg::ActionEvent {
                key: TargetKey::Pr(pr),
                event: ActionEvent::Progress(text),
                ..
            } = msg
            {
                assert_eq!(pr, key());
                texts.push(text);
            }
        }
        texts
    }

    #[tokio::test]
    async fn review_fetches_the_pull_request_and_stores_the_review() {
        let Some(setup) = setup() else { return };
        let (tx, mut rx) = mpsc::channel(256);
        let locks = RepoLocks::default();
        let record = run(
            &setup.job(Tool::Review),
            &setup.config("ok"),
            &setup.store,
            &locks,
            &tx,
        )
        .await
        .unwrap();

        let argv = setup.logged("argv");
        let repo = setup.repo.to_string_lossy();
        assert_eq!(
            argv[..14],
            [
                "review",
                "--repo",
                repo.as_ref(),
                "--from",
                &setup.base,
                "--to",
                &setup.head,
                "--format",
                "json",
                "--audience",
                "human",
                "--color",
                "never",
                "--output",
            ]
        );
        assert!(argv[14].contains(".review.out-"), "{}", argv[14]);
        assert_eq!(argv[15], "--background-file");
        assert!(argv[16].contains(".review.context-"), "{}", argv[16]);
        assert_eq!(
            argv[17..],
            [
                "--provider",
                "anthropic",
                "--model",
                "claude-opus-4-6",
                "--effort",
                "high"
            ]
        );
        assert_eq!(
            setup.logged("background"),
            ["# Add a greeting", "", "The body.", "", "Focus on bugs."]
        );
        assert_eq!(
            setup.logged("cwd"),
            [setup.repo.canonicalize().unwrap().to_string_lossy()]
        );
        assert_eq!(
            progress(&mut rx),
            [
                "1 file(s) changed, reviewing 1",
                "Plan completed for src/lib.rs"
            ]
        );
        assert_eq!(record.backend, Backend::OpenCodeReview);
        assert_eq!(record.tool, "review");
        assert_eq!(record.pr, 7);
        assert_eq!(record.head_sha, setup.head, "the fetched head");
        assert_eq!(record.model, "anthropic/claude-opus-4-6");
        assert_eq!(record.total_tokens(), Some(21344));
        assert_eq!(
            record.data["review"]["key_issues_to_review"][0]["issue_header"],
            "High bug"
        );
        assert_eq!(
            setup.store.read(&key(), ToolKind::Review).unwrap(),
            Some(record)
        );
        assert_eq!(setup.files(), ["review.json"], "no temporary file stays");
        for name in ["refs/chaoshub/base/main", "refs/chaoshub/pr/7/head"] {
            git(&setup.repo, &["rev-parse", "--verify", "--quiet", name]);
        }
    }

    #[tokio::test]
    async fn a_failed_run_keeps_the_old_file() {
        let Some(setup) = setup() else { return };
        let (tx, _rx) = mpsc::channel(256);
        let locks = RepoLocks::default();
        let job = setup.job(Tool::Review);
        let old = run(&job, &setup.config("ok"), &setup.store, &locks, &tx)
            .await
            .unwrap();
        let (fail_tx, mut fail_rx) = mpsc::channel(256);
        let err = run(&job, &setup.config("fail"), &setup.store, &locks, &fail_tx)
            .await
            .unwrap_err();
        assert_eq!(
            progress(&mut fail_rx),
            [
                "1 file(s) changed, reviewing 1",
                "Error: resolve LLM endpoint: no valid LLM endpoint configured"
            ],
            "the JSON lines on stderr give no progress"
        );
        assert_eq!(
            err,
            "exit status 1: Error: resolve LLM endpoint: no valid LLM endpoint configured"
        );
        let err = run(&job, &setup.config("nojson"), &setup.store, &locks, &tx)
            .await
            .unwrap_err();
        assert_eq!(err, "ocr wrote no result");
        assert_eq!(
            setup.store.read(&key(), ToolKind::Review).unwrap(),
            Some(old)
        );
        assert_eq!(setup.files(), ["review.json"]);
    }

    #[tokio::test]
    async fn the_timeout_kills_ocr() {
        let Some(setup) = setup() else { return };
        let config = OcrConfig {
            timeout_secs: 1,
            ..setup.config("sleep")
        };
        let (tx, _rx) = mpsc::channel(256);
        let started = std::time::Instant::now();
        let err = run(
            &setup.job(Tool::Review),
            &config,
            &setup.store,
            &RepoLocks::default(),
            &tx,
        )
        .await
        .unwrap_err();
        assert_eq!(err, "ocr timed out after 1s");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(setup.files(), [""; 0]);
    }

    #[tokio::test]
    async fn a_missing_pull_request_stops_the_run_before_ocr() {
        let Some(setup) = setup() else { return };
        let job = Job {
            key: PrKey { number: 8, ..key() },
            ..setup.job(Tool::Review)
        };
        let (tx, _rx) = mpsc::channel(256);
        let err = run(
            &job,
            &setup.config("ok"),
            &setup.store,
            &RepoLocks::default(),
            &tx,
        )
        .await
        .unwrap_err();
        assert!(err.starts_with("cannot fetch the head: "), "{err}");
        assert!(!setup.log.with_extension("argv").exists());
    }

    #[tokio::test]
    async fn the_other_tools_do_not_run() {
        let Some(setup) = setup() else { return };
        let (tx, _rx) = mpsc::channel(256);
        for tool in [Tool::Improve, Tool::Describe, Tool::Ask("Why?".into())] {
            let name = tool.kind().name();
            let err = run(
                &setup.job(tool),
                &setup.config("ok"),
                &setup.store,
                &RepoLocks::default(),
                &tx,
            )
            .await
            .unwrap_err();
            assert_eq!(err, format!("ocr has no {name}"));
        }
    }

    #[test]
    fn progress_text_strips_the_prefix_and_the_colors() {
        let cases = [
            (
                "[ocr] Summary: 9 file(s) reviewed",
                "Summary: 9 file(s) reviewed",
            ),
            (
                "\u{1b}[2m[ocr]   ▶ file_read \"a.rs\"\u{1b}[0m",
                "▶ file_read \"a.rs\"",
            ),
            ("plain | line - x", "plain | line - x"),
            ("Error: review failed", "Error: review failed"),
            ("  \"status\": \"failed\",", ""),
            ("}", ""),
        ];
        for (line, expected) in cases {
            assert_eq!(progress_text(line), expected, "{line}");
        }
    }

    #[test]
    fn model_and_usage_of_the_output() {
        let data = serde_json::json!({
            "llm": {"provider": "anthropic", "model": "claude-opus-4-6"},
            "summary": {"files_reviewed": 9, "total_tokens": 21344, "input_tokens": 18012,
                        "output_tokens": 3332, "elapsed": "1m12s"}
        });
        assert_eq!(model_of(&data), "anthropic/claude-opus-4-6");
        assert_eq!(
            usage_of(&data),
            serde_json::json!({"total_tokens": 21344, "input_tokens": 18012, "output_tokens": 3332})
        );
        let data = serde_json::json!({"llm": {"model": "gpt-5.4"}});
        assert_eq!(model_of(&data), "gpt-5.4");
        assert_eq!(usage_of(&data), serde_json::json!({}));
    }
}
