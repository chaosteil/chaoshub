//! Cells that several tabs share: CI, review and flag glyphs.

use crate::app::models::{Cell, CellStyle};
use crate::domain::pr::{CiState, Mergeable, PullRequest, ReviewDecision};

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

/// The flags: `D` draft, `C` conflicting, `Q` merge queue.
pub(crate) fn flags(pr: &PullRequest) -> Cell {
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
    fn flags_in_order() {
        let mut pr = testdata::pr("r", 1, "One");
        assert_eq!(flags(&pr).text, "");
        pr.head.value.is_draft = true;
        pr.head.value.mergeable = Mergeable::Conflicting;
        pr.head.value.in_merge_queue = true;
        assert_eq!(flags(&pr), cell("DCQ", CellStyle::Warn));
    }
}
