//! The conversation pane of the pull request popup: issue comments,
//! reviews and review threads in time order. A thread header shows
//! `path:line · 3 comments · resolved`. Enter on a thread header expands
//! and collapses the thread.
//! Comment bodies are markdown cells, which `ui` renders and wraps.
//! The reactions of a comment or a review show on one row under its
//! body: `👍 3  🎉 1`. The reactions of the viewer have the primary
//! color.
//!
//! Enter on a comment, a review or a comment in an expanded thread opens
//! the comment menu: reply, the eight reactions of GitHub (keys `1` to
//! `8`), and in a thread, collapse the thread. A reaction that the viewer
//! already has shows a `✓`, and the menu removes it again, as GitHub
//! does.
//!
//! The reply key (`r`) opens a text box for the comment of the cursor.
//! On a thread or a comment in a thread, the reply goes into the thread.
//! On an issue comment or a review, the reply is a new comment on the
//! pull request that starts with a quote of the author and the first
//! line. On a line without a comment, the reply is a new comment without
//! a quote.

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::actions::REPLY_ID;
use crate::app::age::age;
use crate::app::models::{Cell, CellStyle, PaneModel, TreeNode};
use crate::app::panes::{Pane, PaneAction, PaneMenu, PaneState};
use crate::app::tabs::cells::cell;
use crate::app::tabs::{Handled, KeyCx, tree_toggled};
use crate::bus::{
    KeyChord, KeyCode, KeyModifiers, PendingPart, ReactSpec, ReplySpec, ReplyTo, Size, Target,
};
use crate::domain::detail::{
    Comment, PrDetail, Reaction, ReactionKind, Review, ReviewState, Thread, has_mine, set_mine,
};
use crate::domain::plural;
use crate::domain::pr::PullRequest;

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

    fn short_title(&self) -> &'static str {
        "Convo"
    }

    fn key(&self) -> char {
        '3'
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

    /// The reply key opens the text box of a reply. Enter on a thread
    /// header expands or collapses the thread. Enter on a comment or a
    /// review opens the comment menu.
    fn handle_key(&self, key: KeyChord, cx: &KeyCx<'_>, ps: &mut PaneState) -> Option<Handled> {
        if cx.keys.matches(REPLY_ID, key) {
            return reply(cx, ps);
        }
        if !cx.keys.matches("open", key) {
            return None;
        }
        match cursor_item(cx, ps)? {
            Item::Thread(thread) => toggle(cx, ps, thread),
            item => Some(Handled {
                menu: Some(menu(cx, item)?),
                ..Handled::default()
            }),
        }
    }

    fn hints(&self) -> &'static [(&'static str, &'static str)] {
        &[(REPLY_ID, "reply")]
    }

    fn open_hint(&self) -> &'static str {
        "expand/react"
    }

    /// The reply, on the first chord of the reply key.
    fn actions(&self, cx: &KeyCx<'_>, _ps: &PaneState) -> Vec<PaneAction> {
        cx.keys
            .chords(REPLY_ID)
            .first()
            .map(|&key| PaneAction {
                key,
                label: "reply to the comment".to_owned(),
            })
            .into_iter()
            .collect()
    }

    /// The reply, and the items of the comment menu: a reaction key and
    /// the collapse key.
    fn run_action(&self, key: KeyChord, cx: &KeyCx<'_>, ps: &mut PaneState) -> Option<Handled> {
        if cx.keys.matches(REPLY_ID, key) {
            return reply(cx, ps);
        }
        if key == char_key(COLLAPSE_KEY) {
            let Item::ThreadComment(thread, _) = cursor_item(cx, ps)? else {
                return None;
            };
            return collapse(cx, ps, thread);
        }
        let kind = REACTION_KEYS
            .iter()
            .position(|&c| key == char_key(c))
            .map(|i| ReactionKind::ALL[i])?;
        react(cx, ps, kind)
    }
}

/// The keys of the reactions in the comment menu, in the order of
/// `ReactionKind::ALL`.
const REACTION_KEYS: [char; 8] = ['1', '2', '3', '4', '5', '6', '7', '8'];

/// The key of the comment menu that collapses the thread of the comment.
const COLLAPSE_KEY: char = 'c';

/// The chord of the character `c` without modifiers.
fn char_key(c: char) -> KeyChord {
    KeyChord {
        code: KeyCode::Char(c),
        mods: KeyModifiers::NONE,
    }
}

/// The item of the cursor node. `None` on a status line.
fn cursor_item(cx: &KeyCx<'_>, ps: &PaneState) -> Option<Item> {
    nodes(cx, &expanded(ps)).get(ps.cursor)?.1
}

