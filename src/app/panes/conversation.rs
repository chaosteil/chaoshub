//! The conversation pane of the pull request popup: issue comments,
//! reviews and review threads in time order. A thread header shows
//! `path:line · 3 comments · resolved`; Enter expands and collapses it.
//! Comment bodies are markdown cells, which `ui` renders and wraps.

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::app::age::age;
use crate::app::models::{Cell, CellStyle, PaneModel, TreeNode};
use crate::app::panes::{Pane, PaneState};
use crate::app::tabs::cells::cell;
use crate::app::tabs::{Handled, KeyCx};
use crate::bus::{KeyChord, Size, Target};
use crate::domain::detail::{Comment, PrDetail, Review, ReviewState, Thread};

/// The conversation pane.
pub(crate) struct Conversation;

/// The own state of the pane: the expanded threads, by their index in
/// `PrDetail.threads`.
#[derive(Debug, Default)]
pub(crate) struct ConversationState {
    pub expanded: HashSet<usize>,
}

impl Pane for Conversation {
    fn id(&self) -> &'static str {
        "conversation"
    }

    fn title(&self) -> &'static str {
        "Conversation"
    }

    fn key(&self) -> char {
        '2'
    }

    fn applies(&self, target: &Target) -> bool {
        matches!(target, Target::Pr { .. })
    }

    fn init_state(&self) -> PaneState {
        PaneState {
            ext: Box::new(ConversationState::default()),
            ..PaneState::default()
        }
    }

    fn model(&self, cx: &KeyCx<'_>, ps: &PaneState, _size: Size) -> PaneModel {
        let expanded = expanded(ps);
        PaneModel::Tree {
            nodes: nodes(cx, &expanded)
                .into_iter()
                .map(|(node, _)| node)
                .collect(),
        }
    }

    fn handle_key(&self, key: KeyChord, cx: &KeyCx<'_>, ps: &mut PaneState) -> Option<Handled> {
        if !cx.keys.matches("open", key) {
            return None;
        }
        let thread = {
            let expanded = expanded(ps);
            nodes(cx, &expanded).get(ps.cursor)?.1?
        };
        let state = ps.ext.downcast_mut::<ConversationState>()?;
        if !state.expanded.remove(&thread) {
            state.expanded.insert(thread);
        }
        Some(Handled {
            changed: true,
            ..Handled::default()
        })
    }
}

/// The expanded threads of a pane state.
fn expanded(ps: &PaneState) -> HashSet<usize> {
    ps.ext
        .downcast_ref::<ConversationState>()
        .map(|state| state.expanded.clone())
        .unwrap_or_default()
}

/// One entry of the conversation.
enum Entry<'a> {
    Comment(&'a Comment),
    Review(&'a Review),
    /// A thread and its index in `PrDetail.threads`.
    Thread(usize, &'a Thread),
}

impl Entry<'_> {
    /// The time of the entry. A review without a time and a thread
    /// without comments sort first.
    fn time(&self) -> Option<DateTime<Utc>> {
        match self {
            Entry::Comment(comment) => Some(comment.created_at),
            Entry::Review(review) => review.submitted_at,
            Entry::Thread(_, thread) => thread.comments.first().map(|c| c.created_at),
        }
    }
}

/// The entries of a conversation in time order. Pending reviews and
/// comment-only reviews without a body are left out: their comments
/// show in the threads.
fn entries(detail: &PrDetail) -> Vec<Entry<'_>> {
    let mut entries: Vec<Entry<'_>> = detail.comments.iter().map(Entry::Comment).collect();
    entries.extend(
        detail
            .reviews
            .iter()
            .filter(|r| r.state != ReviewState::Pending)
            .filter(|r| r.state != ReviewState::Commented || !r.body.trim().is_empty())
            .map(Entry::Review),
    );
    entries.extend(
        detail
            .threads
            .iter()
            .enumerate()
            .map(|(i, t)| Entry::Thread(i, t)),
    );
    entries.sort_by_key(Entry::time);
    entries
}

/// A node at `depth` with `cells`.
fn node(depth: u8, expanded: Option<bool>, cells: Vec<Cell>) -> TreeNode {
    TreeNode {
        depth,
        expanded,
        cells,
    }
}

