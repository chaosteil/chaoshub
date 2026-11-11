//! The description pane of the pull request popup. It is the first pane,
//! so the popup opens on it. It starts with a readiness block of three
//! lines: the merge verdict and its reasons; the checks, the reviews, the
//! open threads and the size; and the latest activity. A blank line and
//! the body of the pull request as markdown come after the block.
//!
//! While a merge of the pull request runs, the verdict is the progress of
//! the merge: a spinner and `MERGING` on a badge that pulses between the
//! success green and the merge purple. A failed merge shows a red
//! `MERGE FAILED` badge with the reason, and a merge into the merge queue
//! shows `IN QUEUE`.
//!
//! The reviews come from the pull request, as in the REV column of the
//! list. The threads and the activity come from the conversation detail,
//! which loads in the background. The pane does not start a load.
//!
//! When the head has codeview comments that went to GitHub not yet, one
//! dim line between the block and the body counts them, for example
//! `3 unsent codeview comments · space r to review`. The Review popup
//! lists and sends them. Without such comments, the pane shows no line.

use chrono::{DateTime, TimeDelta, Utc};

use crate::app::age::age;
use crate::app::frame::spinner;
use crate::app::keymap::{Keymap, chord_text};
use crate::app::models::{Cell, CellStyle, PaneModel, Tone};
use crate::app::panes::agent::head;
use crate::app::panes::checks::failed;
use crate::app::panes::{Pane, PaneState};
use crate::app::state::{AppData, MergeState};
use crate::app::tabs::cells::{self, cell};
use crate::app::tabs::{Handled, KeyCx};
use crate::bus::{KeyChord, MergeProgress, MergeStep, Size, Target};
use crate::domain::check::{Check, CheckConclusion, CheckStatus};
use crate::domain::detail::{PrDetail, ReviewState};
use crate::domain::pr::{
    CiState, MergeStateStatus, Mergeable, PrHead, PrState, PullRequest, ReviewDecision,
};

/// The description pane.
pub(crate) struct DescriptionPane;

impl Pane for DescriptionPane {
    fn id(&self) -> &'static str {
        "description"
    }

    fn title(&self) -> &'static str {
        "Description"
    }

    fn short_title(&self) -> &'static str {
        "Desc"
    }

    fn key(&self) -> char {
        '1'
    }

    fn applies(&self, target: &Target) -> bool {
        matches!(target, Target::Pr { .. })
    }

    fn init_state(&self) -> PaneState {
        PaneState::default()
    }

    fn model(&self, cx: &KeyCx<'_>, _ps: &PaneState, _size: Size) -> PaneModel {
        let Some(Target::Pr { pr, .. }) = cx.target else {
            return PaneModel::Lines {
                lines: Vec::new(),
                wrap: true,
            };
        };
        let head = &pr.head.value;
        let html = head.body_html.trim();
        let body = head.body.replace("\r\n", "\n");
        let body = body.trim();
        let line = if !html.is_empty() {
            vec![cell(html, CellStyle::Html)]
        } else if body.is_empty() {
            vec![cell("(no description)", CellStyle::Dim)]
        } else {
            vec![cell(body, CellStyle::Markdown)]
        };
        let mut lines = readiness(cx, pr);
        if let Some(unsent) = codeview_line(cx, pr) {
            lines.push(Vec::new());
            lines.push(unsent);
        }
        lines.push(Vec::new());
        lines.push(line);
        PaneModel::Lines { lines, wrap: true }
    }

    fn handle_key(&self, _key: KeyChord, _cx: &KeyCx<'_>, _ps: &mut PaneState) -> Option<Handled> {
        None
    }
}

/// The dim line that counts the unsent codeview comments of the head of
/// `pr`, with the keys of the Review popup. `None` without such comments,
/// before the load and after a failed load.
fn codeview_line(cx: &KeyCx<'_>, pr: &PullRequest) -> Option<Vec<Cell>> {
    let Some(Ok(comments)) = cx.data.codeview.get(&pr.key) else {
        return None;
    };
    let count = comments.current(&head(cx.data, pr)).len();
    if count == 0 {
        return None;
    }
    let noun = if count == 1 { "comment" } else { "comments" };
    let text = match review_keys(cx.keys) {
        Some(keys) => format!("{count} unsent codeview {noun} · {keys} to review"),
        None => format!("{count} unsent codeview {noun}"),
    };
    Some(vec![cell(text, CellStyle::Dim)])
}

/// The keys that open the Review popup, for example `space r`. `None`
/// when the actions menu or the review action has no key.
fn review_keys(keys: &Keymap) -> Option<String> {
    let menu = keys.chords("actions").first()?;
    let review = keys.chords("codeview_send").first()?;
    Some(format!("{} {}", chord_text(*menu), chord_text(*review)))
}

/// The three lines of the readiness block: the verdict, the signals and
/// the latest activity.
fn readiness(cx: &KeyCx<'_>, pr: &PullRequest) -> Vec<Vec<Cell>> {
    let head = &pr.head.value;
    let checks = pr.checks.value.as_slice();
    // A stale detail still shows: it is the best data until the new load.
    let detail = cx.data.details.get(&pr.key).map(|d| &d.value);
    let error = cx.data.detail_errors.contains_key(&pr.key);
    vec![
        merge_verdict(cx.data, pr).unwrap_or_else(|| verdict(head, pr.ci_state.value, checks)),
        signals(head, checks, detail),
        activity(head, detail, error, cx.data.now),
    ]
}

