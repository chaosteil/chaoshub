//! Live tests against the private scratch repository
//! `chaosteil/chaoshub-test`.
//!
//! The tests run only when `CHAOSHUB_LIVE_TESTS=1` is set. Without it, each
//! test prints a message and passes. Run them with:
//!
//! ```text
//! CHAOSHUB_LIVE_TESTS=1 cargo nextest run -E 'test(live)' --no-capture
//! ```
//!
//! The tests use `gh` and `git` with the credentials of the user. They write
//! only to `chaosteil/chaoshub-test`.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, bail, ensure};
use chaoshub::testing::{LiveClient, LiveMergeProgress, LivePr};
use serde_json::{Value, json};

const OWNER: &str = "chaosteil";
const NAME: &str = "chaoshub-test";
const REPO: &str = "chaosteil/chaoshub-test";
/// The HTTPS remote. `git` reads the credentials from `gh auth git-credential`,
/// because the ssh agent can ask for a confirmation that a test cannot give.
const REMOTE: &str = "https://github.com/chaosteil/chaoshub-test.git";
/// The REST API version of the stacks endpoints.
const STACKS_API_VERSION: &str = "2026-03-10";
/// The maximum time of the merge poll.
const MERGE_TIMEOUT: Duration = Duration::from_secs(300);

/// True when the live tests are on. Else it prints why `test` skips.
fn enabled(test: &str) -> bool {
    if std::env::var("CHAOSHUB_LIVE_TESTS").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("skipped {test}: set CHAOSHUB_LIVE_TESTS=1 to run the live tests");
    false
}

/// Runs `cmd` and returns its standard output. A failed exit is an error
/// with the output and the error output.
fn run(cmd: &mut Command, input: Option<&[u8]>) -> anyhow::Result<String> {
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn().with_context(|| format!("start {cmd:?}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(input)?;
    }
    let output = child.wait_with_output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        bail!(
            "{cmd:?} failed ({}): {} {}",
            output.status,
            stdout.trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(stdout)
}

/// Runs `git` with `args` in `dir`.
fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    run(
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "commit.gpgsign=false", "-c", "credential.helper="])
            .args(["-c", "credential.helper=!gh auth git-credential"])
            .args(args),
        None,
    )
}

/// Sends a REST request with `gh api` and returns the JSON answer. An empty
/// answer is `Value::Null`. `version` sets the `X-GitHub-Api-Version`
/// header.
fn gh_api(
    method: &str,
    path: &str,
    body: Option<&Value>,
    version: Option<&str>,
) -> anyhow::Result<Value> {
    let mut cmd = Command::new("gh");
    cmd.args(["api", "-X", method, path])
        .args(["-H", "Accept: application/vnd.github+json"]);
    if let Some(version) = version {
        cmd.args(["-H", &format!("X-GitHub-Api-Version: {version}")]);
    }
    let input = body.map(serde_json::to_vec).transpose()?;
    if input.is_some() {
        cmd.args(["--input", "-"]);
    }
    let stdout = run(&mut cmd, input.as_deref())?;
    if stdout.trim().is_empty() {
        return Ok(Value::Null);
    }
    Ok(serde_json::from_str(&stdout)?)
}

/// A short random suffix for the branch names.
fn suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_nanos());
    format!(
        "{:08x}",
        (nanos ^ u128::from(std::process::id())) & 0xffff_ffff
    )
}

/// Removes what the test made on GitHub when it is dropped: it unstacks the
/// stack and closes each open pull request, then deletes each branch.
#[derive(Debug, Default)]
struct Cleanup {
    branches: Vec<String>,
    /// Pull request numbers, bottom first.
    prs: Vec<u64>,
    stack: Option<u64>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let open: Vec<u64> = self
            .prs
            .iter()
            .copied()
            .filter(|number| {
                gh_api("GET", &format!("repos/{REPO}/pulls/{number}"), None, None)
                    .is_ok_and(|pr| pr["state"] == "open")
            })
            .collect();
        if let (Some(stack), false) = (self.stack, open.is_empty()) {
            let path = format!("repos/{REPO}/stacks/{stack}/unstack");
            let result = gh_api("POST", &path, None, Some(STACKS_API_VERSION));
            eprintln!("cleanup: unstack #{stack}: {result:?}");
        }
        for number in open.iter().rev() {
            let path = format!("repos/{REPO}/pulls/{number}");
            let result = gh_api("PATCH", &path, Some(&json!({ "state": "closed" })), None);
            eprintln!("cleanup: close #{number}: ok={}", result.is_ok());
        }
        for branch in self.branches.iter().rev() {
            let path = format!("repos/{REPO}/git/refs/heads/{branch}");
            let exists = gh_api("GET", &path, None, None).is_ok();
            if exists {
                let result = gh_api("DELETE", &path, None, None);
                eprintln!("cleanup: delete {branch}: ok={}", result.is_ok());
            }
        }
    }
}

