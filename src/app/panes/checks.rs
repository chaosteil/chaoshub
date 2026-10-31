//! The checks pane of the pull request popup: one row per check with its
//! state and duration. `o` opens the page of the check under the cursor.

use chrono::{DateTime, Utc};

use crate::app::models::{
    Align, CellStyle, Column, ColumnWidth, PaneModel, Row, RowItem, RowStyle,
};
use crate::app::panes::{Pane, PaneState};
use crate::app::tabs::cells::cell;
use crate::app::tabs::{Handled, KeyCx, pr_ref};
use crate::bus::{Cmd, KeyChord, Size, Target};
use crate::domain::check::{Check, CheckConclusion, CheckStatus};

/// The checks pane.
pub(crate) struct Checks;

impl Pane for Checks {
    fn id(&self) -> &'static str {
        "checks"
    }

    fn title(&self) -> &'static str {
        "Checks"
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
        let column = |title: &str, width, align| Column {
            title: title.into(),
            width,
            align,
        };
        let columns = vec![
            column("name", ColumnWidth::Fill { min: 20 }, Align::Left),
            column("state", ColumnWidth::Fixed(12), Align::Left),
            column("duration", ColumnWidth::Fixed(8), Align::Right),
        ];
        let Some(Target::Pr { pr, .. }) = cx.target else {
            return PaneModel::Table {
                columns,
                rows: Vec::new(),
            };
        };
        let item = RowItem::Pr(pr_ref(pr));
        let rows = pr
            .checks
            .value
            .iter()
            .map(|check| {
                let (state, style) = state(check);
                Row {
                    item: item.clone(),
                    cells: vec![
                        cell(check.name.clone(), CellStyle::Normal),
                        cell(state, style),
                        cell(duration(check, cx.data.now), CellStyle::Dim),
                    ],
                    depth: 0,
                    glyph: None,
                    style: RowStyle::Normal,
                }
            })
            .collect();
        PaneModel::Table { columns, rows }
    }

    fn handle_key(&self, key: KeyChord, cx: &KeyCx<'_>, ps: &mut PaneState) -> Option<Handled> {
        if !cx.keys.matches("open_url", key) {
            return None;
        }
        let Some(Target::Pr { pr, .. }) = cx.target else {
            return None;
        };
        let url = pr.checks.value.get(ps.cursor)?.url.clone()?;
        Some(Handled {
            cmds: vec![Cmd::OpenUrl(url)],
            ..Handled::default()
        })
    }
}

/// The state text and style of a check.
fn state(check: &Check) -> (&'static str, CellStyle) {
    match (check.status, check.conclusion) {
        (CheckStatus::Completed, Some(conclusion)) => match conclusion {
            CheckConclusion::Success => ("✓ success", CellStyle::Success),
            CheckConclusion::Failure => ("✗ failure", CellStyle::Danger),
            CheckConclusion::TimedOut => ("✗ timed out", CellStyle::Danger),
            CheckConclusion::StartupFailure => ("✗ startup", CellStyle::Danger),
            CheckConclusion::ActionRequired => ("! action", CellStyle::Warn),
            CheckConclusion::Cancelled => ("⊘ cancelled", CellStyle::Dim),
            CheckConclusion::Skipped => ("· skipped", CellStyle::Dim),
            CheckConclusion::Neutral => ("· neutral", CellStyle::Dim),
            CheckConclusion::Stale => ("· stale", CellStyle::Dim),
        },
        (CheckStatus::Completed, None) => ("· done", CellStyle::Dim),
        (CheckStatus::InProgress, _) => ("● running", CellStyle::Warn),
        (
            CheckStatus::Queued
            | CheckStatus::Waiting
            | CheckStatus::Pending
            | CheckStatus::Requested,
            _,
        ) => ("○ queued", CellStyle::Dim),
    }
}

