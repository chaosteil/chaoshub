//! The questions that the command palette of Rex asks in place of an
//! overlay of chaoshub (see `rex.ui` and `launch::palette`).

use std::collections::HashSet;

use crate::app::state::{AppData, AuthorPicker, OverlayState, PromptFor};
use crate::app::term::pr_status;
use crate::bus::{PaletteAsk, PaletteItem};
use crate::domain::keys::PrKey;

/// The text of the status bar while a question waits for its answer.
pub(crate) const WAITING: &str = "waiting for Rex";

/// The label of the submit row of the question to pr-agent.
const ASK: &str = "Ask";
/// The label of the submit row of the text prompt of an action.
const SEND: &str = "Send";
/// The label of the submit row of the author picker.
const FILTER: &str = "Filter";
/// The label of the submit row of the jump to a pull request.
const OPEN: &str = "Open";

/// The most items of the jump to a pull request. `rex.ui.pick` takes at
/// most 2,000 items.
pub(crate) const MAX_ITEMS: usize = 2000;

/// The icon of each tab in the jump to a pull request, as in
/// `contrib/rex/chaoshub.lua`.
const TAB_ICONS: &[(&str, &str)] = &[("mine", "branch"), ("review", "person"), ("team", "code")];

/// The question of `overlay` for the palette. `None` when the overlay
/// has no palette form. A prompt starts with the text of the overlay.
pub(crate) fn ask(overlay: &OverlayState) -> Option<PaletteAsk> {
    match overlay {
        OverlayState::Authors(picker) => Some(authors(picker)),
        OverlayState::Prompt {
            purpose,
            spec,
            text,
        } => Some(PaletteAsk::Input {
            title: spec.title.clone(),
            placeholder: spec.placeholder.clone(),
            value: text.clone(),
            confirm: Some(match purpose {
                PromptFor::Action(action) if action.id == crate::agent::ID => ASK,
                PromptFor::Action(_) => SEND,
                PromptFor::OpenUrl => OPEN,
            }),
        }),
        _ => None,
    }
}

/// The author picker as a pick: `all authors`, then the authors, each
/// with the `person` icon. The item of the current filter says
/// `current`. The id of an item is its text: a GitHub login has no
/// space, so no login is `all authors`.
fn authors(picker: &AuthorPicker) -> PaletteAsk {
    let items = picker
        .items()
        .into_iter()
        .enumerate()
        .map(|(index, title)| PaletteItem {
            id: title.clone(),
            title,
            subtitle: None,
            detail: (index == picker.cursor).then(|| "current".to_owned()),
            icon: Some("person"),
        })
        .collect();
    PaletteAsk::Pick {
        title: "filter by author".to_owned(),
        placeholder: "filter authors".to_owned(),
        value: String::new(),
        confirm: Some(FILTER),
        allow_other: false,
        items,
    }
}

/// The id of the pull request `key` in the jump: `owner/name#N`, as in
/// `chaoshub_open_pr`.
pub(crate) fn pr_id(key: &PrKey) -> String {
    format!("{}/{}#{}", key.repo.owner, key.repo.name, key.number)
}

/// The jump to a pull request as a pick: one item for each pull request
/// of `rows`, in the order of `rows`. `rows` holds the tab id and the
/// pull request of each row of the tabs, in tab order. A pull request in
/// two tabs shows once, with its first tab. The item has the title, the
/// repo and the author, the status and the icon of its tab, as in
/// `chaoshub_open_pr`. The pick holds at most `MAX_ITEMS` items. `None`
/// when no row is a loaded pull request.
pub(crate) fn jump(data: &AppData, rows: &[(&str, PrKey)]) -> Option<PaletteAsk> {
    let mut seen = HashSet::new();
    let items: Vec<PaletteItem> = rows
        .iter()
        .filter(|(_, key)| seen.insert(key))
        .filter_map(|(tab, key)| {
            let pr = data.prs.get(key)?;
            let head = &pr.head.value;
            Some(PaletteItem {
                id: pr_id(key),
                title: format!("#{} {}", key.number, head.title),
                subtitle: Some(format!(
                    "{}/{} · {}",
                    key.repo.owner, key.repo.name, head.author
                )),
                detail: Some(pr_status(pr)),
                icon: TAB_ICONS
                    .iter()
                    .find(|(id, _)| id == tab)
                    .map(|&(_, icon)| icon),
            })
        })
        .take(MAX_ITEMS)
        .collect();
    if items.is_empty() {
        return None;
    }
    Some(PaletteAsk::Pick {
        title: "jump to PR".to_owned(),
        placeholder: "filter PRs".to_owned(),
        value: String::new(),
        confirm: Some(OPEN),
        allow_other: false,
        items,
    })
}