/// Expands or collapses the thread at `thread`.
fn toggle(cx: &KeyCx<'_>, ps: &mut PaneState, thread: usize) -> Option<Handled> {
    let before = nodes(cx, &expanded(ps)).len();
    let state = ps.ext.downcast_mut::<ConversationState>()?;
    if !state.expanded.remove(&thread) {
        state.expanded.insert(thread);
    }
    let after = nodes(cx, &expanded(ps)).len();
    Some(tree_toggled(ps.cursor, before, after))
}

/// Collapses the expanded thread at `thread` from one of its comments.
/// The cursor moves to the thread header.
fn collapse(cx: &KeyCx<'_>, ps: &mut PaneState, thread: usize) -> Option<Handled> {
    let header = nodes(cx, &expanded(ps))
        .iter()
        .position(|(_, item)| *item == Some(Item::Thread(thread)))?;
    let state = ps.ext.downcast_mut::<ConversationState>()?;
    state.expanded.remove(&thread);
    ps.cursor = header;
    Some(tree_toggled(header, 1, 0))
}

/// The comment or the review of `item` in `detail`: its node id, its
/// author and its reactions. A thread header has none.
fn subject(detail: &PrDetail, item: Item) -> Option<(&str, &str, &[Reaction])> {
    match item {
        Item::Comment(index) => detail
            .comments
            .get(index)
            .map(|c| (c.id.as_str(), c.author.as_str(), c.reactions.as_slice())),
        Item::Review(index) => detail
            .reviews
            .get(index)
            .map(|r| (r.id.as_str(), r.author.as_str(), r.reactions.as_slice())),
        Item::ThreadComment(thread, index) => detail
            .threads
            .get(thread)
            .and_then(|t| t.comments.get(index))
            .map(|c| (c.id.as_str(), c.author.as_str(), c.reactions.as_slice())),
        Item::Thread(_) => None,
    }
}

/// The reactions of the comment or the review `id` as the viewer sees
/// them: the GitHub data with the pending reactions of the viewer (see
/// `app::pending`). A pending reaction shows before GitHub has it.
fn shown(cx: &KeyCx<'_>, id: &str, reactions: &[Reaction]) -> Vec<Reaction> {
    let mut shown = reactions.to_vec();
    if let Some(Target::Pr { pr, .. }) = cx.target {
        for (subject, kind, add) in cx.data.pending.reactions(&pr.key) {
            if subject == id {
                set_mine(&mut shown, kind, add);
            }
        }
    }
    shown
}

/// The detail of the pull request of the popup.
fn detail<'a>(cx: &KeyCx<'a>) -> Option<&'a PrDetail> {
    let Some(Target::Pr { pr, .. }) = cx.target else {
        return None;
    };
    cx.data.details.get(&pr.key).map(|d| &d.value)
}

/// The comment menu of `item`: reply, a gap, the eight reactions with a
/// `✓` on the reactions of the viewer, and in a thread a gap and
/// collapse. `None` without a comment or a review.
fn menu(cx: &KeyCx<'_>, item: Item) -> Option<PaneMenu> {
    let detail = detail(cx)?;
    let (id, author, reactions) = subject(detail, item)?;
    let reactions = shown(cx, id, reactions);
    let title = match item {
        Item::Review(_) => format!("review of {author}"),
        _ => format!("comment of {author}"),
    };
    let mut items: Vec<PaneAction> = cx
        .keys
        .chords(REPLY_ID)
        .first()
        .map(|&key| PaneAction {
            key,
            label: "reply".to_owned(),
        })
        .into_iter()
        .collect();
    let mut gaps = Vec::new();
    if !items.is_empty() {
        gaps.push(items.len());
    }
    items.extend(
        ReactionKind::ALL
            .iter()
            .zip(REACTION_KEYS)
            .map(|(&kind, key)| {
                let mine = if has_mine(&reactions, kind) {
                    " ✓"
                } else {
                    ""
                };
                PaneAction::char(key, format!("{} {}{mine}", kind.emoji(), kind.name()))
            }),
    );
    if matches!(item, Item::ThreadComment(..)) {
        gaps.push(items.len());
        items.push(PaneAction::char(COLLAPSE_KEY, "collapse the thread"));
    }
    Some(PaneMenu { title, items, gaps })
}

/// The reaction `kind` of the viewer on the comment of the cursor: an
/// add, or a remove when the viewer already has it.
fn react(cx: &KeyCx<'_>, ps: &PaneState, kind: ReactionKind) -> Option<Handled> {
    let item = cursor_item(cx, ps)?;
    let (id, _, reactions) = subject(detail(cx)?, item)?;
    let reactions = shown(cx, id, reactions);
    Some(Handled {
        react: Some(ReactSpec {
            subject: id.to_owned(),
            kind,
            add: !has_mine(&reactions, kind),
        }),
        ..Handled::default()
    })
}