/// Makes a git checkout of the test repository in `dir` with `main` checked
/// out. When `main` has no commit, it makes an initial commit and pushes it.
/// It returns the commit of `main`.
fn checkout(dir: &Path) -> anyhow::Result<String> {
    git(dir, &["init", "-q", "-b", "main"])?;
    git(dir, &["remote", "add", "origin", REMOTE])?;
    let heads = git(dir, &["ls-remote", "--heads", "origin", "main"])?;
    if heads.trim().is_empty() {
        std::fs::write(
            dir.join("README.md"),
            "# chaoshub-test\n\nScratch repository of the chaoshub live tests.\n",
        )?;
        git(dir, &["add", "README.md"])?;
        git(dir, &["commit", "-q", "-m", "Initial commit"])?;
        git(dir, &["push", "-q", "origin", "main"])?;
    } else {
        git(dir, &["fetch", "-q", "origin", "main"])?;
        git(dir, &["checkout", "-q", "-B", "main", "FETCH_HEAD"])?;
    }
    Ok(git(dir, &["rev-parse", "HEAD"])?.trim().to_owned())
}

/// Makes branch `branch` from the current commit with one commit that adds
/// the file `file`.
fn commit_branch(dir: &Path, branch: &str, file: &str) -> anyhow::Result<()> {
    git(dir, &["checkout", "-q", "-b", branch])?;
    std::fs::create_dir_all(dir.join("live"))?;
    std::fs::write(dir.join("live").join(file), format!("{branch}\n"))?;
    git(dir, &["add", "live"])?;
    git(dir, &["commit", "-q", "-m", &format!("Add {file}")])?;
    Ok(())
}

/// Opens a pull request from `head` into `base` and returns its number and
/// its GraphQL node id.
fn create_pr(head: &str, base: &str, title: &str, body: &str) -> anyhow::Result<(u64, String)> {
    let pr = gh_api(
        "POST",
        &format!("repos/{REPO}/pulls"),
        Some(&json!({ "title": title, "head": head, "base": base, "body": body })),
        None,
    )?;
    let number = pr["number"].as_u64().context("no PR number")?;
    let node_id = pr["node_id"].as_str().context("no PR node id")?.to_owned();
    println!("created #{number} ({head} -> {base})");
    Ok((number, node_id))
}

/// The pull request numbers of a REST stack, bottom first.
fn stack_numbers(stack: &Value) -> Vec<u64> {
    stack["pull_requests"]
        .as_array()
        .map(|prs| prs.iter().filter_map(|pr| pr["number"].as_u64()).collect())
        .unwrap_or_default()
}

/// Links `prs` (bottom first) as a stack and checks it with a read. It
/// returns the stack number.
fn link_stack(prs: &[u64]) -> anyhow::Result<u64> {
    let created = gh_api(
        "POST",
        &format!("repos/{REPO}/stacks"),
        Some(&json!({ "pull_requests": prs })),
        Some(STACKS_API_VERSION),
    )?;
    let number = created["number"].as_u64().context("no stack number")?;
    let read = gh_api(
        "GET",
        &format!("repos/{REPO}/stacks/{number}"),
        None,
        Some(STACKS_API_VERSION),
    )?;
    let numbers = stack_numbers(&read);
    println!(
        "stack #{number}: size {}, positions {numbers:?}",
        numbers.len()
    );
    ensure!(numbers.len() == 2, "stack size: {read}");
    ensure!(numbers == prs, "stack positions: {numbers:?}");
    Ok(number)
}