/// The body of a comment or a review as a markdown node, if it has text.
fn body(depth: u8, text: &str) -> Option<TreeNode> {
    let text = text.trim();
    (!text.is_empty()).then(|| node(depth, None, vec![cell(text, CellStyle::Markdown)]))
}

/// The nodes of the pane, each with the index of the thread whose header
/// it is.
fn nodes(cx: &KeyCx<'_>, expanded: &HashSet<usize>) -> Vec<(TreeNode, Option<usize>)> {
    let Some(Target::Pr { pr, .. }) = cx.target else {
        return Vec::new();
    };
    let now = cx.data.now;
    let mut out: Vec<(TreeNode, Option<usize>)> = Vec::new();
    let mut push = |node: TreeNode| out.push((node, None));
    if let Some(reason) = cx.data.detail_errors.get(&pr.key) {
        push(node(
            0,
            None,
            vec![cell(
                format!("cannot load the conversation: {reason}"),
                CellStyle::Danger,
            )],
        ));
    }
    let Some(detail) = cx.data.details.get(&pr.key) else {
        push(node(
            0,
            None,
            vec![cell("loading the conversation…", CellStyle::Dim)],
        ));
        return out;
    };
    let entries = entries(&detail.value);
    if entries.is_empty() {
        push(node(0, None, vec![cell("no comments yet", CellStyle::Dim)]));
    }
    for entry in entries {
        match entry {
            Entry::Comment(comment) => {
                out.push((comment_header(0, comment, "commented", now), None));
                out.extend(body(1, &comment.body).map(|n| (n, None)));
            }
            Entry::Review(review) => {
                out.push((review_header(review, now), None));
                out.extend(body(1, &review.body).map(|n| (n, None)));
            }
            Entry::Thread(index, thread) => {
                let open = expanded.contains(&index);
                out.push((thread_header(thread, open), Some(index)));
                if open {
                    for comment in &thread.comments {
                        out.push((comment_header(1, comment, "", now), None));
                        out.extend(body(2, &comment.body).map(|n| (n, None)));
                    }
                }
            }
        }
    }
    out
}

/// `author · <verb> 2h ago`.
fn comment_header(depth: u8, comment: &Comment, verb: &str, now: DateTime<Utc>) -> TreeNode {
    let when = age(comment.created_at, now);
    let meta = if verb.is_empty() {
        format!(" · {when} ago")
    } else {
        format!(" · {verb} {when} ago")
    };
    node(
        depth,
        None,
        vec![
            cell(comment.author.clone(), CellStyle::Info),
            cell(meta, CellStyle::Dim),
        ],
    )
}

/// `author · approved 2h ago`, in the color of the review state.
fn review_header(review: &Review, now: DateTime<Utc>) -> TreeNode {
    let (verb, style) = match review.state {
        ReviewState::Approved => ("approved", CellStyle::Success),
        ReviewState::ChangesRequested => ("requested changes", CellStyle::Danger),
        ReviewState::Commented => ("reviewed", CellStyle::Dim),
        ReviewState::Dismissed => ("review dismissed", CellStyle::Dim),
        ReviewState::Pending => ("review pending", CellStyle::Dim),
    };
    let when = review
        .submitted_at
        .map(|at| format!(" {} ago", age(at, now)))
        .unwrap_or_default();
    node(
        0,
        None,
        vec![
            cell(review.author.clone(), CellStyle::Info),
            cell(format!(" · {verb}{when}"), style),
        ],
    )
}