/// The badge text and the tone of the verdict: the first row of the
/// verdict table that matches.
fn badge(head: &PrHead, ci: CiState) -> (&'static str, Tone) {
    if head.state == PrState::Merged {
        ("MERGED", Tone::Merge)
    } else if head.state == PrState::Closed {
        ("CLOSED", Tone::Dim)
    } else if head.is_draft {
        ("DRAFT", Tone::Dim)
    } else if head.in_merge_queue {
        ("IN QUEUE", Tone::Merge)
    } else if conflicts(head) {
        ("CONFLICTS", Tone::Danger)
    } else if ci == CiState::Failure
        || head.review_decision == Some(ReviewDecision::ChangesRequested)
    {
        ("BLOCKED", Tone::Danger)
    } else if head.review_decision == Some(ReviewDecision::ReviewRequired)
        || matches!(
            head.merge_state,
            MergeStateStatus::Behind | MergeStateStatus::Blocked
        )
    {
        ("BLOCKED", Tone::Warn)
    } else if ci == CiState::Pending {
        ("WAITING", Tone::Warn)
    } else if head.mergeable == Mergeable::Unknown || head.merge_state == MergeStateStatus::Unknown
    {
        ("CHECKING", Tone::Dim)
    } else {
        ("READY", Tone::Success)
    }
}

/// True when the pull request has merge conflicts.
fn conflicts(head: &PrHead) -> bool {
    head.mergeable == Mergeable::Conflicting || head.merge_state == MergeStateStatus::Dirty
}

/// The text style of a tone.
fn tone_style(tone: Tone) -> CellStyle {
    match tone {
        Tone::Success => CellStyle::Success,
        Tone::Danger => CellStyle::Danger,
        Tone::Warn => CellStyle::Warn,
        Tone::Merge => CellStyle::Merge,
        Tone::Dim => CellStyle::Dim,
    }
}

/// `1 check` or `N checks`.
fn checks_noun(count: usize) -> String {
    if count == 1 {
        "1 check".to_owned()
    } else {
        format!("{count} checks")
    }
}

/// The number of checks with a failing conclusion.
fn failed_count(checks: &[Check]) -> usize {
    checks
        .iter()
        .filter(|c| c.conclusion.is_some_and(failed))
        .count()
}

/// The number of checks that did not complete.
fn running_count(checks: &[Check]) -> usize {
    checks
        .iter()
        .filter(|c| c.status != CheckStatus::Completed)
        .count()
}

/// The reasons of the verdict, in display order, each in its style.
fn reasons(head: &PrHead, checks: &[Check]) -> Vec<Cell> {
    let mut out = Vec::new();
    let failed = failed_count(checks);
    if failed > 0 {
        out.push(cell(
            format!("{} failed", checks_noun(failed)),
            CellStyle::Danger,
        ));
    }
    let running = running_count(checks);
    if running > 0 {
        out.push(cell(
            format!("{} running", checks_noun(running)),
            CellStyle::Warn,
        ));
    }
    match head.review_decision {
        Some(ReviewDecision::ChangesRequested) => {
            out.push(cell("changes requested", CellStyle::Danger));
        }
        Some(ReviewDecision::ReviewRequired) => {
            out.push(cell("review required", CellStyle::Warn));
        }
        Some(ReviewDecision::Approved) | None => {}
    }
    if conflicts(head) {
        out.push(cell("conflicts", CellStyle::Danger));
    }
    if head.merge_state == MergeStateStatus::Behind {
        out.push(cell(format!("behind {}", head.base_ref), CellStyle::Warn));
    }
    if head.auto_merge {
        out.push(cell("auto-merge on", CellStyle::Dim));
    }
    out
}

/// Line 1: the badge, then ` ╱ ` and the reasons. Without reasons, the
/// badge stands alone.
fn verdict(head: &PrHead, ci: CiState, checks: &[Check]) -> Vec<Cell> {
    let (text, tone) = badge(head, ci);
    let mut out = vec![cell(format!(" {text} "), CellStyle::Badge(tone))];
    let mut reasons = reasons(head, checks);
    if reasons.is_empty() && text == "READY" {
        reasons.push(cell("ready to merge", CellStyle::Normal));
    }
    if reasons.is_empty() {
        return out;
    }
    out.push(cell(" ╱ ", tone_style(tone)));
    for (i, reason) in reasons.into_iter().enumerate() {
        if i > 0 {
            out.push(cell(" · ", CellStyle::Normal));
        }
        out.push(reason);
    }
    out
}

/// Line 1 while a merge run of `pr` goes on, after it failed, or after it
/// put `pr` in the merge queue. `None` without a run of `pr`, and when
/// `pr` is no longer open: line 1 is then the verdict, for example
/// MERGED.
fn merge_verdict(data: &AppData, pr: &PullRequest) -> Option<Vec<Cell>> {
    let merge = data.merge.as_ref().filter(|merge| merge.key == pr.key)?;
    if pr.head.value.state != PrState::Open {
        return None;
    }
    let line = match &merge.progress {
        MergeProgress::Failed(reason) => vec![
            cell(" MERGE FAILED ", CellStyle::Badge(Tone::Danger)),
            cell(" ╱ ", CellStyle::Danger),
            cell(reason.as_str(), CellStyle::Danger),
        ],
        MergeProgress::Enqueued => vec![cell(" IN QUEUE ", CellStyle::Badge(Tone::Merge))],
        MergeProgress::Pending | MergeProgress::Merged { .. } => {
            let text = format!(" {} {} ", spinner(data.now), merging_text(merge));
            let mix = pulse(data.now - merge.started, data.animations);
            vec![cell(text, CellStyle::MergeBadge(mix))]
        }
    };
    Some(line)
}

/// The badge text of a merge run: `MERGING`, `MERGING 2/3 #12` for an
/// entry of a stack, `MERGING · waiting for #13 to retarget` between two
/// entries, and `MERGING · fetching` after GitHub merged the last entry,
/// until the refreshed pull request shows the merge.
fn merging_text(merge: &MergeState) -> String {
    let merged = matches!(merge.progress, MergeProgress::Merged { .. });
    match merge.step {
        MergeStep::Retarget { number } => format!("MERGING · waiting for #{number} to retarget"),
        MergeStep::Entry { index, total, .. } if merged && index == total => {
            "MERGING · fetching".to_owned()
        }
        MergeStep::Entry { total: 1, .. } => "MERGING".to_owned(),
        MergeStep::Entry {
            number,
            index,
            total,
        } => format!("MERGING {index}/{total} #{number}"),
    }
}