/// Reads the pull request `node_id` with the GraphQL client until it shows
/// its stack and a known `mergeStateStatus`, or until 60 seconds pass.
async fn read_stacked(client: &LiveClient, node_id: &str, login: &str) -> anyhow::Result<LivePr> {
    let start = Instant::now();
    loop {
        let pr = client
            .pr_by_node_id(node_id, login)
            .await?
            .context("PR not found")?;
        println!(
            "#{}: stack {:?}/{:?}, mergeStateStatus {}, mergeable {}",
            pr.number, pr.stack_position, pr.stack_size, pr.merge_state, pr.mergeable
        );
        let known = pr.stack_size.is_some() && pr.merge_state != "Unknown";
        if known || start.elapsed() > Duration::from_secs(60) {
            return Ok(pr);
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// Merges `pr` with merge-async and a squash, and polls until the merge
/// ends. It returns the final progress.
async fn merge(client: &LiveClient, pr: &LivePr) -> anyhow::Result<LiveMergeProgress> {
    let target = (OWNER, NAME, pr.number);
    let title = format!("{} (#{})", pr.title, pr.number);
    let message = pr.body.replace("\r\n", "\n").trim_end().to_owned();
    let (uuid, mut progress) = client
        .merge_async(target, "squash", &title, &message, &pr.head_oid)
        .await?;
    println!("merge-async #{}: uuid {uuid:?}, {progress:?}", pr.number);
    let start = Instant::now();
    while progress == LiveMergeProgress::Pending {
        let Some(uuid) = uuid.as_deref() else {
            bail!("pending merge without a uuid");
        };
        ensure!(start.elapsed() < MERGE_TIMEOUT, "merge poll timed out");
        tokio::time::sleep(Duration::from_secs(2)).await;
        progress = client.merge_async_status(target, uuid).await?;
        println!("merge-async status: {progress:?}");
    }
    Ok(progress)
}

/// Prints the commits on `main` after `base`, oldest first.
fn print_new_commits(base: &str) -> anyhow::Result<()> {
    let compare = gh_api(
        "GET",
        &format!("repos/{REPO}/compare/{base}...main"),
        None,
        None,
    )?;
    let commits = compare["commits"].as_array().cloned().unwrap_or_default();
    println!("{} new commits on main:", commits.len());
    for commit in &commits {
        let sha = commit["sha"].as_str().unwrap_or_default();
        let message = commit["commit"]["message"].as_str().unwrap_or_default();
        println!("--- {sha}\n{message}\n---");
    }
    Ok(())
}

#[tokio::test]
async fn live_stack_merge_squashes_both_entries() -> anyhow::Result<()> {
    if !enabled("live_stack_merge_squashes_both_entries") {
        return Ok(());
    }
    let client = LiveClient::gh()?;
    let login = client.viewer_login().await?;
    let dir = tempfile::tempdir()?;
    let mut cleanup = Cleanup::default();

    let main_sha = checkout(dir.path())?;
    let id = suffix();
    let branch_a = format!("live-a-{id}");
    let branch_b = format!("live-b-{id}");
    cleanup.branches.push(branch_a.clone());
    cleanup.branches.push(branch_b.clone());
    commit_branch(dir.path(), &branch_a, &format!("{id}-a.txt"))?;
    commit_branch(dir.path(), &branch_b, &format!("{id}-b.txt"))?;
    git(dir.path(), &["push", "-q", "origin", &branch_a, &branch_b])?;

    let body_a = format!("Body of PR A ({id}).\n\nThe lower entry of the stack.");
    let body_b = format!("Body of PR B ({id}).\n\nThe upper entry of the stack.");
    let (number_a, _) = create_pr(&branch_a, "main", &format!("Live A {id}"), &body_a)?;
    cleanup.prs.push(number_a);
    let (number_b, node_b) = create_pr(&branch_b, &branch_a, &format!("Live B {id}"), &body_b)?;
    cleanup.prs.push(number_b);

    cleanup.stack = Some(link_stack(&[number_a, number_b])?);

    let pr_b = read_stacked(&client, &node_b, &login).await?;
    println!(
        "PR B mergeStateStatus before the merge: {}",
        pr_b.merge_state
    );
    assert_eq!(pr_b.stack_size, Some(2));
    assert_eq!(pr_b.stack_position, Some(2));
    assert_eq!(pr_b.base_ref, branch_a);

    let progress = merge(&client, &pr_b).await?;
    ensure!(
        matches!(progress, LiveMergeProgress::Merged { .. }),
        "merge of #{number_b} ended with {progress:?}"
    );

    for number in [number_a, number_b] {
        let pr = gh_api("GET", &format!("repos/{REPO}/pulls/{number}"), None, None)?;
        println!(
            "#{number}: merged {}, merge_commit_sha {}",
            pr["merged"], pr["merge_commit_sha"]
        );
        assert_eq!(pr["merged"], json!(true), "#{number} is not merged");
    }
    print_new_commits(&main_sha)?;
    drop(cleanup);
    Ok(())
}

#[tokio::test]
async fn live_viewer_login() -> anyhow::Result<()> {
    if !enabled("live_viewer_login") {
        return Ok(());
    }
    let login = LiveClient::gh()?.viewer_login().await?;
    println!("viewer login: {login}");
    assert_ne!(login, "");
    Ok(())
}
