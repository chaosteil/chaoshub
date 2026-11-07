//! The stack pane of the pull request popup: the entries of the stack by
//! position, the current pull request highlighted. The cursor starts on
//! the current pull request. Enter opens the popup of the entry under the
//! cursor, and `o` opens its page.

use crate::app::models::{Cell, CellStyle, PaneModel, TreeNode};
use crate::app::panes::{Pane, PaneState};
use crate::app::tabs::cells::{self, cell};
use crate::app::tabs::{Handled, KeyCx};
use crate::bus::{Cmd, KeyChord, Size, Target, TargetKey};
use crate::domain::pr::PullRequest;

/// The stack pane.
pub(crate) struct StackPane;

impl Pane for StackPane {
    fn id(&self) -> &'static str {
        "stack"
    }

    fn title(&self) -> &'static str {
        "Stack"
    }

    fn key(&self) -> char {
        '4'
    }

    fn applies(&self, target: &Target) -> bool {
        matches!(target, Target::Pr { pr, .. } if pr.head.value.stack.is_some())
    }

    fn init_state(&self) -> PaneState {
        PaneState::default()
    }

    /// The cursor goes to the node of the current pull request.
    fn place(&self, target: &Target, ps: &mut PaneState) {
        let Target::Pr { pr, .. } = target;
        if let Some(info) = &pr.head.value.stack {
            ps.cursor = usize::try_from(info.position).unwrap_or(0);
        }
    }

    fn model(&self, cx: &KeyCx<'_>, _ps: &PaneState, _size: Size) -> PaneModel {
        let mut nodes: Vec<TreeNode> = Vec::new();
        let Some(Target::Pr { pr, stack, .. }) = cx.target else {
            return PaneModel::Tree { nodes };
        };
        let Some(info) = &pr.head.value.stack else {
            return PaneModel::Tree { nodes };
        };
        let noun = if info.size == 1 { "PR" } else { "PRs" };
        nodes.push(node(vec![cell(
            format!(
                "stack #{} -> {} · {} {noun} · position 1 merges first",
                info.number, info.base_ref, info.size
            ),
            CellStyle::Dim,
        )]));
        for position in 1..=info.size.max(info.position) {
            let Some(entry) = entry(stack, position) else {
                nodes.push(node(vec![
                    cell(format!("  {position} "), CellStyle::Dim),
                    cell("not loaded", CellStyle::Dim),
                ]));
                continue;
            };
            let current = entry.key == pr.key;
            let (marker, title_style) = if current {
                ("› ", CellStyle::Primary)
            } else {
                ("  ", CellStyle::Normal)
            };
            nodes.push(node(vec![
                cell(format!("{marker}{position} "), CellStyle::Dim),
                cell(format!("#{} ", entry.key.number), CellStyle::Primary),
                cell(entry.head.value.title.clone(), title_style),
                cell("  ", CellStyle::Dim),
                cells::ci(entry.ci_state.value),
                cell(" ", CellStyle::Dim),
                cells::review(entry.head.value.review_decision),
                cell(" ", CellStyle::Dim),
                cells::flags(entry, cx.data.pane_open(&TargetKey::Pr(entry.key.clone()))),
            ]));
        }
        PaneModel::Tree { nodes }
    }

    /// Enter opens the popup of the entry under the cursor. `o` opens the
    /// page of that entry. On the summary node and on an entry that is not
    /// loaded, Enter does nothing and `o` goes to the actions.
    fn handle_key(&self, key: KeyChord, cx: &KeyCx<'_>, ps: &mut PaneState) -> Option<Handled> {
        let open = cx.keys.matches("open", key);
        if !open && !cx.keys.matches("open_url", key) {
            return None;
        }
        let Some(Target::Pr { stack, .. }) = cx.target else {
            return None;
        };
        let Some(entry) = u32::try_from(ps.cursor)
            .ok()
            .and_then(|position| entry(stack, position))
        else {
            return open.then(Handled::default);
        };
        Some(if open {
            Handled {
                open_pr: Some(entry.key.clone()),
                ..Handled::default()
            }
        } else {
            Handled {
                cmds: vec![Cmd::OpenUrl(entry.head.value.url.clone())],
                ..Handled::default()
            }
        })
    }
}

/// A node of the pane: a leaf at depth 0.
fn node(cells: Vec<Cell>) -> TreeNode {
    TreeNode {
        depth: 0,
        expanded: None,
        cells,
    }
}