/// The text box of a reply to the item of the cursor.
fn reply(cx: &KeyCx<'_>, ps: &PaneState) -> Option<Handled> {
    let Some(Target::Pr { pr, .. }) = cx.target else {
        return None;
    };
    let item = nodes(cx, &expanded(ps)).get(ps.cursor).and_then(|n| n.1);
    let detail = cx.data.details.get(&pr.key).map(|d| &d.value);
    let spec = reply_spec(detail, item, pr.key.number);
    Some(Handled {
        reply: Some(spec),
        ..Handled::default()
    })
}

/// The reply to `item` of `detail`, on the pull request `number`. A
/// thread, or a comment in a thread, gets a reply in the thread. An
/// issue comment or a review gets a new comment on the pull request,
/// with a quote. No item gets a new comment without a quote.
fn reply_spec(detail: Option<&PrDetail>, item: Option<Item>, number: u64) -> ReplySpec {
    let new_comment = |text: String| ReplySpec {
        to: ReplyTo::Pr,
        title: format!("comment on #{number}"),
        text,
    };
    let Some(detail) = detail else {
        return new_comment(String::new());
    };
    match item {
        Some(Item::Thread(index) | Item::ThreadComment(index, _)) => {
            match detail.threads.get(index) {
                Some(thread) => {
                    let place = place(thread);
                    ReplySpec {
                        title: format!("reply on {place}"),
                        to: ReplyTo::Thread {
                            id: thread.id.clone(),
                            place,
                        },
                        text: String::new(),
                    }
                }
                None => new_comment(String::new()),
            }
        }
        Some(Item::Comment(index)) => detail.comments.get(index).map_or_else(
            || new_comment(String::new()),
            |comment| new_comment(quote(&comment.author, &comment.body)),
        ),
        Some(Item::Review(index)) => detail.reviews.get(index).map_or_else(
            || new_comment(String::new()),
            |review| new_comment(quote(&review.author, &review.body)),
        ),
        None => new_comment(String::new()),
    }
}

/// The start text of a reply to a comment of `author` with `body`: a
/// markdown quote of the author and the first line with text, then an
/// empty line. A body without text gives a mention of the author.
fn quote(author: &str, body: &str) -> String {
    match body.lines().map(str::trim).find(|line| !line.is_empty()) {
        Some(line) => format!("> @{author}: {line}\n\n"),
        None => format!("@{author} "),
    }
}

/// The expanded threads of a pane state.
fn expanded(ps: &PaneState) -> HashSet<usize> {
    ps.ext
        .downcast_ref::<ConversationState>()
        .map(|state| state.expanded.clone())
        .unwrap_or_default()
}

/// One entry of the conversation, with its index in `PrDetail`.
enum Entry<'a> {
    Comment(usize, &'a Comment),
    Review(usize, &'a Review),
    Thread(usize, &'a Thread),
}

/// The entry of a node: the index of an issue comment, a review or a
/// thread in `PrDetail`. The nodes of a thread are the header and the
/// comments of the thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Comment(usize),
    Review(usize),
    /// The header of a thread.
    Thread(usize),
    /// A comment in an expanded thread: the index of the thread and the
    /// index of the comment in the thread.
    ThreadComment(usize, usize),
}

impl Entry<'_> {
    /// The time of the entry. A review without a time and a thread
    /// without comments sort first.
    fn time(&self) -> Option<DateTime<Utc>> {
        match self {
            Entry::Comment(_, comment) => Some(comment.created_at),
            Entry::Review(_, review) => review.submitted_at,
            Entry::Thread(_, thread) => thread.comments.first().map(|c| c.created_at),
        }
    }
}

