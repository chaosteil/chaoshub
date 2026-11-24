//! The screenshots of the README: the list, the PR view and the stack
//! merge popup. The data is made up. No GitHub account is used.
//!
//! The test is ignored, so the usual test run writes no files. To write
//! the images again to `docs/screenshots/`, run (see "Screenshots" in
//! `docs/development.md`):
//!
//! ```sh
//! cargo test --locked readme_screenshots -- --ignored
//! ```

mod svg;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::app::App;
use crate::app::keymap::Keymap;
use crate::app::testdata::{change, observed, pr, pr_key, repo_key};
use crate::bus::{Msg, Scope, Size};
use crate::domain::check::{Check, CheckConclusion, CheckKind, CheckStatus};
use crate::domain::detail::{
    Comment, PrDetail, Reaction, ReactionKind, Review, ReviewState, Thread,
};
use crate::domain::local::LocalState;
use crate::domain::pr::{CiState, MergeStateStatus, Mergeable, PullRequest, ReviewDecision};
use crate::domain::repo::{Repo, SquashMessageDefault, SquashTitleDefault};
use crate::domain::stack::{OpenEntry, StackRef};
use crate::domain::test_time;
use crate::store::Snapshot;
use crate::theme::{Theme, ThemeKind};
use crate::ui::hits::HitMap;
use crate::ui::transition::Transitions;

/// The terminal width of each screenshot.
const WIDTH: u16 = 120;
/// The terminal height of the list screenshot. The list is short, so a
/// lower terminal has less empty space.
const LIST_HEIGHT: u16 = 22;
/// The terminal height of the popup screenshots. The merge popup then
/// comes below the stack, so both show.
const POPUP_HEIGHT: u16 = 30;

/// The stack of `chaoshub`: 3 pull requests on `main`.
const STACK: [(u64, &str); 3] = [
    (141, "feat(sync): read PRs in pages"),
    (142, "feat(sync): retry rate limits"),
    (143, "feat(ui): sync progress bar"),
];

/// The description of #142.
const BODY: &str = "\
## Why

A large repo can hit the rate limit of GitHub in the middle of a sync. \
The sync then stopped, and the list showed old data.

## What

- The sync waits until the limit resets, then reads the page again.
- The status bar shows `rate limited, retry in 42s`.
- A second failure stops the sync with an error.

## Testing

- New `wiremock` tests for the 403 and the 429 replies.
- `scripts/check.sh` passes.
";

/// Writes the three screenshots to `docs/screenshots/`.
#[test]
#[ignore = "writes the README screenshots; run it with --ignored"]
fn readme_screenshots() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/screenshots");
    std::fs::create_dir_all(&dir).unwrap();
    let shots = [
        ("list.svg", list_app()),
        ("pr-view.svg", pr_view_app()),
        ("stack-merge.svg", stack_merge_app()),
    ];
    for (name, mut app) in shots {
        std::fs::write(dir.join(name), shot(&mut app)).unwrap();
    }
}

/// The list on the Mine tab, with the cursor on the stack.
fn list_app() -> App {
    let mut app = app(LIST_HEIGHT);
    select(&mut app, 143);
    app
}

/// The PR view of #142 on the Description pane.
fn pr_view_app() -> App {
    let mut app = app(POPUP_HEIGHT);
    select(&mut app, 142);
    key(&mut app, KeyCode::Enter);
    let observed = crate::domain::observed::Observed {
        value: detail(),
        observed_at: test_time(),
    };
    app.update(Msg::DetailLoaded(pr_key("chaoshub", 142), Ok(observed)));
    app.update(Msg::TransitionsDone);
    app
}

/// The merge popup of the stack on #143.
fn stack_merge_app() -> App {
    let mut app = app(POPUP_HEIGHT);
    select(&mut app, 143);
    key(&mut app, KeyCode::Char(' '));
    app.update(Msg::TransitionsDone);
    key(&mut app, KeyCode::Char('s'));
    app.update(Msg::TransitionsDone);
    app
}