/// The entry of `stack` at `position`. Position 0 is the summary node, so
/// it has no entry.
fn entry(stack: &[PullRequest], position: u32) -> Option<&PullRequest> {
    stack.iter().find(|entry| {
        entry
            .head
            .value
            .stack
            .as_ref()
            .is_some_and(|s| s.position == position)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::layout::layout;
    use crate::app::state::AppState;
    use crate::app::target::pr_target;
    use crate::app::testdata;
    use crate::bus::{KeyCode, KeyModifiers};

    fn state() -> AppState {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        state.load(testdata::mine_snapshot(), Vec::new());
        state
    }

    fn lines_for(state: &AppState, number: u64) -> Option<Vec<String>> {
        let target = pr_target(&state.data, &testdata::pr_key("chaoshub", number))?;
        if !StackPane.applies(&target) {
            return None;
        }
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let PaneModel::Tree { nodes } = StackPane.model(
            &cx,
            &PaneState::default(),
            Size {
                width: 80,
                height: 20,
            },
        ) else {
            panic!("not a tree");
        };
        Some(
            nodes
                .iter()
                .map(|node| node.cells.iter().map(|c| c.text.as_str()).collect())
                .collect(),
        )
    }

    /// The result of `code` on the node `cursor` of the stack pane of
    /// #`number`.
    fn key_for(state: &AppState, number: u64, cursor: usize, code: KeyCode) -> Option<Handled> {
        let target = pr_target(&state.data, &testdata::pr_key("chaoshub", number)).unwrap();
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let mut ps = PaneState {
            cursor,
            ..PaneState::default()
        };
        let key = KeyChord {
            code,
            mods: KeyModifiers::NONE,
        };
        StackPane.handle_key(key, &cx, &mut ps)
    }

    #[test]
    fn entries_by_position_with_the_current_one_marked() {
        let mut state = state();
        assert_eq!(
            lines_for(&state, 102).unwrap(),
            [
                "stack #12 -> main · 3 PRs · position 1 merges first",
                "  1 #101 feat(app): Pane trait, registry and overlay stack  ✗ ✗ C",
                "› 2 #102 feat(app): checks pane  ✓ ✓ ",
                "  3 #103 feat(ui): popup frame with pane tabs  ● ○ D",
            ]
        );
        state.data.prs.remove(&testdata::pr_key("chaoshub", 101));
        assert_eq!(lines_for(&state, 102).unwrap()[1], "  1 not loaded");
        assert_eq!(
            lines_for(&state, 99),
            None,
            "a single pull request has no stack pane"
        );
    }

    #[test]
    fn the_cursor_starts_on_the_current_entry() {
        let state = state();
        let target = pr_target(&state.data, &testdata::pr_key("chaoshub", 102)).unwrap();
        let mut ps = StackPane.init_state();
        StackPane.place(&target, &mut ps);
        assert_eq!(ps.cursor, 2);
    }

    #[test]
    fn keys_on_the_nodes() {
        let mut state = state();
        let url = |number: u64| {
            state.data.prs[&testdata::pr_key("chaoshub", number)]
                .head
                .value
                .url
                .clone()
        };
        let cases = [
            (
                "enter on an entry opens it",
                3,
                KeyCode::Enter,
                Some(Handled {
                    open_pr: Some(testdata::pr_key("chaoshub", 103)),
                    ..Handled::default()
                }),
            ),
            (
                "enter on the current entry opens it",
                2,
                KeyCode::Enter,
                Some(Handled {
                    open_pr: Some(testdata::pr_key("chaoshub", 102)),
                    ..Handled::default()
                }),
            ),
            (
                "enter on the summary does nothing",
                0,
                KeyCode::Enter,
                Some(Handled::default()),
            ),
            (
                "o on an entry opens its page",
                1,
                KeyCode::Char('o'),
                Some(Handled {
                    cmds: vec![Cmd::OpenUrl(url(101))],
                    ..Handled::default()
                }),
            ),
            (
                "o on the summary goes to the actions",
                0,
                KeyCode::Char('o'),
                None,
            ),
            ("another key goes on", 1, KeyCode::Char('x'), None),
        ];
        for (name, cursor, code, want) in cases {
            assert_eq!(key_for(&state, 102, cursor, code), want, "{name}");
        }
        state.data.prs.remove(&testdata::pr_key("chaoshub", 101));
        assert_eq!(
            key_for(&state, 102, 1, KeyCode::Enter),
            Some(Handled::default()),
            "enter on an entry that is not loaded does nothing"
        );
    }
}