/// The entries of a conversation in time order. Pending reviews and
/// comment-only reviews without a body are left out: their comments
/// show in the threads.
fn entries(detail: &PrDetail) -> Vec<Entry<'_>> {
    let mut entries: Vec<Entry<'_>> = detail
        .comments
        .iter()
        .enumerate()
        .map(|(i, c)| Entry::Comment(i, c))
        .collect();
    entries.extend(
        detail
            .reviews
            .iter()
            .enumerate()
            .filter(|(_, r)| r.state != ReviewState::Pending)
            .filter(|(_, r)| r.state != ReviewState::Commented || !r.body.trim().is_empty())
            .map(|(i, r)| Entry::Review(i, r)),
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

/// The body of a comment or a review as a node, if it has text. The node
/// has the HTML of GitHub when it is not empty, else the markdown.
fn body(depth: u8, text: &str, html: &str) -> Option<TreeNode> {
    let html = html.trim();
    if !html.is_empty() {
        return Some(node(depth, None, vec![cell(html, CellStyle::Html)]));
    }
    let text = text.trim();
    (!text.is_empty()).then(|| node(depth, None, vec![cell(text, CellStyle::Markdown)]))
}

/// The reactions of a comment or a review as a node: `👍 3  🎉 1`, in
/// the order of GitHub. The reactions of the viewer have the primary
/// color, the others are dim. A reaction of `pulses` pulses with its
/// mix: it is on its way to GitHub. A removed reaction that is on its
/// way and has no count left shows as `👍 0`. `None` without reactions.
fn reactions(depth: u8, reactions: &[Reaction], pulses: &[(ReactionKind, u8)]) -> Option<TreeNode> {
    let pulse = |kind: ReactionKind| {
        pulses
            .iter()
            .find(|(pending, _)| *pending == kind)
            .map(|&(_, mix)| mix)
    };
    let mut cells = Vec::new();
    let mut push = |text: String, style: CellStyle| {
        if !cells.is_empty() {
            cells.push(cell("  ", CellStyle::Normal));
        }
        cells.push(cell(text, style));
    };
    for reaction in reactions {
        let style = match pulse(reaction.kind) {
            Some(mix) => CellStyle::Pulse(mix),
            None if reaction.mine => CellStyle::Primary,
            None => CellStyle::Dim,
        };
        push(
            format!("{} {}", reaction.kind.emoji(), reaction.count),
            style,
        );
    }
    for &(kind, mix) in pulses {
        if !reactions.iter().any(|reaction| reaction.kind == kind) {
            push(format!("{} 0", kind.emoji()), CellStyle::Pulse(mix));
        }
    }
    (!cells.is_empty()).then(|| node(depth, None, cells))
}

/// The reactions node of the comment or the review `id`, as the viewer
/// sees it: with the pending reactions of the viewer, which pulse.
fn reaction_node(cx: &KeyCx<'_>, depth: u8, id: &str, raw: &[Reaction]) -> Option<TreeNode> {
    let shown = shown(cx, id, raw);
    let mut pulses = Vec::new();
    if let Some(Target::Pr { pr, .. }) = cx.target {
        for (subject, kind, _) in cx.data.pending.reactions(&pr.key) {
            let part = PendingPart::Reaction(subject.to_owned(), kind);
            if subject == id
                && let Some(mix) = cx.data.pending_pulse(&pr.key, &part)
            {
                pulses.push((kind, mix));
            }
        }
    }
    reactions(depth, &shown, &pulses)
}

/// The nodes of the pane, each with its entry. A status line has none.
/// The header of a thread has the item of the thread, and the nodes of
/// a comment in the thread have the item of the comment.
fn nodes(cx: &KeyCx<'_>, expanded: &HashSet<usize>) -> Vec<(TreeNode, Option<Item>)> {
    let Some(Target::Pr { pr, .. }) = cx.target else {
        return Vec::new();
    };
    let now = cx.data.now;
    let mut out: Vec<(TreeNode, Option<Item>)> = Vec::new();
    let mut push = |node: TreeNode| out.push((node, None));
    let error = cx.data.detail_errors.get(&pr.key);
    if let Some(reason) = error {
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
        // After a failed load, the error is the whole content.
        if error.is_none() {
            push(node(
                0,
                None,
                vec![cell("loading the conversation…", CellStyle::Dim)],
            ));
        }
        return out;
    };
    if is_stale(&detail.value, pr) {
        push(node(
            0,
            None,
            vec![cell(
                "stale: read at an older head, a new load is on its way",
                CellStyle::Warn,
            )],
        ));
    }
    let entries = entries(&detail.value);
    if entries.is_empty() {
        push(node(0, None, vec![cell("no comments yet", CellStyle::Dim)]));
    }
    for entry in entries {
        match entry {
            Entry::Comment(index, comment) => {
                let item = Some(Item::Comment(index));
                out.push((comment_header(0, comment, "commented", now), item));
                out.extend(body(1, &comment.body, &comment.body_html).map(|n| (n, item)));
                out.extend(
                    reaction_node(cx, 1, &comment.id, &comment.reactions).map(|n| (n, item)),
                );
            }
            Entry::Review(index, review) => {
                let item = Some(Item::Review(index));
                out.push((review_header(review, now), item));
                out.extend(body(1, &review.body, &review.body_html).map(|n| (n, item)));
                out.extend(reaction_node(cx, 1, &review.id, &review.reactions).map(|n| (n, item)));
            }
            Entry::Thread(index, thread) => {
                let open = expanded.contains(&index);
                let pulse = cx
                    .data
                    .pending_pulse(&pr.key, &PendingPart::Thread(thread.id.clone()));
                let header = thread_header(thread, open, pulse);
                out.push((header, Some(Item::Thread(index))));
                if open {
                    for (at, comment) in thread.comments.iter().enumerate() {
                        let item = Some(Item::ThreadComment(index, at));
                        out.push((comment_header(1, comment, "", now), item));
                        out.extend(body(2, &comment.body, &comment.body_html).map(|n| (n, item)));
                        let node = reaction_node(cx, 2, &comment.id, &comment.reactions);
                        out.extend(node.map(|n| (n, item)));
                    }
                }
            }
        }
    }
    // A new comment or a review on its way has no place in the data yet.
    // A pulsing line at the end stands for it.
    for (part, text) in [
        (PendingPart::Review, "sending your review…"),
        (PendingPart::Comment, "sending your comment…"),
    ] {
        if let Some(mix) = cx.data.pending_pulse(&pr.key, &part) {
            out.push((node(0, None, vec![cell(text, CellStyle::Pulse(mix))]), None));
        }
    }
    out
}

/// True when the conversation was read at another head than the pull
/// request has now.
pub(crate) fn is_stale(detail: &PrDetail, pr: &PullRequest) -> bool {
    detail.head_oid != pr.head.value.head_oid
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

/// The place of a thread: `path:line`, or the path alone for a thread
/// without a line.
fn place(thread: &Thread) -> String {
    match thread.line {
        Some(line) => format!("{}:{line}", thread.path),
        None => thread.path.clone(),
    }
}

/// `path:line · 3 comments · resolved`. A resolved thread is grey. While
/// a reply to the thread is on its way, the header pulses with the mix
/// `pulse`.
fn thread_header(thread: &Thread, open: bool, pulse: Option<u8>) -> TreeNode {
    let place = place(thread);
    let count = thread.comments.len();
    let noun = plural::noun(count, "comment", "comments");
    let (resolved, style) = if thread.is_resolved {
        (" · resolved", CellStyle::Dim)
    } else {
        ("", CellStyle::Normal)
    };
    let style = pulse.map_or(style, CellStyle::Pulse);
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
    use crate::bus::{KeyCode, KeyModifiers, UiEvent};
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
        let handled = Conversation.handle_key(enter, &cx, &mut ps).unwrap();
        assert!(handled.menu.is_some(), "Enter on a comment opens the menu");
        assert!(!handled.changed);
    }

    /// The pane context of `state` on #7, with the default keys.
    fn cx_of<'a>(
        state: &'a AppState,
        target: &'a Target,
        areas: &'a crate::app::layout::Areas,
    ) -> KeyCx<'a> {
        KeyCx {
            data: &state.data,
            target: Some(target),
            areas,
            keys: &crate::app::keymap::DEFAULT,
        }
    }

    fn chord(code: KeyCode) -> KeyChord {
        KeyChord {
            code,
            mods: KeyModifiers::NONE,
        }
    }

    /// The comment menu of Enter on the node `cursor` after the keys
    /// `keys`. `None` when Enter opens no menu.
    fn menu_at(state: &AppState, keys: &[KeyCode], cursor: usize) -> Option<PaneMenu> {
        let mut ps = Conversation.init_state();
        model_and_keys(state, &mut ps, keys);
        ps.cursor = cursor;
        let key = testdata::pr_key("chaoshub", 7);
        let target = pr_target(&state.data, &key).unwrap();
        let areas = layout(state.data.size);
        let cx = cx_of(state, &target, &areas);
        Conversation
            .handle_key(chord(KeyCode::Enter), &cx, &mut ps)
            .and_then(|h| h.menu)
    }

    /// The items of `menu` as `key  label` lines.
    fn lines(menu: &PaneMenu) -> Vec<String> {
        menu.items
            .iter()
            .map(|item| format!("{}  {}", crate::app::frame::key_text(item.key), item.label))
            .collect()
    }

    #[test]
    fn enter_on_a_comment_opens_the_comment_menu() {
        let state = state_with(Some(testdata::detail_with_reactions()), None);
        for cursor in 0..=2 {
            let menu = menu_at(&state, &[], cursor).unwrap();
            assert_eq!(menu.title, "comment of alice", "node {cursor}");
            assert_eq!(
                lines(&menu),
                [
                    "r  reply",
                    "1  👍 thumbs up ✓",
                    "2  👎 thumbs down",
                    "3  😄 laugh",
                    "4  🎉 hooray",
                    "5  😕 confused",
                    "6  ❤️ heart",
                    "7  🚀 rocket",
                    "8  👀 eyes",
                ],
                "only the reactions of the viewer have a ✓"
            );
            assert_eq!(menu.gaps, [1], "a gap after the reply");
        }
        // The nodes after alice: two threads, then bob and his body and
        // his reactions.
        let review = menu_at(&state, &[], 5).unwrap();
        assert_eq!(review.title, "review of bob");
        assert_eq!(review.items.len(), 9, "no collapse for a review");
        assert!(lines(&review).iter().all(|line| !line.ends_with('✓')));
    }

    #[test]
    fn enter_on_a_comment_in_a_thread_can_collapse_the_thread() {
        let state = state_with(Some(testdata::detail_with_reactions()), None);
        assert_eq!(menu_at(&state, &[], 3), None, "a thread header expands");
        // Node 3 is the thread on `src/ui/list.rs:42`. After the expand,
        // node 6 is the reactions of its first comment.
        let mut ps = Conversation.init_state();
        ps.cursor = 3;
        model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        let key = testdata::pr_key("chaoshub", 7);
        let target = pr_target(&state.data, &key).unwrap();
        let areas = layout(state.data.size);
        let cx = cx_of(&state, &target, &areas);
        ps.cursor = 6;
        let menu = Conversation
            .handle_key(chord(KeyCode::Enter), &cx, &mut ps)
            .and_then(|h| h.menu)
            .unwrap();
        assert_eq!(menu.title, "comment of bob");
        let lines = lines(&menu);
        assert_eq!(lines[8], "8  👀 eyes ✓");
        assert_eq!(lines[9], "c  collapse the thread");
        assert_eq!(menu.gaps, [1, 9]);
        let handled = Conversation
            .run_action(chord(KeyCode::Char('c')), &cx, &mut ps)
            .unwrap();
        assert!(handled.changed);
        assert_eq!(ps.cursor, 3, "the cursor goes to the thread header");
        let model = Conversation.model(
            &cx,
            &ps,
            Size {
                width: 80,
                height: 20,
            },
        );
        assert_eq!(
            outline(&model)[3],
            "▸ src/ui/list.rs:42 · 2 comments · resolved"
        );
        assert_eq!(
            Conversation.run_action(chord(KeyCode::Char('c')), &cx, &mut ps),
            None,
            "collapse needs a comment in a thread"
        );
    }

    #[test]
    fn enter_on_a_status_line_does_nothing() {
        let loading = state_with(None, None);
        assert_eq!(menu_at(&loading, &[], 0), None);
        let mut stale = testdata::detail();
        stale.head_oid = "an older head".into();
        let stale = state_with(Some(stale), None);
        assert_eq!(menu_at(&stale, &[], 0), None);
    }

    #[test]
    fn a_reaction_key_toggles_the_reaction_of_the_viewer() {
        let state = state_with(Some(testdata::detail_with_reactions()), None);
        let key = testdata::pr_key("chaoshub", 7);
        let target = pr_target(&state.data, &key).unwrap();
        let areas = layout(state.data.size);
        let cx = cx_of(&state, &target, &areas);
        let mut ps = Conversation.init_state();
        let react = |ps: &mut PaneState, c: char| {
            Conversation
                .run_action(chord(KeyCode::Char(c)), &cx, ps)
                .and_then(|h| h.react)
        };
        let spec = |subject: &str, kind, add| {
            Some(ReactSpec {
                subject: subject.into(),
                kind,
                add,
            })
        };
        assert_eq!(
            react(&mut ps, '1'),
            spec("IC_0", ReactionKind::ThumbsUp, false),
            "the viewer has 👍, so the key removes it"
        );
        assert_eq!(
            react(&mut ps, '4'),
            spec("IC_0", ReactionKind::Hooray, true),
            "🎉 of another person: the key adds the one of the viewer"
        );
        ps.cursor = 5;
        assert_eq!(
            react(&mut ps, '6'),
            spec("PRR_bob", ReactionKind::Heart, true)
        );
        ps.cursor = 3;
        assert_eq!(react(&mut ps, '1'), None, "a thread header has no reaction");
        assert_eq!(react(&mut ps, '9'), None, "no ninth reaction");
    }

    #[test]
    fn an_expanded_thread_unfolds_its_comments() {
        let state = state_with(Some(testdata::detail()), None);
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
        let mut ps = Conversation.init_state();
        ps.cursor = 3;
        let handled = Conversation.handle_key(enter, &cx, &mut ps).unwrap();
        assert_eq!(
            handled.events,
            [UiEvent::TreeExpanded {
                first_row: 4,
                rows: 2
            }]
        );
        let handled = Conversation.handle_key(enter, &cx, &mut ps).unwrap();
        assert!(handled.changed);
        assert_eq!(handled.events, [], "a collapse is instant");
    }

    /// The outline of `model`, with each pulsing cell in `«»`.
    fn pulses(model: &PaneModel) -> String {
        let PaneModel::Tree { nodes } = model else {
            panic!("not a tree");
        };
        nodes
            .iter()
            .map(|n| {
                let text: String = n
                    .cells
                    .iter()
                    .map(|c| match c.style {
                        CellStyle::Pulse(_) => format!("«{}»", c.text),
                        _ => c.text.clone(),
                    })
                    .collect();
                format!("{}{text}", "  ".repeat(usize::from(n.depth)))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_parts_on_their_way_pulse_in_the_conversation() {
        use crate::bus::Expect;
        let mut state = state_with(Some(testdata::detail_with_reactions()), None);
        let key = testdata::pr_key("chaoshub", 7);
        testdata::pend(
            &mut state,
            &key,
            vec![
                (PendingPart::Thread("PRRT_7".into()), Expect::NewComment),
                (
                    PendingPart::Reaction("IC_0".into(), ReactionKind::ThumbsUp),
                    Expect::Reaction(false),
                ),
                (
                    PendingPart::Reaction("IC_0".into(), ReactionKind::Rocket),
                    Expect::Reaction(true),
                ),
                (
                    PendingPart::Reaction("PRR_bob".into(), ReactionKind::Heart),
                    Expect::Reaction(true),
                ),
                (PendingPart::Comment, Expect::NewComment),
            ],
        );
        let mut ps = Conversation.init_state();
        ps.cursor = 3;
        let model = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        insta::assert_snapshot!(pulses(&model));
    }

    #[test]
    fn a_removed_reaction_on_its_way_shows_with_no_count() {
        use crate::bus::Expect;
        let mut state = state_with(Some(testdata::detail_with_reactions()), None);
        let key = testdata::pr_key("chaoshub", 7);
        let thread_comment = state.data.details[&key].value.threads[0].comments[0]
            .id
            .clone();
        testdata::pend(
            &mut state,
            &key,
            vec![(
                PendingPart::Reaction(thread_comment, ReactionKind::Eyes),
                Expect::Reaction(false),
            )],
        );
        let mut ps = Conversation.init_state();
        ps.cursor = 3;
        let model = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        let text = pulses(&model);
        assert!(text.contains("    «👀 0»"), "{text}");
    }

    #[test]
    fn the_reactions_show_under_the_body() {
        let state = state_with(Some(testdata::detail_with_reactions()), None);
        let mut ps = Conversation.init_state();
        ps.cursor = 3;
        let model = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        assert_eq!(
            outline(&model),
            [
                "alice · commented 3h ago",
                "  Looks good, one question about `layout`.",
                "  👍 3  🎉 1",
                "▾ src/ui/list.rs:42 · 2 comments · resolved",
                "  bob · 2h ago",
                "    Off by one?",
                "    👀 1",
                "  chaosteil · 2h ago",
                "    Fixed.",
                "▸ src/ui/pane.rs:7 · 1 comment",
                "bob · requested changes 1h ago",
                "  Please split the **transition** module.",
                "  ❤️ 2",
            ]
        );
        let PaneModel::Tree { nodes } = &model else {
            unreachable!()
        };
        let styles: Vec<CellStyle> = nodes[2].cells.iter().map(|c| c.style).collect();
        assert_eq!(
            styles,
            [CellStyle::Primary, CellStyle::Normal, CellStyle::Dim],
            "the reaction of the viewer has the primary color"
        );
        assert_eq!(nodes[12].cells[0].style, CellStyle::Dim);
    }

    #[test]
    fn a_detail_of_an_older_head_is_stale() {
        let mut detail = testdata::detail();
        detail.head_oid = "an older head".into();
        let state = state_with(Some(detail), None);
        let mut ps = Conversation.init_state();
        let model = model_and_keys(&state, &mut ps, &[]);
        assert_eq!(
            outline(&model)[0],
            "stale: read at an older head, a new load is on its way"
        );
        let fresh = state_with(Some(testdata::detail()), None);
        let model = model_and_keys(&fresh, &mut ps, &[]);
        assert_eq!(outline(&model)[0], "alice · commented 3h ago");
    }

    #[test]
    fn loading_empty_and_error_states() {
        let mut ps = Conversation.init_state();
        let loading = state_with(None, None);
        assert_eq!(
            outline(&model_and_keys(&loading, &mut ps, &[])),
            ["loading the conversation…"]
        );
        let failed = state_with(None, Some("offline"));
        assert_eq!(
            outline(&model_and_keys(&failed, &mut ps, &[])),
            ["cannot load the conversation: offline"],
            "no loading line under the error"
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

    /// The reply of the `r` key with the cursor on the node `cursor`,
    /// after the keys `keys`.
    fn reply_at(state: &AppState, keys: &[KeyCode], cursor: usize) -> ReplySpec {
        let mut ps = Conversation.init_state();
        model_and_keys(state, &mut ps, keys);
        ps.cursor = cursor;
        let key = testdata::pr_key("chaoshub", 7);
        let target = pr_target(&state.data, &key).unwrap();
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let r = KeyChord {
            code: KeyCode::Char('r'),
            mods: KeyModifiers::NONE,
        };
        let handled = Conversation.handle_key(r, &cx, &mut ps).unwrap();
        assert!(!handled.changed && handled.cmds.is_empty());
        handled.reply.unwrap()
    }

    fn thread(id: &str, place: &str) -> ReplyTo {
        ReplyTo::Thread {
            id: id.into(),
            place: place.into(),
        }
    }

    #[test]
    fn each_node_of_a_comment_a_review_or_a_thread_has_its_reply() {
        let state = state_with(Some(testdata::detail()), None);
        let alice = "> @alice: Looks good, one question about `layout`.\n\n";
        let bob = "> @bob: Please split the **transition** module.\n\n";
        // The nodes: alice, her body, two threads, bob, his body.
        for (cursor, to, title, text) in [
            (0, ReplyTo::Pr, "comment on #7", alice),
            (1, ReplyTo::Pr, "comment on #7", alice),
            (
                2,
                thread("PRRT_42", "src/ui/list.rs:42"),
                "reply on src/ui/list.rs:42",
                "",
            ),
            (
                3,
                thread("PRRT_7", "src/ui/pane.rs:7"),
                "reply on src/ui/pane.rs:7",
                "",
            ),
            (4, ReplyTo::Pr, "comment on #7", bob),
            (5, ReplyTo::Pr, "comment on #7", bob),
        ] {
            let spec = reply_at(&state, &[], cursor);
            assert_eq!(
                spec,
                ReplySpec {
                    to,
                    title: title.into(),
                    text: text.into(),
                },
                "node {cursor}"
            );
        }
    }

    #[test]
    fn a_comment_in_an_expanded_thread_replies_in_the_thread() {
        let state = state_with(Some(testdata::detail()), None);
        let mut ps = Conversation.init_state();
        ps.cursor = 2;
        model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        let expanded = ps.ext.downcast_ref::<ConversationState>().unwrap();
        assert!(expanded.expanded.contains(&0));
        // The nodes 3 to 6: the two comments of the thread and their
        // bodies.
        let key = testdata::pr_key("chaoshub", 7);
        let target = pr_target(&state.data, &key).unwrap();
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let r = KeyChord {
            code: KeyCode::Char('r'),
            mods: KeyModifiers::NONE,
        };
        for cursor in 3..=6 {
            ps.cursor = cursor;
            let spec = Conversation.handle_key(r, &cx, &mut ps).unwrap().reply;
            assert_eq!(
                spec.map(|s| s.to),
                Some(thread("PRRT_42", "src/ui/list.rs:42")),
                "node {cursor}"
            );
        }
    }

    #[test]
    fn a_line_without_a_comment_starts_a_new_comment() {
        let new = ReplySpec {
            to: ReplyTo::Pr,
            title: "comment on #7".into(),
            text: String::new(),
        };
        let loading = state_with(None, None);
        assert_eq!(reply_at(&loading, &[], 0), new, "no detail yet");
        let empty = PrDetail {
            comments: Vec::new(),
            reviews: Vec::new(),
            threads: Vec::new(),
            head_oid: "oid7".into(),
        };
        let empty = state_with(Some(empty), None);
        assert_eq!(reply_at(&empty, &[], 0), new, "no comments yet");
        let mut stale = testdata::detail();
        stale.head_oid = "an older head".into();
        let stale = state_with(Some(stale), None);
        assert_eq!(reply_at(&stale, &[], 0), new, "the stale line");
    }

    #[test]
    fn a_quote_has_the_author_and_the_first_line_with_text() {
        assert_eq!(
            quote("bob", "\r\n  First.  \r\nSecond."),
            "> @bob: First.\n\n"
        );
        assert_eq!(quote("bob", " \n "), "@bob ");
    }

    #[test]
    fn the_pane_shows_and_runs_the_reply_key() {
        assert_eq!(Conversation.hints(), [("reply", "reply")]);
        let state = state_with(Some(testdata::detail()), None);
        let key = testdata::pr_key("chaoshub", 7);
        let target = pr_target(&state.data, &key).unwrap();
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let mut ps = Conversation.init_state();
        let actions = Conversation.actions(&cx, &ps);
        assert_eq!(actions, [PaneAction::char('r', "reply to the comment")]);
        let handled = Conversation.run_action(actions[0].key, &cx, &mut ps);
        assert_eq!(
            handled.and_then(|h| h.reply).map(|s| s.to),
            Some(ReplyTo::Pr)
        );
        let other = PaneAction::char('x', "").key;
        assert_eq!(Conversation.run_action(other, &cx, &mut ps), None);
    }
}