/// The run time of a check: until it completed, or until `now` while it
/// runs. Empty before it starts.
fn duration(check: &Check, now: DateTime<Utc>) -> String {
    let Some(started) = check.started else {
        return String::new();
    };
    let end = check.completed.unwrap_or(now);
    let secs = (end - started).num_seconds().max(0);
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;
    use crate::app::keymap::Keymap;
    use crate::app::layout::layout;
    use crate::app::state::AppState;
    use crate::app::target::pr_target;
    use crate::app::testdata;
    use crate::bus::{KeyCode, KeyModifiers};
    use crate::domain::check::CheckKind;
    use crate::domain::test_time;

    fn check(
        name: &str,
        status: CheckStatus,
        conclusion: Option<CheckConclusion>,
        secs: Option<i64>,
    ) -> Check {
        Check {
            name: name.into(),
            kind: CheckKind::CheckRun,
            status,
            conclusion,
            started: secs.map(|_| test_time()),
            completed: secs
                .filter(|_| status == CheckStatus::Completed)
                .map(|s| test_time() + TimeDelta::seconds(s)),
            url: Some(format!("https://ci/{name}")),
            run_id: None,
            suite_id: None,
        }
    }

    #[test]
    fn states_and_durations() {
        let now = test_time() + TimeDelta::seconds(48);
        let cases = [
            (
                check(
                    "a",
                    CheckStatus::Completed,
                    Some(CheckConclusion::Success),
                    Some(130),
                ),
                "✓ success",
                CellStyle::Success,
                "2m 10s",
            ),
            (
                check(
                    "b",
                    CheckStatus::Completed,
                    Some(CheckConclusion::Failure),
                    Some(3725),
                ),
                "✗ failure",
                CellStyle::Danger,
                "1h 02m",
            ),
            (
                check("c", CheckStatus::InProgress, None, Some(0)),
                "● running",
                CellStyle::Warn,
                "48s",
            ),
            (
                check("d", CheckStatus::Queued, None, None),
                "○ queued",
                CellStyle::Dim,
                "",
            ),
            (
                check(
                    "e",
                    CheckStatus::Completed,
                    Some(CheckConclusion::Skipped),
                    Some(0),
                ),
                "· skipped",
                CellStyle::Dim,
                "0s",
            ),
            (
                check(
                    "f",
                    CheckStatus::Completed,
                    Some(CheckConclusion::Cancelled),
                    Some(5),
                ),
                "⊘ cancelled",
                CellStyle::Dim,
                "5s",
            ),
        ];
        for (check, text, style, time) in cases {
            assert_eq!(state(&check), (text, style), "{}", check.name);
            assert_eq!(duration(&check, now), time, "{}", check.name);
        }
    }

    #[test]
    fn rows_and_the_open_url_key() {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        let mut pr = testdata::pr("chaoshub", 7, "Seven");
        pr.checks.value = vec![
            check(
                "build",
                CheckStatus::Completed,
                Some(CheckConclusion::Success),
                Some(70),
            ),
            check("test", CheckStatus::InProgress, None, Some(0)),
        ];
        state.data.prs.insert(pr.key.clone(), pr.clone());
        state.data.now = test_time() + TimeDelta::seconds(5);
        let target = pr_target(&state.data, &pr.key).unwrap();
        let areas = layout(state.data.size);
        let keys = Keymap::default();
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &keys,
        };
        let PaneModel::Table { columns, rows } = Checks.model(
            &cx,
            &PaneState::default(),
            Size {
                width: 80,
                height: 20,
            },
        ) else {
            panic!("not a table");
        };
        assert_eq!(columns.len(), 3);
        let texts: Vec<Vec<String>> = rows
            .iter()
            .map(|row| row.cells.iter().map(|c| c.text.clone()).collect())
            .collect();
        assert_eq!(
            texts,
            [
                ["build", "✓ success", "1m 10s"],
                ["test", "● running", "5s"]
            ]
        );
        let o = KeyChord {
            code: KeyCode::Char('o'),
            mods: KeyModifiers::NONE,
        };
        let mut ps = PaneState {
            cursor: 1,
            ..PaneState::default()
        };
        assert_eq!(
            Checks.handle_key(o, &cx, &mut ps).map(|h| h.cmds),
            Some(vec![Cmd::OpenUrl("https://ci/test".into())])
        );
        ps.cursor = 9;
        assert_eq!(Checks.handle_key(o, &cx, &mut ps), None);
    }
}
