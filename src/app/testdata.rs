//! Fixtures for the tests of `app`: pull requests and seeded states.

use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::observed::Observed;
use crate::domain::pr::{CiState, MergeStateStatus, Mergeable, PrHead, PrState, PullRequest};
use crate::domain::repo::RepoSettings;
use crate::domain::test_time;

/// A value observed at the test time.
pub(crate) fn observed<T>(value: T) -> Observed<T> {
    Observed {
        value,
        observed_at: test_time(),
    }
}

/// The key of the repo `chaosteil/<name>`.
pub(crate) fn repo_key(name: &str) -> RepoKey {
    RepoKey {
        owner: "chaosteil".into(),
        name: name.into(),
    }
}

/// The key of a pull request of `chaosteil/<repo>`.
pub(crate) fn pr_key(repo: &str, number: u64) -> PrKey {
    PrKey {
        repo: repo_key(repo),
        number,
    }
}

/// An open pull request of `chaosteil/<repo>` by `chaosteil`, observed at
/// the test time, without a stack and without checks.
pub(crate) fn pr(repo: &str, number: u64, title: &str) -> PullRequest {
    PullRequest {
        key: pr_key(repo, number),
        node_id: format!("PR_{repo}_{number}"),
        head: observed(PrHead {
            title: title.into(),
            body: String::new(),
            body_html: String::new(),
            author: "chaosteil".into(),
            state: PrState::Open,
            is_draft: false,
            head_ref: format!("chaosteil/{number}"),
            head_oid: format!("oid{number}"),
            base_ref: "main".into(),
            additions: 42,
            deletions: 7,
            changed_files: 3,
            review_decision: None,
            review_requested: false,
            reviewers: Vec::new(),
            approvals: 0,
            changes_requested: 0,
            stack: None,
            auto_merge: false,
            in_merge_queue: false,
            mergeable: Mergeable::Unknown,
            merge_state: MergeStateStatus::Unknown,
            url: format!("https://github.com/chaosteil/{repo}/pull/{number}"),
            updated_at: test_time(),
            repo_settings: RepoSettings {
                squash: true,
                merge: true,
                rebase: true,
                default_branch: None,
                squash_title_default: None,
                squash_message_default: None,
            },
        }),
        ci_state: observed(CiState::None),
        checks: observed(Vec::new()),
    }
}

/// A pull request of `chaoshub` in the stack `S_1` (#12, 3 entries) at
/// `position`.
pub(crate) fn stacked(number: u64, position: u32, title: &str) -> PullRequest {
    let mut pr = pr("chaoshub", number, title);
    pr.head.value.stack = Some(crate::domain::stack::StackRef {
        id: "S_1".into(),
        number: 12,
        size: 3,
        position,
        base_ref: "main".into(),
    });
    pr
}

/// The pull requests of the user: a 3-PR stack with mixed CI and review
/// states, and two single pull requests. Newest update first: #99, the
/// stack, #7.
pub(crate) fn mine_prs() -> Vec<PullRequest> {
    use crate::domain::pr::ReviewDecision;

    let mut base = stacked(101, 1, "feat(app): Pane trait, registry and overlay stack");
    base.ci_state.value = CiState::Failure;
    base.checks.value = checks();
    base.head.value.review_decision = Some(ReviewDecision::ChangesRequested);
    base.head.value.approvals = 1;
    base.head.value.changes_requested = 1;
    base.head.value.mergeable = Mergeable::Conflicting;
    let mut middle = stacked(102, 2, "feat(app): checks pane");
    middle.ci_state.value = CiState::Success;
    middle.head.value.review_decision = Some(ReviewDecision::Approved);
    middle.head.value.approvals = 2;
    let mut top = stacked(103, 3, "feat(ui): popup frame with pane tabs");
    top.ci_state.value = CiState::Pending;
    top.head.value.review_decision = Some(ReviewDecision::ReviewRequired);
    top.head.value.is_draft = true;
    let mut single = pr(
        "chaoshub",
        99,
        "fix(store): reopen the connection after an error",
    );
    single.head.value.updated_at += chrono::TimeDelta::hours(2);
    let mut old = pr(
        "codeview",
        7,
        "docs: describe the export file format in the README",
    );
    old.head.value.updated_at -= chrono::TimeDelta::days(3);
    old.ci_state.value = CiState::Success;
    vec![base, middle, top, single, old]
}