/// The time of one pulse of the merge badge: from green to purple and back
/// to green.
const PULSE: TimeDelta = TimeDelta::milliseconds(1500);

/// The color mix of the merge badge `elapsed` after the start of the run:
/// 0 is the success green, 255 the merge purple. The mix follows a cosine,
/// so it starts on green and slows down at both colors. With `animations`
/// off, it stays on green.
fn pulse(elapsed: TimeDelta, animations: bool) -> u8 {
    if !animations {
        return 0;
    }
    let period = PULSE.num_milliseconds();
    let phase = elapsed.num_milliseconds().rem_euclid(period);
    let phase = f64::from(u32::try_from(phase).unwrap_or(0))
        / f64::from(u32::try_from(period).unwrap_or(1));
    let mix = (1.0 - (std::f64::consts::TAU * phase).cos()) / 2.0 * 255.0;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "mix is rounded and between 0 and 255"
    )]
    let mix = mix.round() as u8;
    mix
}

/// The space between two groups of the signals line.
const GROUP_GAP: &str = "   ";

/// Line 2: the checks, the reviews, the threads and the size.
fn signals(head: &PrHead, checks: &[Check], detail: Option<&PrDetail>) -> Vec<Cell> {
    let groups = [
        check_signal(checks),
        review_signal(head),
        thread_signal(detail),
        size(head),
    ];
    let mut out = Vec::new();
    for (i, group) in groups.into_iter().enumerate() {
        if i > 0 {
            out.push(cell(GROUP_GAP, CellStyle::Normal));
        }
        out.extend(group);
    }
    out
}

/// `CHECKS ✓2 ✗1 ●1`, without the counts of 0. `CHECKS none` when the
/// pull request has no checks.
fn check_signal(checks: &[Check]) -> Vec<Cell> {
    let mut out = vec![cell("CHECKS", CellStyle::Dim)];
    if checks.is_empty() {
        out.push(cell(" none", CellStyle::Dim));
        return out;
    }
    let passed = checks
        .iter()
        .filter(|c| c.conclusion == Some(CheckConclusion::Success))
        .count();
    let counts = [
        ("✓", passed, CellStyle::Success),
        ("✗", failed_count(checks), CellStyle::Danger),
        ("●", running_count(checks), CellStyle::Warn),
    ];
    for (glyph, count, style) in counts {
        if count > 0 {
            out.push(cell(format!(" {glyph}{count}"), style));
        }
    }
    out
}

/// `REVIEWS ✓1 ✗1 · @bob requested`: the counts of the REV column of the
/// list, then the requested reviewers. The counts are the latest
/// opinionated review of each author on GitHub, also of the user. Without
/// counts, the glyph of the review decision shows, as in the list.
/// `REVIEWS none` without counts, decision and requested reviewers.
fn review_signal(head: &PrHead) -> Vec<Cell> {
    let mut out = vec![cell("REVIEWS", CellStyle::Dim)];
    let counts = [
        ("✓", head.approvals, CellStyle::Success),
        ("✗", head.changes_requested, CellStyle::Danger),
    ];
    for (glyph, count, style) in counts {
        if count > 0 {
            out.push(cell(format!(" {glyph}{count}"), style));
        }
    }
    if out.len() == 1 && head.review_decision.is_some() {
        let decision = cells::review(head.review_decision);
        out.push(cell(format!(" {}", decision.text), decision.style));
    }
    let counted = out.len() > 1;
    if !head.reviewers.is_empty() {
        let logins: Vec<String> = head.reviewers.iter().map(|r| format!("@{r}")).collect();
        let lead = if counted { " · " } else { " " };
        out.push(cell(
            format!("{lead}{} requested", logins.join(" ")),
            CellStyle::Dim,
        ));
    } else if !counted {
        out.push(cell(" none", CellStyle::Dim));
    }
    out
}

/// `THREADS 2 open`: the unresolved threads. `THREADS …` before the
/// detail loads.
fn thread_signal(detail: Option<&PrDetail>) -> Vec<Cell> {
    let mut out = vec![cell("THREADS", CellStyle::Dim)];
    match detail {
        Some(detail) => {
            let open = detail.threads.iter().filter(|t| !t.is_resolved).count();
            let style = if open == 0 {
                CellStyle::Dim
            } else {
                CellStyle::Warn
            };
            out.push(cell(format!(" {open} open"), style));
        }
        None => out.push(cell(" …", CellStyle::Dim)),
    }
    out
}

/// The size of the change: `3 files +42 −7`.
fn size(head: &PrHead) -> Vec<Cell> {
    let files = if head.changed_files == 1 {
        "1 file".to_owned()
    } else {
        format!("{} files", head.changed_files)
    };
    vec![
        cell(files, CellStyle::Primary),
        cell(format!(" +{}", head.additions), CellStyle::Success),
        cell(format!(" −{}", head.deletions), CellStyle::Danger),
    ]
}

/// The newest event of the detail: its author, its verb and its time.
/// Issue comments, submitted reviews and thread comments count.
fn latest_event(detail: &PrDetail) -> Option<(&str, String, DateTime<Utc>)> {
    let comments = detail
        .comments
        .iter()
        .map(|c| (c.author.as_str(), "commented".to_owned(), c.created_at));
    let reviews = detail
        .reviews
        .iter()
        .filter(|r| r.state != ReviewState::Pending)
        .filter_map(|r| {
            let verb = match r.state {
                ReviewState::Approved => "approved",
                ReviewState::ChangesRequested => "requested changes",
                ReviewState::Commented | ReviewState::Dismissed | ReviewState::Pending => {
                    "reviewed"
                }
            };
            r.submitted_at
                .map(|at| (r.author.as_str(), verb.to_owned(), at))
        });
    let threads = detail.threads.iter().flat_map(|t| {
        t.comments.iter().map(|c| {
            (
                c.author.as_str(),
                format!("commented on {}", t.path),
                c.created_at,
            )
        })
    });
    comments
        .chain(reviews)
        .chain(threads)
        .max_by_key(|(_, _, at)| *at)
}