/// `path:line · 3 comments · resolved`. A resolved thread is grey.
fn thread_header(thread: &Thread, open: bool) -> TreeNode {
    let place = match thread.line {
        Some(line) => format!("{}:{line}", thread.path),
        None => thread.path.clone(),
    };
    let count = thread.comments.len();
    let noun = if count == 1 { "comment" } else { "comments" };
    let (resolved, style) = if thread.is_resolved {
        (" · resolved", CellStyle::Dim)
    } else {
        ("", CellStyle::Normal)
    };
    node(
        0,
        Some(open),
        vec![cell(format!("{place} · {count} {noun}{resolved}"), style)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::keymap::Keymap;
    use crate::app::layout::layout;
    use crate::app::state::AppState;
    use crate::app::target::pr_target;
    use crate::app::testdata;
    use crate::bus::{KeyCode, KeyModifiers};
    use crate::domain::test_time;

    /// The text of each node, with its depth and expand marker.
    fn outline(model: &PaneModel) -> Vec<String> {
        let PaneModel::Tree { nodes } = model else {
            panic!("not a tree");
        };
        nodes
            .iter()
            .map(|n| {
                let marker = match n.expanded {
                    Some(true) => "▾ ",
                    Some(false) => "▸ ",
                    None => "",
                };
                let text: String = n.cells.iter().map(|c| c.text.as_str()).collect();
                format!("{}{marker}{text}", "  ".repeat(usize::from(n.depth)))
            })
            .collect()
    }

    fn state_with(detail: Option<PrDetail>, error: Option<&str>) -> AppState {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        state.data.now = test_time() + chrono::TimeDelta::hours(3);
        let pr = testdata::pr("chaoshub", 7, "Seven");
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
        state.data.prs.insert(pr.key.clone(), pr);
        state
    }

    fn model_and_keys(state: &AppState, ps: &mut PaneState, keys: &[KeyCode]) -> PaneModel {
        let key = testdata::pr_key("chaoshub", 7);
        let target = pr_target(&state.data, &key).unwrap();
        let areas = layout(state.data.size);
        let keymap = Keymap::default();
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &keymap,
        };
        for &code in keys {
            let chord = KeyChord {
                code,
                mods: KeyModifiers::NONE,
            };
            Conversation.handle_key(chord, &cx, ps);
        }
        Conversation.model(
            &cx,
            ps,
            Size {
                width: 80,
                height: 20,
            },
        )
    }

    #[test]
    fn comments_reviews_and_threads_in_time_order() {
        let state = state_with(Some(testdata::detail()), None);
        let mut ps = Conversation.init_state();
        let model = model_and_keys(&state, &mut ps, &[]);
        assert_eq!(
            outline(&model),
            [
                "alice · commented 3h ago",
                "  Looks good, one question about `layout`.",
                "▸ src/ui/list.rs:42 · 2 comments · resolved",
                "▸ src/ui/pane.rs:7 · 1 comment",
                "bob · requested changes 1h ago",
                "  Please split the **transition** module.",
            ]
        );
        let PaneModel::Tree { nodes } = &model else {
            unreachable!()
        };
        assert_eq!(nodes[1].cells[0].style, CellStyle::Markdown);
        assert_eq!(nodes[4].cells[1].style, CellStyle::Danger);
    }

    #[test]
    fn enter_expands_and_collapses_a_thread() {
        let state = state_with(Some(testdata::detail()), None);
        let mut ps = Conversation.init_state();
        ps.cursor = 3;
        let model = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        assert_eq!(
            outline(&model)[3..6],
            [
                "▾ src/ui/pane.rs:7 · 1 comment",
                "  bob · 2h ago",
                "    The gutter eats one column.",
            ]
        );
        let model = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        assert_eq!(outline(&model)[3], "▸ src/ui/pane.rs:7 · 1 comment");
        ps.cursor = 0;
        let key = testdata::pr_key("chaoshub", 7);
        let target = pr_target(&state.data, &key).unwrap();
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let enter = KeyChord {
            code: KeyCode::Enter,
            mods: KeyModifiers::NONE,
        };
        assert_eq!(
            Conversation.handle_key(enter, &cx, &mut ps),
            None,
            "Enter on a comment does nothing"
        );
    }

    #[test]
    fn loading_empty_and_error_states() {
        let mut ps = Conversation.init_state();
        let loading = state_with(None, None);
        assert_eq!(
            outline(&model_and_keys(&loading, &mut ps, &[])),
            ["loading the conversation…"]
        );
        let empty = PrDetail {
            comments: Vec::new(),
            reviews: Vec::new(),
            threads: Vec::new(),
            head_oid: "oid7".into(),
        };
        let empty = state_with(Some(empty), Some("offline"));
        assert_eq!(
            outline(&model_and_keys(&empty, &mut ps, &[])),
            ["cannot load the conversation: offline", "no comments yet"]
        );
    }
}