/// A local change of the `chaoshub` scan.
pub(crate) fn change(
    workspace: &str,
    change_id: &str,
    commit_id: &str,
    parents: &[&str],
    description: &str,
) -> crate::domain::local::LocalChange {
    crate::domain::local::LocalChange {
        workspace: workspace.into(),
        change_id: change_id.into(),
        commit_id: commit_id.into(),
        bookmarks: Vec::new(),
        empty: description.is_empty(),
        parents: parents.iter().map(|p| (*p).to_owned()).collect(),
        description: description.into(),
        vcs: crate::domain::local::Vcs::Jj,
    }
}

/// The jj scan of `chaoshub` for `mine_prs`: #103 synced, #102 diverged
/// by its bookmark, a change without a pull request on top of #103, one in
/// the `spike` workspace, and a fresh empty working copy.
pub(crate) fn mine_local() -> (RepoKey, Observed<crate::domain::local::LocalState>) {
    let mut diverged = change(
        "default",
        "xlmkrtpqxlmk",
        "9f9f9f9f9f9f",
        &["trptoqkqtrpt"],
        "feat(app): checks pane",
    );
    diverged.bookmarks = vec!["chaosteil/102".into()];
    let changes = vec![
        change("default", "kkkkllllmmmm", "1234abcd", &["zzmqxkrtzzmq"], ""),
        change(
            "default",
            "zzmqxkrtzzmq",
            "5678ef01",
            &["wqnvzxopwqnv"],
            "wip: try the new layout",
        ),
        change(
            "default",
            "wqnvzxopwqnv",
            "oid103",
            &["xlmkrtpqxlmk"],
            "feat(ui): popup frame with pane tabs",
        ),
        diverged,
        change(
            "spike",
            "ppqqrrssttuu",
            "aa55aa55",
            &["trunk"],
            "spike: a faster sync",
        ),
    ];
    (
        repo_key("chaoshub"),
        observed(crate::domain::local::LocalState {
            workspaces: Vec::new(),
            changes,
        }),
    )
}

/// Three checks: a success, a failure and a running check.
pub(crate) fn checks() -> Vec<crate::domain::check::Check> {
    use crate::domain::check::{Check, CheckConclusion, CheckKind, CheckStatus};

    let check = |name: &str, status, conclusion, secs: i64| Check {
        name: name.into(),
        kind: CheckKind::CheckRun,
        status,
        conclusion,
        started: Some(test_time()),
        completed: (status == CheckStatus::Completed)
            .then(|| test_time() + chrono::TimeDelta::seconds(secs)),
        url: Some(format!(
            "https://github.com/chaosteil/chaoshub/actions/runs/{secs}"
        )),
        run_id: Some(1),
        suite_id: Some(2),
    };
    vec![
        check(
            "ci / build",
            CheckStatus::Completed,
            Some(CheckConclusion::Success),
            130,
        ),
        check(
            "ci / test",
            CheckStatus::Completed,
            Some(CheckConclusion::Failure),
            242,
        ),
        check("ci / lint", CheckStatus::InProgress, None, 0),
    ]
}

/// A startup snapshot with the pull requests of `mine_prs` and the jj
/// scan of `mine_local`.
pub(crate) fn mine_snapshot() -> crate::store::Snapshot {
    crate::store::Snapshot {
        prs: mine_prs()
            .into_iter()
            .map(|pr| (crate::bus::Scope::Mine, pr))
            .collect(),
        synced_at: Some(test_time()),
        local: vec![mine_local()],
        ..crate::store::Snapshot::default()
    }
}

/// The pull requests of colleagues: three ready ones (#88 requests the
/// review of the user), a draft (#85, requested) and one with failing CI
/// (#84). Ages at `mine_snapshot` time plus 5 minutes.
pub(crate) fn team_prs() -> Vec<PullRequest> {
    use crate::domain::pr::ReviewDecision;

    let colleague = |number: u64, title: &str, author: &str, hours: i64| {
        let mut pr = pr("chaoshub", number, title);
        pr.head.value.author = author.into();
        pr.head.value.updated_at = test_time() - chrono::TimeDelta::hours(hours);
        pr.ci_state.value = CiState::Success;
        pr.head.value.review_decision = Some(ReviewDecision::ReviewRequired);
        pr
    };
    let mut requested = colleague(88, "docs(readme): keymap table", "alice", 2);
    requested.head.value.review_requested = true;
    let mut draft = colleague(85, "wip(sync): adaptive page size", "carol", 1);
    draft.head.value.is_draft = true;
    draft.head.value.review_requested = true;
    let mut failing = colleague(84, "fix(vcs): parse empty bookmarks", "dave", 30);
    failing.ci_state.value = CiState::Failure;
    let mut other = colleague(86, "perf(ui): cache the texture", "bob", 50);
    other.key.repo = repo_key("codeview");
    other.head.value.review_decision = Some(ReviewDecision::Approved);
    other.head.value.approvals = 1;
    vec![
        requested,
        colleague(87, "perf(store): batch inserts", "bob", 3),
        other,
        draft,
        failing,
    ]
}