/// The cursor of the item with the id `id` in the author picker: 0 for
/// `all authors`. `None` when `id` is not an item of the picker.
pub(crate) fn author_cursor(picker: &AuthorPicker, id: &str) -> Option<usize> {
    picker.items().iter().position(|item| item == id)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn picker(cursor: usize) -> AuthorPicker {
        AuthorPicker {
            tab: 1,
            authors: vec!["alice".into(), "bob".into()],
            cursor,
        }
    }

    fn item(id: &str, current: bool) -> PaletteItem {
        PaletteItem {
            id: id.into(),
            title: id.into(),
            subtitle: None,
            detail: current.then(|| "current".into()),
            icon: Some("person"),
        }
    }

    #[test]
    fn the_author_picker_is_a_pick_with_all_authors_first() {
        let overlay = OverlayState::Authors(picker(2));
        assert_eq!(
            ask(&overlay),
            Some(PaletteAsk::Pick {
                title: "filter by author".into(),
                placeholder: "filter authors".into(),
                value: String::new(),
                confirm: Some("Filter"),
                allow_other: false,
                items: vec![
                    item("all authors", false),
                    item("alice", false),
                    item("bob", true),
                ],
            })
        );
        let PaletteAsk::Pick { items, .. } = authors(&picker(0)) else {
            panic!("not a pick");
        };
        assert_eq!(items[0], item("all authors", true));
    }

    #[test]
    fn a_prompt_asks_with_its_text_and_a_confirm_label() {
        use crate::app::state::ActionDialog;
        use crate::app::testdata;
        use crate::bus::PromptSpec;

        let mut state = crate::app::state::AppState::new(crate::bus::Size {
            width: 80,
            height: 20,
        });
        state.load(testdata::mine_snapshot(), Vec::new());
        let key = &testdata::mine_prs()[0].key;
        let prompt = |id: &'static str| OverlayState::Prompt {
            purpose: PromptFor::Action(ActionDialog {
                id,
                selection: Box::new(crate::bus::Selection {
                    target: crate::app::target::pr_target(&state.data, key).unwrap(),
                    me: String::new(),
                    action_config: None,
                }),
            }),
            spec: PromptSpec {
                title: "t".into(),
                placeholder: String::new(),
            },
            text: "draft".into(),
        };
        let confirm = |id| match ask(&prompt(id)) {
            Some(PaletteAsk::Input { value, confirm, .. }) => (value, confirm),
            other => panic!("not an input: {other:?}"),
        };
        assert_eq!(confirm("agent"), ("draft".into(), Some("Ask")));
        assert_eq!(confirm("request_changes"), ("draft".into(), Some("Send")));
        let url = OverlayState::Prompt {
            purpose: PromptFor::OpenUrl,
            spec: PromptSpec {
                title: "open url".into(),
                placeholder: "https://github.com/OWNER/NAME/pull/N".into(),
            },
            text: "github.com/a/b/pull/1".into(),
        };
        assert_eq!(
            ask(&url),
            Some(PaletteAsk::Input {
                title: "open url".into(),
                placeholder: "https://github.com/OWNER/NAME/pull/N".into(),
                value: "github.com/a/b/pull/1".into(),
                confirm: Some("Open"),
            })
        );
    }

    #[test]
    fn the_jump_shows_each_pull_request_once_and_at_most_2000() {
        use crate::app::testdata;

        let mut data = crate::app::state::AppState::new(crate::bus::Size {
            width: 80,
            height: 20,
        })
        .data;
        let count = MAX_ITEMS as u64 + 5;
        for number in 1..=count {
            let pr = testdata::pr("chaoshub", number, "t");
            data.prs.insert(pr.key.clone(), pr);
        }
        let key = |number| testdata::pr_key("chaoshub", number);
        let mut rows = vec![("mine", key(1)), ("team", key(1)), ("review", key(404_404))];
        rows.extend((2..=count).map(|number| ("team", key(number))));
        let Some(PaletteAsk::Pick { items, .. }) = jump(&data, &rows) else {
            panic!("no pick");
        };
        assert_eq!(items.len(), MAX_ITEMS);
        assert_eq!(items[0].id, "chaosteil/chaoshub#1");
        assert_eq!(items[0].icon, Some("branch"), "the first tab wins");
        assert_eq!(items[1].id, "chaosteil/chaoshub#2", "no item without data");
        assert_eq!(jump(&data, &[]), None);
    }

    #[test]
    fn an_answer_is_an_author_or_all_authors() {
        let picker = picker(0);
        assert_eq!(author_cursor(&picker, "all authors"), Some(0));
        assert_eq!(author_cursor(&picker, "bob"), Some(2));
        assert_eq!(author_cursor(&picker, "carol"), None);
    }
}
