//! Cells that several tabs share: CI, review and flag glyphs.

use crate::app::models::{Cell, CellStyle};
use crate::domain::check::CheckConclusion;
use crate::domain::pr::{CiState, Mergeable, PrHead, PullRequest, ReviewDecision};

/// A cell.
pub(crate) fn cell(text: impl Into<String>, style: CellStyle) -> Cell {
    Cell {
        text: text.into(),
        style,
    }
}

/// The CI glyph: `✓` success, `✗` failure, `●` pending, `·` none.
pub(crate) fn ci(state: CiState) -> Cell {
    match state {
        CiState::Success => cell("✓", CellStyle::Success),
        CiState::Failure => cell("✗", CellStyle::Danger),
        CiState::Pending => cell("●", CellStyle::Warn),
        CiState::None => cell("·", CellStyle::Dim),
    }
}

/// The CI glyph and the passed checks of all checks: `✓ 14/14`,
/// `✗ 12/14` or `● 9/14`, in the style of the CI glyph. A check passed
/// with the conclusion `Success`, `Neutral` or `Skipped`. `·` without
/// checks.
pub(crate) fn checks(pr: &PullRequest) -> Cell {
    let checks = &pr.checks.value;
    if checks.is_empty() {
        return cell("·", CellStyle::Dim);
    }
    let passed = checks
        .iter()
        .filter(|check| {
            matches!(
                check.conclusion,
                Some(
                    CheckConclusion::Success | CheckConclusion::Neutral | CheckConclusion::Skipped
                )
            )
        })
        .count();
    let glyph = ci(pr.ci_state.value);
    cell(
        format!("{} {passed}/{}", glyph.text, checks.len()),
        glyph.style,
    )
}

/// The review counts: `✓n` approvals, then `✗n` change requests. With
/// both counts, the whole cell has the danger style: a change request
/// matters more. Without counts, the `review` glyph of the decision.
pub(crate) fn reviews(head: &PrHead) -> Cell {
    let mut parts = Vec::new();
    if head.approvals > 0 {
        parts.push(format!("✓{}", head.approvals));
    }
    if head.changes_requested > 0 {
        parts.push(format!("✗{}", head.changes_requested));
    }
    if parts.is_empty() {
        return review(head.review_decision);
    }
    let style = if head.changes_requested > 0 {
        CellStyle::Danger
    } else {
        CellStyle::Success
    };
    cell(parts.join(" "), style)
}

/// The review glyph: `✓` approved, `✗` changes requested, `○` review
/// required, `·` none.
pub(crate) fn review(decision: Option<ReviewDecision>) -> Cell {
    match decision {
        Some(ReviewDecision::Approved) => cell("✓", CellStyle::Success),
        Some(ReviewDecision::ChangesRequested) => cell("✗", CellStyle::Danger),
        Some(ReviewDecision::ReviewRequired) => cell("○", CellStyle::Dim),
        None => cell("·", CellStyle::Dim),
    }
}

/// The flags: `D` draft, `C` conflicting, `Q` merge queue, `◆` an open
/// pane.
pub(crate) fn flags(pr: &PullRequest, pane: bool) -> Cell {
    let head = &pr.head.value;
    let mut text = String::new();
    if head.is_draft {
        text.push('D');
    }
    if head.mergeable == Mergeable::Conflicting {
        text.push('C');
    }
    if head.in_merge_queue {
        text.push('Q');
    }
    if pane {
        text.push('◆');
    }
    cell(text, CellStyle::Warn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testdata;

    #[test]
    fn glyphs() {
        let cases = [
            (ci(CiState::Success), "✓", CellStyle::Success),
            (ci(CiState::Failure), "✗", CellStyle::Danger),
            (ci(CiState::Pending), "●", CellStyle::Warn),
            (ci(CiState::None), "·", CellStyle::Dim),
            (
                review(Some(ReviewDecision::Approved)),
                "✓",
                CellStyle::Success,
            ),
            (
                review(Some(ReviewDecision::ChangesRequested)),
                "✗",
                CellStyle::Danger,
            ),
            (
                review(Some(ReviewDecision::ReviewRequired)),
                "○",
                CellStyle::Dim,
            ),
            (review(None), "·", CellStyle::Dim),
        ];
        for (got, text, style) in cases {
            assert_eq!(got, cell(text, style));
        }
    }

    #[test]
    fn checks_count_the_passed_checks() {
        use crate::domain::check::{Check, CheckKind, CheckStatus};

        let check = |conclusion: Option<CheckConclusion>| Check {
            name: "ci".into(),
            kind: CheckKind::CheckRun,
            status: if conclusion.is_some() {
                CheckStatus::Completed
            } else {
                CheckStatus::InProgress
            },
            conclusion,
            started: None,
            completed: None,
            url: None,
            run_id: None,
            suite_id: None,
        };
        let mut pr = testdata::pr("r", 1, "One");
        assert_eq!(checks(&pr), cell("·", CellStyle::Dim));
        pr.checks.value = vec![
            check(Some(CheckConclusion::Success)),
            check(Some(CheckConclusion::Neutral)),
            check(Some(CheckConclusion::Skipped)),
            check(Some(CheckConclusion::Failure)),
            check(None),
        ];
        let cases = [
            (CiState::Success, "✓ 3/5", CellStyle::Success),
            (CiState::Failure, "✗ 3/5", CellStyle::Danger),
            (CiState::Pending, "● 3/5", CellStyle::Warn),
            (CiState::None, "· 3/5", CellStyle::Dim),
        ];
        for (state, text, style) in cases {
            pr.ci_state.value = state;
            assert_eq!(checks(&pr), cell(text, style), "{state:?}");
        }
    }

    #[test]
    fn reviews_show_the_counts_or_the_decision() {
        let mut head = testdata::pr("r", 1, "One").head.value;
        assert_eq!(reviews(&head), cell("·", CellStyle::Dim));
        head.review_decision = Some(ReviewDecision::ReviewRequired);
        assert_eq!(reviews(&head), cell("○", CellStyle::Dim));
        head.approvals = 2;
        assert_eq!(reviews(&head), cell("✓2", CellStyle::Success));
        head.changes_requested = 1;
        assert_eq!(reviews(&head), cell("✓2 ✗1", CellStyle::Danger));
        head.approvals = 0;
        assert_eq!(reviews(&head), cell("✗1", CellStyle::Danger));
    }

    #[test]
    fn flags_in_order() {
        let mut pr = testdata::pr("r", 1, "One");
        assert_eq!(flags(&pr, false).text, "");
        assert_eq!(flags(&pr, true).text, "◆");
        pr.head.value.is_draft = true;
        pr.head.value.mergeable = Mergeable::Conflicting;
        pr.head.value.in_merge_queue = true;
        assert_eq!(flags(&pr, false), cell("DCQ", CellStyle::Warn));
        assert_eq!(flags(&pr, true), cell("DCQ◆", CellStyle::Warn));
    }
}