/// Line 3: the latest activity, then the stack position.
fn activity(
    head: &PrHead,
    detail: Option<&PrDetail>,
    error: bool,
    now: DateTime<Utc>,
) -> Vec<Cell> {
    let mut out = if error {
        vec![cell("cannot load the activity", CellStyle::Danger)]
    } else {
        match detail {
            None => vec![cell("loading the activity…", CellStyle::Dim)],
            Some(detail) => match latest_event(detail) {
                Some((author, verb, at)) => vec![
                    cell(author, CellStyle::Info),
                    cell(format!(" {verb} {} ago", age(at, now)), CellStyle::Dim),
                ],
                None => vec![cell("no activity yet", CellStyle::Dim)],
            },
        }
    };
    if let Some(stack) = &head.stack {
        out.push(cell(
            format!(" · stack {} of {}", stack.position, stack.size),
            CellStyle::Dim,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;
    use crate::app::layout::layout;
    use crate::app::state::AppState;
    use crate::app::target::pr_target;
    use crate::app::testdata;
    use crate::domain::check::CheckKind;
    use crate::domain::detail::{Comment, Review, Thread};
    use crate::domain::test_time;

    /// The lines of the pane for a pull request with `body`.
    fn lines_for(body: &str) -> Vec<Vec<Cell>> {
        lines_with(body, None, None)
    }

    /// The lines of the pane for a pull request with `body`, with the
    /// detail `detail` and the detail error `error`.
    fn lines_with(body: &str, detail: Option<PrDetail>, error: Option<&str>) -> Vec<Vec<Cell>> {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        state.data.now = test_time() + TimeDelta::hours(3);
        let mut pr = testdata::pr("chaoshub", 7, "Seven");
        pr.head.value.body = body.into();
        if let Some(detail) = detail {
            state
                .data
                .details
                .insert(pr.key.clone(), testdata::observed(detail));
        }
        if let Some(error) = error {
            state
                .data
                .detail_errors
                .insert(pr.key.clone(), error.into());
        }
        state.data.prs.insert(pr.key.clone(), pr.clone());
        let target = pr_target(&state.data, &pr.key).unwrap();
        assert!(DescriptionPane.applies(&target));
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let PaneModel::Lines { lines, wrap } = DescriptionPane.model(
            &cx,
            &PaneState::default(),
            Size {
                width: 80,
                height: 20,
            },
        ) else {
            panic!("not lines");
        };
        assert!(wrap);
        lines
    }

    /// The text of a line.
    fn text(cells: &[Cell]) -> String {
        cells.iter().map(|c| c.text.as_str()).collect()
    }

    /// The head of an open pull request that is ready to merge.
    fn ready() -> PrHead {
        let mut head = testdata::pr("chaoshub", 7, "Seven").head.value;
        head.mergeable = Mergeable::Mergeable;
        head.merge_state = MergeStateStatus::Clean;
        head
    }

    /// A check with `status` and `conclusion`.
    fn check(status: CheckStatus, conclusion: Option<CheckConclusion>) -> Check {
        Check {
            name: "ci".into(),
            kind: CheckKind::CheckRun,
            status,
            conclusion,
            started: None,
            completed: None,
            url: None,
            run_id: None,
            suite_id: None,
        }
    }

    /// A completed check with `conclusion`.
    fn done(conclusion: CheckConclusion) -> Check {
        check(CheckStatus::Completed, Some(conclusion))
    }

    /// The badge text and style of `head` with the CI state `ci`.
    fn badge_of(head: &PrHead, ci: CiState) -> (String, CellStyle) {
        let line = verdict(head, ci, &[]);
        (line[0].text.clone(), line[0].style)
    }

    /// Asserts the badge of `head` with `ci`.
    fn assert_badge(head: &PrHead, ci: CiState, text: &str, tone: Tone) {
        assert_eq!(
            badge_of(head, ci),
            (format!(" {text} "), CellStyle::Badge(tone))
        );
    }

    /// An empty detail at the head of the test pull request.
    fn empty_detail() -> PrDetail {
        PrDetail {
            comments: Vec::new(),
            reviews: Vec::new(),
            threads: Vec::new(),
            head_oid: "oid7".into(),
        }
    }

    /// A comment of `author` at `minutes` after the test time.
    fn comment(author: &str, minutes: i64) -> Comment {
        Comment {
            author: author.into(),
            body: "text".into(),
            body_html: String::new(),
            created_at: test_time() + TimeDelta::minutes(minutes),
            url: String::new(),
        }
    }

    /// A review of `author` in `state` at `minutes` after the test time.
    fn review(author: &str, state: ReviewState, minutes: Option<i64>) -> Review {
        Review {
            author: author.into(),
            state,
            body: String::new(),
            body_html: String::new(),
            submitted_at: minutes.map(|m| test_time() + TimeDelta::minutes(m)),
        }
    }

    /// A thread on `path` with `comments`.
    fn thread(path: &str, is_resolved: bool, comments: Vec<Comment>) -> Thread {
        Thread {
            path: path.into(),
            line: Some(1),
            is_resolved,
            comments,
        }
    }

    #[test]
    fn the_body_is_one_markdown_cell() {
        let lines = lines_for("\r\n## Why\r\n\r\nThe body.\r\n\r\n");
        assert_eq!(lines.len(), 5);
        assert_eq!(
            lines[3..],
            [
                Vec::new(),
                vec![cell("## Why\n\nThe body.", CellStyle::Markdown)]
            ]
        );
    }

    #[test]
    fn a_blank_body_shows_no_description() {
        for body in ["", "  \r\n\n "] {
            let lines = lines_for(body);
            assert_eq!(
                lines[3..],
                [Vec::new(), vec![cell("(no description)", CellStyle::Dim)]],
                "{body:?}"
            );
        }
    }

    #[test]
    fn the_block_takes_the_first_three_lines() {
        let lines = lines_for("Body");
        assert_eq!(
            lines[..3].iter().map(|line| text(line)).collect::<Vec<_>>(),
            [
                " CHECKING ",
                "CHECKS none   REVIEWS none   THREADS …   3 files +42 −7",
                "loading the activity…",
            ]
        );
        let lines = lines_with("Body", Some(testdata::detail()), None);
        assert_eq!(
            text(&lines[1]),
            "CHECKS none   REVIEWS none   THREADS 1 open   3 files +42 −7"
        );
        assert_eq!(text(&lines[2]), "bob requested changes 1h ago");
    }

    #[test]
    fn a_merged_pull_request_is_merged() {
        let mut head = ready();
        head.state = PrState::Merged;
        head.is_draft = true;
        assert_badge(&head, CiState::Failure, "MERGED", Tone::Merge);
    }

    #[test]
    fn a_closed_pull_request_is_closed() {
        let mut head = ready();
        head.state = PrState::Closed;
        head.is_draft = true;
        assert_badge(&head, CiState::Failure, "CLOSED", Tone::Dim);
    }

    #[test]
    fn a_draft_is_a_draft() {
        let mut head = ready();
        head.is_draft = true;
        head.in_merge_queue = true;
        assert_badge(&head, CiState::Failure, "DRAFT", Tone::Dim);
    }

    #[test]
    fn a_queued_pull_request_is_in_the_queue() {
        let mut head = ready();
        head.in_merge_queue = true;
        head.mergeable = Mergeable::Conflicting;
        assert_badge(&head, CiState::Success, "IN QUEUE", Tone::Merge);
    }

    #[test]
    fn conflicts_come_from_mergeable_or_a_dirty_merge_state() {
        let mut head = ready();
        head.mergeable = Mergeable::Conflicting;
        head.review_decision = Some(ReviewDecision::ChangesRequested);
        assert_badge(&head, CiState::Failure, "CONFLICTS", Tone::Danger);
        let mut head = ready();
        head.merge_state = MergeStateStatus::Dirty;
        assert_badge(&head, CiState::Success, "CONFLICTS", Tone::Danger);
    }

    #[test]
    fn failed_ci_or_requested_changes_block_in_danger() {
        let mut head = ready();
        head.review_decision = Some(ReviewDecision::ReviewRequired);
        assert_badge(&head, CiState::Failure, "BLOCKED", Tone::Danger);
        let mut head = ready();
        head.review_decision = Some(ReviewDecision::ChangesRequested);
        assert_badge(&head, CiState::Success, "BLOCKED", Tone::Danger);
    }

    #[test]
    fn a_required_review_or_the_merge_state_block_in_warn() {
        let mut head = ready();
        head.review_decision = Some(ReviewDecision::ReviewRequired);
        assert_badge(&head, CiState::Pending, "BLOCKED", Tone::Warn);
        for state in [MergeStateStatus::Behind, MergeStateStatus::Blocked] {
            let mut head = ready();
            head.merge_state = state;
            assert_badge(&head, CiState::Pending, "BLOCKED", Tone::Warn);
        }
    }

    #[test]
    fn pending_ci_waits() {
        let mut head = ready();
        head.mergeable = Mergeable::Unknown;
        assert_badge(&head, CiState::Pending, "WAITING", Tone::Warn);
    }

    #[test]
    fn an_unknown_merge_state_is_checking() {
        let mut head = ready();
        head.mergeable = Mergeable::Unknown;
        assert_badge(&head, CiState::Success, "CHECKING", Tone::Dim);
        let mut head = ready();
        head.merge_state = MergeStateStatus::Unknown;
        assert_badge(&head, CiState::None, "CHECKING", Tone::Dim);
    }

    #[test]
    fn else_it_is_ready_to_merge() {
        let head = ready();
        assert_badge(&head, CiState::Success, "READY", Tone::Success);
        assert_eq!(
            verdict(&head, CiState::Success, &[]),
            [
                cell(" READY ", CellStyle::Badge(Tone::Success)),
                cell(" ╱ ", CellStyle::Success),
                cell("ready to merge", CellStyle::Normal),
            ]
        );
    }

    #[test]
    fn merged_closed_and_draft_without_reasons_have_only_the_badge() {
        for edit in [
            (|h: &mut PrHead| h.state = PrState::Merged) as fn(&mut PrHead),
            |h| h.state = PrState::Closed,
            |h| h.is_draft = true,
        ] {
            let mut head = ready();
            edit(&mut head);
            assert_eq!(verdict(&head, CiState::Success, &[]).len(), 1);
        }
    }

    #[test]
    fn the_reasons_in_order() {
        let mut head = ready();
        head.review_decision = Some(ReviewDecision::ChangesRequested);
        head.mergeable = Mergeable::Conflicting;
        head.merge_state = MergeStateStatus::Behind;
        head.auto_merge = true;
        let checks = [
            done(CheckConclusion::Failure),
            done(CheckConclusion::TimedOut),
            check(CheckStatus::InProgress, None),
            done(CheckConclusion::Success),
        ];
        let line = verdict(&head, CiState::Failure, &checks);
        assert_eq!(
            text(&line),
            " CONFLICTS  ╱ 2 checks failed · 1 check running · changes requested \
             · conflicts · behind main · auto-merge on"
        );
        assert_eq!(line[1], cell(" ╱ ", CellStyle::Danger));
        let styles: Vec<CellStyle> = line[2..].iter().step_by(2).map(|c| c.style).collect();
        assert_eq!(
            styles,
            [
                CellStyle::Danger,
                CellStyle::Warn,
                CellStyle::Danger,
                CellStyle::Danger,
                CellStyle::Warn,
                CellStyle::Dim,
            ]
        );
        assert!(
            line[3..]
                .iter()
                .step_by(2)
                .all(|c| *c == cell(" · ", CellStyle::Normal))
        );
        let mut head = ready();
        head.review_decision = Some(ReviewDecision::ReviewRequired);
        let checks = [done(CheckConclusion::Failure)];
        assert_eq!(
            text(&verdict(&head, CiState::Failure, &checks)),
            " BLOCKED  ╱ 1 check failed · review required"
        );
    }

    #[test]
    fn the_check_counts_leave_out_zeros() {
        assert_eq!(
            check_signal(&[]),
            [
                cell("CHECKS", CellStyle::Dim),
                cell(" none", CellStyle::Dim)
            ]
        );
        let checks = [
            done(CheckConclusion::Success),
            done(CheckConclusion::Success),
            done(CheckConclusion::Failure),
            done(CheckConclusion::StartupFailure),
            done(CheckConclusion::ActionRequired),
            done(CheckConclusion::Cancelled),
            done(CheckConclusion::Skipped),
            check(CheckStatus::Queued, None),
        ];
        assert_eq!(
            check_signal(&checks),
            [
                cell("CHECKS", CellStyle::Dim),
                cell(" ✓2", CellStyle::Success),
                cell(" ✗4", CellStyle::Danger),
                cell(" ●1", CellStyle::Warn),
            ]
        );
        let checks = [done(CheckConclusion::Success)];
        assert_eq!(text(&check_signal(&checks)), "CHECKS ✓1");
    }

    #[test]
    fn the_reviews_show_the_counts_of_the_list() {
        let mut head = ready();
        head.approvals = 1;
        head.changes_requested = 1;
        assert_eq!(
            review_signal(&head),
            [
                cell("REVIEWS", CellStyle::Dim),
                cell(" ✓1", CellStyle::Success),
                cell(" ✗1", CellStyle::Danger),
            ]
        );
        head.reviewers = vec!["bob".into(), "erin".into()];
        assert_eq!(
            text(&review_signal(&head)),
            "REVIEWS ✓1 ✗1 · @bob @erin requested"
        );
        assert_eq!(
            review_signal(&head).last(),
            Some(&cell(" · @bob @erin requested", CellStyle::Dim))
        );
        head.changes_requested = 0;
        assert_eq!(
            text(&review_signal(&head)),
            "REVIEWS ✓1 · @bob @erin requested"
        );
    }

    #[test]
    fn the_reviews_need_no_detail() {
        // The review of the user went out: GitHub counts it and drops the
        // request. An old detail with no reviews changes nothing.
        let mut head = ready();
        head.approvals = 1;
        let mut detail = empty_detail();
        assert_eq!(
            text(&signals(&head, &[], Some(&detail))),
            "CHECKS none   REVIEWS ✓1   THREADS 0 open   3 files +42 −7"
        );
        detail.reviews = vec![review("alice", ReviewState::ChangesRequested, Some(0))];
        assert_eq!(text(&review_signal(&head)), "REVIEWS ✓1");
    }

    #[test]
    fn the_reviews_without_counts_show_the_decision_or_none() {
        let mut head = ready();
        assert_eq!(text(&review_signal(&head)), "REVIEWS none");
        head.reviewers = vec!["bob".into()];
        assert_eq!(text(&review_signal(&head)), "REVIEWS @bob requested");
        head.review_decision = Some(ReviewDecision::ReviewRequired);
        assert_eq!(
            review_signal(&head),
            [
                cell("REVIEWS", CellStyle::Dim),
                cell(" ○", CellStyle::Dim),
                cell(" · @bob requested", CellStyle::Dim),
            ]
        );
        head.reviewers.clear();
        head.review_decision = Some(ReviewDecision::Approved);
        assert_eq!(review_signal(&head)[1], cell(" ✓", CellStyle::Success));
        head.review_decision = Some(ReviewDecision::ChangesRequested);
        assert_eq!(review_signal(&head)[1], cell(" ✗", CellStyle::Danger));
    }

    #[test]
    fn the_threads_count_the_open_threads() {
        assert_eq!(
            thread_signal(None),
            [cell("THREADS", CellStyle::Dim), cell(" …", CellStyle::Dim)]
        );
        let mut detail = empty_detail();
        assert_eq!(
            thread_signal(Some(&detail))[1],
            cell(" 0 open", CellStyle::Dim)
        );
        detail.threads = vec![
            thread("a.rs", false, Vec::new()),
            thread("b.rs", true, Vec::new()),
            thread("c.rs", false, Vec::new()),
        ];
        assert_eq!(
            thread_signal(Some(&detail))[1],
            cell(" 2 open", CellStyle::Warn)
        );
    }

    #[test]
    fn the_size_counts_the_files_and_the_lines() {
        let mut head = ready();
        head.changed_files = 1;
        head.additions = 5;
        head.deletions = 0;
        assert_eq!(
            size(&head),
            [
                cell("1 file", CellStyle::Primary),
                cell(" +5", CellStyle::Success),
                cell(" −0", CellStyle::Danger),
            ]
        );
        head.changed_files = 12;
        head.additions = 340;
        head.deletions = 56;
        assert_eq!(text(&size(&head)), "12 files +340 −56");
    }

    #[test]
    fn the_signals_join_the_groups_with_three_spaces() {
        let mut head = ready();
        head.reviewers = vec!["bob".into()];
        head.approvals = 1;
        let mut detail = empty_detail();
        detail.threads = vec![thread("a.rs", false, Vec::new())];
        let checks = [
            done(CheckConclusion::Success),
            done(CheckConclusion::Failure),
        ];
        assert_eq!(
            text(&signals(&head, &checks, Some(&detail))),
            "CHECKS ✓1 ✗1   REVIEWS ✓1 · @bob requested   THREADS 1 open   3 files +42 −7"
        );
    }

    #[test]
    fn the_activity_shows_the_newest_event_of_each_kind() {
        let now = test_time() + TimeDelta::hours(3);
        let head = ready();
        let line = |detail: &PrDetail| text(&activity(&head, Some(detail), false, now));
        let mut detail = empty_detail();
        detail.comments = vec![comment("alice", 0), comment("bob", 60)];
        assert_eq!(line(&detail), "bob commented 2h ago");
        let states = [
            (ReviewState::Approved, "approved"),
            (ReviewState::ChangesRequested, "requested changes"),
            (ReviewState::Commented, "reviewed"),
        ];
        for (state, verb) in states {
            let mut detail = empty_detail();
            detail.comments = vec![comment("alice", 0)];
            detail.reviews = vec![
                review("carol", state, Some(120)),
                review("dave", ReviewState::Pending, None),
            ];
            assert_eq!(line(&detail), format!("carol {verb} 1h ago"));
        }
        let mut detail = empty_detail();
        detail.comments = vec![comment("alice", 0)];
        detail.threads = vec![thread(
            "src/ui/pane.rs",
            false,
            vec![comment("bob", 10), comment("erin", 170)],
        )];
        assert_eq!(line(&detail), "erin commented on src/ui/pane.rs 10m ago");
        assert_eq!(
            activity(&head, Some(&detail), false, now),
            [
                cell("erin", CellStyle::Info),
                cell(" commented on src/ui/pane.rs 10m ago", CellStyle::Dim),
            ]
        );
    }

    #[test]
    fn the_activity_without_events_a_detail_or_with_an_error() {
        let now = test_time();
        let head = ready();
        assert_eq!(
            activity(&head, None, false, now),
            [cell("loading the activity…", CellStyle::Dim)]
        );
        assert_eq!(
            activity(&head, Some(&empty_detail()), false, now),
            [cell("no activity yet", CellStyle::Dim)]
        );
        assert_eq!(
            activity(&head, None, true, now),
            [cell("cannot load the activity", CellStyle::Danger)]
        );
        let lines = lines_with("", None, Some("offline"));
        assert_eq!(text(&lines[2]), "cannot load the activity");
    }

    #[test]
    fn the_activity_shows_the_stack_position() {
        let now = test_time();
        let head = testdata::stacked(7, 2, "Seven").head.value;
        assert_eq!(
            activity(&head, None, false, now),
            [
                cell("loading the activity…", CellStyle::Dim),
                cell(" · stack 2 of 3", CellStyle::Dim),
            ]
        );
        assert_eq!(
            text(&activity(&head, Some(&empty_detail()), false, now)),
            "no activity yet · stack 2 of 3"
        );
    }

    /// The codeview comments of the tests on the head `oid7` of #7: a
    /// comment on lines, a pull request comment and a commit message
    /// comment, and one comment on the older push `old1`.
    fn codeview_comments() -> crate::domain::codeview::Comments {
        use crate::domain::codeview::{CommentKind, Comments, Push};
        let comment = |id: &str, file: &str, lines: (u32, u32), body: &str| {
            crate::domain::codeview::Comment {
                id: id.into(),
                file: file.into(),
                start_line: lines.0,
                end_line: lines.1,
                side: "new".into(),
                body: body.into(),
                kind: CommentKind::of(file),
            }
        };
        Comments {
            pushes: vec![
                Push {
                    head: "oid7".into(),
                    comments: vec![
                        comment("c1", "src/lib.rs", (12, 14), "Trim the name.\nMore text."),
                        comment("c2", "codeview://commit/oid7", (1, 1), "Fix the typo"),
                        comment("c3", "codeview://pr/7", (1, 1), "Looks good overall."),
                    ],
                },
                Push {
                    head: "old1".into(),
                    comments: vec![comment("c0", "src/lib.rs", (4, 4), "An old note")],
                },
            ],
        }
    }

    /// The state of #7 with `comments`.
    fn codeview_state(
        comments: Option<Result<crate::domain::codeview::Comments, String>>,
    ) -> AppState {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        let pr = testdata::pr("chaoshub", 7, "Seven");
        if let Some(comments) = comments {
            state.data.codeview.insert(pr.key.clone(), comments);
        }
        state.data.prs.insert(pr.key.clone(), pr);
        state
    }

    /// The pane of #7 in `state` with the key map `keys`: its lines, the
    /// result of `enter` and the count of its own actions.
    fn codeview_pane(state: &AppState, keys: &Keymap) -> (Vec<Vec<Cell>>, Option<Handled>, usize) {
        let target = pr_target(&state.data, &testdata::pr_key("chaoshub", 7)).unwrap();
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys,
        };
        let size = Size {
            width: 80,
            height: 20,
        };
        let mut ps = DescriptionPane.init_state();
        let PaneModel::Lines { lines, .. } = DescriptionPane.model(&cx, &ps, size) else {
            panic!("not lines");
        };
        let enter = KeyChord {
            code: crate::bus::KeyCode::Enter,
            mods: crate::bus::KeyModifiers::NONE,
        };
        let handled = DescriptionPane.handle_key(enter, &cx, &mut ps);
        (lines, handled, DescriptionPane.actions(&cx, &ps).len())
    }

    /// The texts of the lines of the pane of #7 in `state`.
    fn codeview_texts(state: &AppState, keys: &Keymap) -> Vec<String> {
        let (lines, ..) = codeview_pane(state, keys);
        lines.iter().map(|line| text(line)).collect()
    }

    #[test]
    fn one_dim_line_counts_the_unsent_comments_of_the_head() {
        let state = codeview_state(Some(Ok(codeview_comments())));
        let (lines, enter, actions) = codeview_pane(&state, &crate::app::keymap::DEFAULT);
        assert_eq!(
            codeview_texts(&state, &crate::app::keymap::DEFAULT)[3..],
            [
                "",
                "3 unsent codeview comments · space r to review",
                "",
                "(no description)",
            ]
        );
        assert_eq!(lines[4][0].style, CellStyle::Dim);
        assert_eq!(enter, None, "enter marks nothing in the pane");
        assert_eq!(actions, 0, "the pane has no send");
    }

    #[test]
    fn the_line_follows_the_keys_and_the_count() {
        let mut comments = codeview_comments();
        comments.pushes[0].comments.truncate(1);
        let state = codeview_state(Some(Ok(comments)));
        let overrides =
            std::collections::BTreeMap::from([("codeview_send".to_owned(), "ctrl-r".to_owned())]);
        let keys = Keymap::new(&overrides).unwrap();
        let lines = codeview_texts(&state, &keys);
        assert_eq!(
            lines[4],
            "1 unsent codeview comment · space ctrl-r to review"
        );
        let overrides =
            std::collections::BTreeMap::from([("codeview_send".to_owned(), String::new())]);
        let keys = Keymap::new(&overrides).unwrap();
        let lines = codeview_texts(&state, &keys);
        assert_eq!(lines[4], "1 unsent codeview comment");
    }

    #[test]
    fn without_unsent_comments_on_the_head_the_pane_shows_no_line() {
        let empty = crate::domain::codeview::Comments::default();
        let mut older = codeview_comments();
        older.pushes.remove(0);
        for comments in [
            None,
            Some(Ok(empty)),
            Some(Ok(older)),
            Some(Err("version 2 is not 1".into())),
        ] {
            let state = codeview_state(comments);
            let lines = codeview_texts(&state, &crate::app::keymap::DEFAULT);
            assert_eq!(lines.len(), 5, "{lines:?}");
        }
    }

    /// The state of #7 with the merge run `merge` of #7, `elapsed` after
    /// the start of the run.
    fn merge_state(merge: Option<MergeState>, elapsed: TimeDelta) -> AppState {
        let mut state = codeview_state(None);
        state.data.now = test_time() + elapsed;
        state.data.merge = merge;
        state
    }

    /// A merge run of #7 that started at the test time.
    fn run_of_7() -> MergeState {
        MergeState::new(testdata::pr_key("chaoshub", 7), test_time())
    }

    /// Line 1 of the pane of #7 in `state`.
    fn line_1(state: &AppState) -> Vec<Cell> {
        let (lines, ..) = codeview_pane(state, &crate::app::keymap::DEFAULT);
        lines[0].clone()
    }

    #[test]
    fn a_merge_shows_merging_on_a_badge_that_pulses() {
        let ms = TimeDelta::milliseconds;
        for (elapsed, mix) in [(0, 0), (250, 64), (750, 255), (1250, 64), (1500, 0)] {
            let state = merge_state(Some(run_of_7()), ms(elapsed));
            let text = format!(" {} MERGING ", spinner(state.data.now));
            assert_eq!(
                line_1(&state),
                [cell(text, CellStyle::MergeBadge(mix))],
                "{elapsed} ms"
            );
        }
    }

    #[test]
    fn without_animations_the_merge_badge_stays_green() {
        let mut state = merge_state(Some(run_of_7()), TimeDelta::milliseconds(750));
        state.data.animations = false;
        assert_eq!(line_1(&state)[0].style, CellStyle::MergeBadge(0));
    }

    #[test]
    fn the_merge_badge_shows_the_step_of_the_run() {
        let merged = MergeProgress::Merged { sha: "abc".into() };
        let entry = |number, index, total| MergeStep::Entry {
            number,
            index,
            total,
        };
        let cases = [
            (
                entry(12, 2, 3),
                MergeProgress::Pending,
                false,
                "MERGING 2/3 #12",
            ),
            (entry(12, 2, 3), merged.clone(), false, "MERGING 2/3 #12"),
            (
                MergeStep::Retarget { number: 13 },
                merged.clone(),
                false,
                "MERGING · waiting for #13 to retarget",
            ),
            (entry(13, 3, 3), merged.clone(), false, "MERGING · fetching"),
            (entry(7, 1, 1), merged.clone(), false, "MERGING · fetching"),
            (entry(7, 1, 1), merged, true, "MERGING · fetching"),
        ];
        for (step, progress, done, text) in cases {
            let merge = MergeState {
                progress,
                step,
                done,
                ..run_of_7()
            };
            let state = merge_state(Some(merge), TimeDelta::zero());
            let line = line_1(&state);
            assert_eq!(line.len(), 1, "{text}");
            assert_eq!(
                line[0].text,
                format!(" {} {text} ", spinner(state.data.now))
            );
        }
    }

    #[test]
    fn a_failed_merge_shows_the_reason_on_a_red_badge() {
        let merge = MergeState {
            progress: MergeProgress::Failed("checks failing".into()),
            done: true,
            ..run_of_7()
        };
        let state = merge_state(Some(merge), TimeDelta::zero());
        assert_eq!(
            line_1(&state),
            [
                cell(" MERGE FAILED ", CellStyle::Badge(Tone::Danger)),
                cell(" ╱ ", CellStyle::Danger),
                cell("checks failing", CellStyle::Danger),
            ]
        );
    }

    #[test]
    fn a_merge_into_the_queue_ends_on_the_queue_badge() {
        let merge = MergeState {
            progress: MergeProgress::Enqueued,
            done: true,
            ..run_of_7()
        };
        let state = merge_state(Some(merge), TimeDelta::zero());
        assert_eq!(
            line_1(&state),
            [cell(" IN QUEUE ", CellStyle::Badge(Tone::Merge))]
        );
    }

    #[test]
    fn a_merged_pull_request_shows_merged_over_the_run() {
        let merge = MergeState {
            progress: MergeProgress::Merged { sha: "abc".into() },
            done: true,
            ..run_of_7()
        };
        let mut state = merge_state(Some(merge), TimeDelta::zero());
        let key = testdata::pr_key("chaoshub", 7);
        state.data.prs.get_mut(&key).unwrap().head.value.state = PrState::Merged;
        assert_eq!(
            line_1(&state),
            [cell(" MERGED ", CellStyle::Badge(Tone::Merge))]
        );
    }

    #[test]
    fn the_run_of_another_pull_request_does_not_change_the_verdict() {
        let other = MergeState::new(testdata::pr_key("chaoshub", 8), test_time());
        let state = merge_state(Some(other), TimeDelta::zero());
        let plain = merge_state(None, TimeDelta::zero());
        assert_eq!(line_1(&state), line_1(&plain));
    }
}