/// A startup snapshot with `mine_snapshot` and the `team_prs`.
pub(crate) fn team_snapshot() -> crate::store::Snapshot {
    let mut snapshot = mine_snapshot();
    snapshot.prs.extend(
        team_prs()
            .into_iter()
            .map(|pr| (crate::bus::Scope::Team, pr)),
    );
    snapshot
}

/// A conversation at head `oid7`: an issue comment (at the test time), a
/// resolved thread of two comments, an open thread, a review that asks for
/// changes (2 hours later), and a pending review.
pub(crate) fn detail() -> crate::domain::detail::PrDetail {
    use crate::domain::detail::{Comment, PrDetail, Review, ReviewState, Thread};

    let comment = |author: &str, body: &str, minutes: i64| Comment {
        author: author.into(),
        body: body.into(),
        body_html: String::new(),
        created_at: test_time() + chrono::TimeDelta::minutes(minutes),
        url: format!("https://github.com/c/{minutes}"),
    };
    PrDetail {
        comments: vec![comment(
            "alice",
            "Looks good, one question about `layout`.\r\n",
            0,
        )],
        reviews: vec![
            Review {
                author: "bob".into(),
                state: ReviewState::ChangesRequested,
                body: "Please split the **transition** module.".into(),
                body_html: String::new(),
                submitted_at: Some(test_time() + chrono::TimeDelta::hours(2)),
            },
            Review {
                author: "carol".into(),
                state: ReviewState::Pending,
                body: String::new(),
                body_html: String::new(),
                submitted_at: None,
            },
        ],
        threads: vec![
            Thread {
                path: "src/ui/list.rs".into(),
                line: Some(42),
                is_resolved: true,
                comments: vec![
                    comment("bob", "Off by one?", 10),
                    comment("chaosteil", "Fixed.", 20),
                ],
            },
            Thread {
                path: "src/ui/pane.rs".into(),
                line: Some(7),
                is_resolved: false,
                comments: vec![comment("bob", "The gutter eats one column.", 60)],
            },
        ],
        head_oid: "oid7".into(),
    }
}

/// The changed files at head `oid7`: an added file, a removed file, a
/// modified file in a nested directory, a renamed file and a binary file
/// without a patch.
pub(crate) fn files() -> crate::domain::files::PrFiles {
    use crate::domain::files::{ChangedFile, FileStatus, PrFiles};

    let file = |path: &str, status, additions, deletions, patch: Option<&str>| ChangedFile {
        path: path.into(),
        previous_path: None,
        status,
        additions,
        deletions,
        patch: patch.map(Into::into),
    };
    let mut renamed = file(
        "src/main.rs",
        FileStatus::Renamed,
        1,
        1,
        Some("@@ -1 +1 @@\n-fn old() {}\n+fn new() {}"),
    );
    renamed.previous_path = Some("src/start.rs".into());
    PrFiles {
        files: vec![
            file(
                "docs/usage.md",
                FileStatus::Added,
                3,
                0,
                Some("@@ -0,0 +1,3 @@\n+# Usage\n+\n+Run the tool."),
            ),
            file(
                "old.txt",
                FileStatus::Removed,
                0,
                2,
                Some("@@ -1,2 +0,0 @@\n-first\n-second"),
            ),
            file(
                "src/app/panes/files.rs",
                FileStatus::Modified,
                2,
                1,
                Some(
                    "@@ -1,3 +1,4 @@\n fn main() {\n-    println!(\"old\");\n+    println!(\"new\");\n+    println!(\"more\");\n }\n\\ No newline at end of file",
                ),
            ),
            renamed,
            file("assets/logo.png", FileStatus::Modified, 0, 0, None),
        ],
        truncated: false,
        head_oid: "oid7".into(),
    }
}