/// The app with the data of `snapshot` in a terminal of `height` rows,
/// 5 minutes after the last sync.
fn app(height: u16) -> App {
    let theme = theme();
    let repos = ["chaoshub", "codeview", "dotfiles"]
        .into_iter()
        .map(|name| Repo {
            key: repo_key(name),
            path: None,
            settings: None,
        })
        .collect();
    App::new(
        Size {
            width: WIDTH,
            height,
        },
        Keymap::default(),
        true,
        snapshot(),
        repos,
        test_time() + chrono::TimeDelta::minutes(5),
    )
    .with_selection_shift(theme.selected_row().shift)
}

/// The default theme on a true-color terminal.
fn theme() -> Theme {
    Theme::new(ThemeKind::TrueColor, true, true)
}

/// Draws the app once without transitions and returns the SVG image.
fn shot(app: &mut App) -> String {
    let theme = theme();
    let p = &theme.palette;
    let colors = svg::Colors {
        fg: p.fg,
        bg: p.bg,
        bar: p.surface,
    };
    svg::render(&draw(app), "chaoshub", &colors)
}

/// Draws the app once without transitions and returns the buffer.
fn draw(app: &mut App) -> Buffer {
    let size = app.size();
    let mut terminal = Terminal::new(TestBackend::new(size.width, size.height)).unwrap();
    let theme = theme();
    let mut transitions = Transitions::new(false, 160).with_theme(theme);
    super::draw(
        &mut terminal,
        app,
        &mut transitions,
        &theme,
        Duration::ZERO,
        &mut HitMap::default(),
        None,
        &mut String::new(),
    )
    .unwrap();
    terminal.backend().buffer().clone()
}

/// Moves the cursor down to the row of the pull request `number`.
fn select(app: &mut App, number: u64) {
    let tag = format!("#{number} ");
    for _ in 0..20 {
        let buf = draw(app);
        let selected = (0..buf.area.height).any(|y| {
            let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            row.contains('▶') && row.contains(&tag)
        });
        if selected {
            return;
        }
        key(app, KeyCode::Char('j'));
    }
    panic!("no row for #{number}");
}

fn key(app: &mut App, code: KeyCode) {
    app.update(Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

/// The cache: the pull requests of `prs` and the jj scan of `chaoshub`.
fn snapshot() -> Snapshot {
    Snapshot {
        prs: prs().into_iter().map(|pr| (Scope::Mine, pr)).collect(),
        synced_at: Some(test_time()),
        local: vec![(repo_key("chaoshub"), observed(local()))],
        ..Snapshot::default()
    }
}

/// The pull requests of the user in three repos: the stack of `STACK`,
/// four single pull requests in `chaoshub`, three in `codeview` and one
/// in `dotfiles`.
fn prs() -> Vec<PullRequest> {
    let mut prs = stack();
    let mut store = mine(
        "chaoshub",
        138,
        "fix(store): reopen the connection after an error",
        26,
    );
    store.ci_state.value = CiState::Failure;
    store.checks.value = checks(Some(CheckStatus::Completed));
    store.head.value.review_decision = Some(ReviewDecision::ChangesRequested);
    store.head.value.changes_requested = 1;
    store.head.value.approvals = 1;
    let mut docs = mine(
        "chaoshub",
        136,
        "docs(readme): describe the jj and git modes",
        50,
    );
    approve(&mut docs, 1);
    let mut export = mine(
        "codeview",
        57,
        "feat(export): write the comments as JSON lines",
        3,
    );
    export.head.value.review_decision = Some(ReviewDecision::ReviewRequired);
    let mut gutter = mine(
        "codeview",
        55,
        "wip(ui): a gutter for the comment marks",
        72,
    );
    gutter.head.value.is_draft = true;
    gutter.ci_state.value = CiState::None;
    gutter.checks.value = Vec::new();
    gutter.head.value.mergeable = Mergeable::Unknown;
    let mut pins = mine("dotfiles", 12, "chore(nvim): pin the plugin versions", 8);
    pins.head.value.review_decision = None;
    let mut review = mine(
        "chaoshub",
        145,
        "feat(review): send a pending review from the popup",
        6,
    );
    approve(&mut review, 2);
    let mut fold = mine(
        "codeview",
        58,
        "fix(diff): keep the cursor on a folded hunk",
        5,
    );
    fold.ci_state.value = CiState::Failure;
    fold.checks.value = checks(Some(CheckStatus::Completed));
    fold.head.value.review_decision = Some(ReviewDecision::ReviewRequired);
    let mut rows = mine(
        "chaoshub",
        134,
        "perf(ui): draw only the rows that changed",
        30,
    );
    rows.ci_state.value = CiState::Pending;
    rows.checks.value = checks(Some(CheckStatus::InProgress));
    rows.head.value.review_decision = Some(ReviewDecision::ReviewRequired);
    prs.extend([store, docs, export, gutter, pins, review, fold, rows]);
    prs
}

/// The 3 pull requests of `STACK`, bottom first. #141 and #142 pass CI
/// and are approved. CI runs on #143.
fn stack() -> Vec<PullRequest> {
    let mut prs = Vec::new();
    for (i, (number, title)) in STACK.into_iter().enumerate() {
        let position = u32::try_from(i).unwrap() + 1;
        let mut pr = mine("chaoshub", number, title, 20 - i64::from(position));
        pr.head.value.stack = Some(StackRef {
            id: "S_sync".into(),
            number: 7,
            size: 3,
            position,
            base_ref: "main".into(),
            merged: Vec::new(),
            open: STACK
                .iter()
                .zip(1..)
                .map(|((number, _), position)| OpenEntry {
                    position,
                    number: *number,
                    node_id: format!("PR_chaoshub_{number}"),
                })
                .collect(),
        });
        prs.push(pr);
    }
    let [base, middle, top] = &mut prs[..] else {
        unreachable!("the stack has 3 entries");
    };
    approve(base, 2);
    base.head.value.additions = 214;
    base.head.value.deletions = 61;
    approve(middle, 1);
    middle.head.value.body = BODY.into();
    middle.head.value.additions = 138;
    middle.head.value.deletions = 12;
    middle.head.value.changed_files = 5;
    middle.checks.value = checks(None);
    approve(top, 1);
    top.ci_state.value = CiState::Pending;
    top.checks.value = checks(Some(CheckStatus::InProgress));
    prs
}

/// An open pull request of the user, updated `hours` before the sync,
/// with a passing CI and a clean merge state. The repo squashes with the
/// title and the body of the pull request, so a stack merges at once.
fn mine(repo: &str, number: u64, title: &str, hours: i64) -> PullRequest {
    let mut pr = pr(repo, number, title);
    let head = &mut pr.head.value;
    head.repo_settings.squash_title_default = Some(SquashTitleDefault::PrTitle);
    head.repo_settings.squash_message_default = Some(SquashMessageDefault::PrBody);
    head.head_ref = format!("chaosteil/push-{number}");
    head.head_oid = commit(number);
    head.updated_at = test_time() - chrono::TimeDelta::hours(hours);
    head.created_at = Some(head.updated_at - chrono::TimeDelta::days(2));
    head.mergeable = Mergeable::Mergeable;
    head.merge_state = MergeStateStatus::Clean;
    pr.ci_state.value = CiState::Success;
    pr.checks.value = checks(None);
    pr
}

/// Sets an approval of `count` reviewers.
fn approve(pr: &mut PullRequest, count: u32) {
    pr.head.value.review_decision = Some(ReviewDecision::Approved);
    pr.head.value.approvals = count;
}

/// The head commit of the pull request `number`.
fn commit(number: u64) -> String {
    format!("{:08x}", number.wrapping_mul(0x9e37_79b1) & 0xffff_ffff)
}

/// The checks of CI: build and lint pass, and the test job has `test`
/// (`None` for a pass, a completed status for a failure).
fn checks(test: Option<CheckStatus>) -> Vec<Check> {
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
    let test = match test {
        None => check(
            "ci / test (ubuntu)",
            CheckStatus::Completed,
            Some(CheckConclusion::Success),
            251,
        ),
        Some(CheckStatus::Completed) => check(
            "ci / test (ubuntu)",
            CheckStatus::Completed,
            Some(CheckConclusion::Failure),
            242,
        ),
        Some(status) => check("ci / test (ubuntu)", status, None, 0),
    };
    vec![
        check(
            "ci / build",
            CheckStatus::Completed,
            Some(CheckConclusion::Success),
            131,
        ),
        test,
        check(
            "ci / test (macos)",
            CheckStatus::Completed,
            Some(CheckConclusion::Success),
            318,
        ),
        check(
            "ci / lint",
            CheckStatus::Completed,
            Some(CheckConclusion::Success),
            64,
        ),
    ]
}

/// The jj scan of `chaoshub`: the stack in the `default` workspace, a
/// fresh working copy on top of the stack, and #138 and #145 in their
/// own workspaces.
fn local() -> LocalState {
    let changes = vec![
        change("default", "kpqwmzrtkpqw", "77aa01bc", &["ysxnrvloysxn"], ""),
        change(
            "default",
            "ysxnrvloysxn",
            &commit(143),
            &["tqvmlkzwtqvm"],
            STACK[2].1,
        ),
        change(
            "default",
            "tqvmlkzwtqvm",
            &commit(142),
            &["rnwpxyuqrnwp"],
            STACK[1].1,
        ),
        change(
            "default",
            "rnwpxyuqrnwp",
            &commit(141),
            &["trunk"],
            STACK[0].1,
        ),
        change(
            "store",
            "mvzlwxtpmvzl",
            &commit(138),
            &["trunk"],
            "fix(store): reopen the connection after an error",
        ),
        change(
            "review",
            "olqzvsnkolqz",
            &commit(145),
            &["trunk"],
            "feat(review): send a pending review from the popup",
        ),
    ];
    LocalState {
        workspaces: Vec::new(),
        changes,
    }
}

/// The conversation of #142: the reactions on the pull request, a
/// comment of alice, a resolved thread and the approval of bob.
fn detail() -> PrDetail {
    let reaction = |kind, count, mine, logins: &[&str]| Reaction {
        kind,
        count,
        mine,
        logins: logins.iter().map(|&login| login.to_owned()).collect(),
    };
    let comment = |author: &str, body: &str, minutes: i64| Comment {
        id: format!("IC_{minutes}"),
        author: author.into(),
        body: body.into(),
        body_html: String::new(),
        created_at: test_time() - chrono::TimeDelta::minutes(minutes),
        url: format!("https://github.com/chaosteil/chaoshub/pull/142#{minutes}"),
        reactions: Vec::new(),
    };
    PrDetail {
        comments: vec![comment(
            "alice",
            "Nice, this fixes the stuck sync that I saw on Monday.",
            90,
        )],
        reviews: vec![Review {
            id: "PRR_bob".into(),
            author: "bob".into(),
            state: ReviewState::Approved,
            body: "Looks good to me.".into(),
            body_html: String::new(),
            submitted_at: Some(test_time() - chrono::TimeDelta::minutes(30)),
            reactions: Vec::new(),
        }],
        threads: vec![Thread {
            id: "PRRT_1".into(),
            path: "src/workers/sync.rs".into(),
            line: Some(212),
            is_resolved: true,
            comments: vec![
                comment("bob", "Can the wait use the reset header?", 80),
                comment("chaosteil", "Done, it reads `x-ratelimit-reset` now.", 60),
            ],
        }],
        reactions: vec![
            reaction(ReactionKind::ThumbsUp, 3, true, &["bob", "chaosteil"]),
            reaction(ReactionKind::Rocket, 2, false, &["alice", "carol"]),
        ],
        head_oid: commit(142),
    }
}
