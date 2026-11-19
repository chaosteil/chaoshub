//! The questions that the command palette of Rex asks in place of an
//! overlay of chaoshub (see `rex.ui` and `launch::palette`).

use crate::actions::{Action, Applicability};
use crate::app::frame;
use crate::app::keymap::Keymap;
use crate::app::state::{ActionMenu, AppData, AuthorPicker, OverlayState};
use crate::bus::{PaletteAsk, PaletteItem, Target};

/// The text of the status bar while a question waits for its answer.
pub(crate) const WAITING: &str = "waiting for an answer in Rex";

/// The label of the submit row of the question to pr-agent.
const ASK: &str = "Ask";
/// The label of the submit row of the text prompt of an action.
const SEND: &str = "Send";
/// The label of the submit row of the author picker.
const FILTER: &str = "Filter";
/// The label of the submit row of the actions menu and of the comment
/// menu.
const RUN: &str = "Run";

/// The icon of each action in the actions menu, from the icon names of
/// `rex.ui`. An action that is not in the list has no icon.
const ICONS: &[(&str, &str)] = &[
    ("browser", "globe"),
    ("copy_url", "link"),
    ("copy_change_id", "tag"),
    ("review", "code"),
    ("merge", "branch"),
    ("merge_stack", "branch"),
    ("rerun_checks", "gear"),
    ("draft", "document"),
    ("agent", "terminal"),
];

/// The question of `overlay` for the palette. `None` when the overlay
/// has no palette form. A prompt starts with the text of the overlay.
pub(crate) fn ask(overlay: &OverlayState) -> Option<PaletteAsk> {
    match overlay {
        OverlayState::Authors(picker) => Some(authors(picker)),
        OverlayState::Prompt { action, spec, text } => Some(PaletteAsk::Input {
            title: spec.title.clone(),
            placeholder: spec.placeholder.clone(),
            value: text.clone(),
            confirm: Some(if action.id == crate::agent::ID {
                ASK
            } else {
                SEND
            }),
        }),
        _ => None,
    }
}

/// The icon of the action `id` in the actions menu.
fn icon(id: &str) -> Option<&'static str> {
    ICONS
        .iter()
        .find(|(action, _)| *action == id)
        .map(|&(_, icon)| icon)
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
        title: "author".to_owned(),
        placeholder: "filter the authors".to_owned(),
        value: String::new(),
        confirm: Some(FILTER),
        allow_other: false,
        items,
    }
}

/// The prefix of the id of a pane action in the actions menu. The id of
/// an action has no `:`.
const PANE_PREFIX: &str = "pane:";

/// The detail of a pane action with `mine`: a reaction that you gave.
const MINE: &str = "yours";

/// The actions menu as a pick, in the order of the menu. The id of an
/// action item is the action id, its detail is its first key, and its
/// icon comes from `ICONS`. A
/// disabled action has its reason as the subtitle. The id of a pane
/// action is `pane:<index>`. As in the menu of chaoshub, a pane action
/// with the key of an action shows no key. A pane action with `mine`,
/// for example a reaction that you gave, has `yours` as its detail in
/// place of its key. The comment menu is an actions menu with only pane
/// actions.
pub(crate) fn menu(
    data: &AppData,
    keymap: &Keymap,
    actions: &[Box<dyn Action>],
    menu: &ActionMenu,
) -> PaletteAsk {
    let states = frame::applicability(data, actions, (*menu.target).clone());
    let key = |name: &str| {
        keymap
            .chords(name)
            .first()
            .map(|&chord| frame::key_text(chord))
    };
    let action_items = menu.ids.iter().map(|&id| {
        let found = actions
            .iter()
            .zip(&states)
            .find(|(action, _)| action.id() == id);
        let (title, subtitle) = match found {
            Some((action, state)) => (
                action.target_label(&menu.target).to_owned(),
                match state {
                    Applicability::Disabled(reason) => Some((*reason).to_owned()),
                    Applicability::Enabled | Applicability::Hidden => None,
                },
            ),
            None => (id.replace('_', " "), None),
        };
        PaletteItem {
            id: id.to_owned(),
            title,
            subtitle,
            detail: key(id),
            icon: icon(id),
        }
    });
    let pane_items = menu.pane.iter().enumerate().map(|(index, action)| {
        let shadowed = menu.ids.iter().any(|id| keymap.matches(id, action.key));
        let detail = if action.mine {
            Some(MINE.to_owned())
        } else {
            (!shadowed).then(|| frame::key_text(action.key))
        };
        PaletteItem {
            id: format!("{PANE_PREFIX}{index}"),
            title: action.label.clone(),
            subtitle: None,
            detail,
            icon: None,
        }
    });
    let Target::Pr { pr, .. } = &*menu.target;
    PaletteAsk::Pick {
        title: menu
            .title
            .clone()
            .unwrap_or_else(|| format!("actions for #{}", pr.key.number)),
        placeholder: "filter the actions".to_owned(),
        value: String::new(),
        confirm: Some(RUN),
        allow_other: false,
        items: action_items.chain(pane_items).collect(),
    }
}

/// The index of the item with the id `id` in the actions menu: the
/// actions first, then the pane actions. `None` when `id` is not an
/// item of the menu.
pub(crate) fn menu_item(menu: &ActionMenu, id: &str) -> Option<usize> {
    if let Some(index) = id.strip_prefix(PANE_PREFIX) {
        let index: usize = index.parse().ok()?;
        return (index < menu.pane.len()).then(|| menu.ids.len() + index);
    }
    menu.ids.iter().position(|action| *action == id)
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
                title: "author".into(),
                placeholder: "filter the authors".into(),
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

    /// The icon names of `rex.ui.pick`, from
    /// <https://www.superlogical.com/rex/docs/customize/custom-ui.md>.
    const REX_ICONS: &[&str] = &[
        "document", "folder", "branch", "terminal", "code", "server", "database", "package",
        "globe", "link", "person", "tag", "clock", "star", "gear", "plus",
    ];

    #[test]
    fn each_menu_icon_is_a_rex_icon_of_an_action() {
        use crate::actions::REGISTRY;
        for (id, name) in ICONS {
            assert!(REX_ICONS.contains(name), "{id}: {name} is not a Rex icon");
            assert!(
                REGISTRY.iter().any(|action| action.id() == *id),
                "{id} is not an action"
            );
        }
        assert_eq!(icon("merge_stack"), Some("branch"));
        assert_eq!(icon("close"), None);
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
            action: ActionDialog {
                id,
                selection: Box::new(crate::bus::Selection {
                    target: crate::app::target::pr_target(&state.data, key).unwrap(),
                    me: String::new(),
                    action_config: None,
                }),
            },
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
    }

    #[test]
    fn an_answer_is_an_author_or_all_authors() {
        let picker = picker(0);
        assert_eq!(author_cursor(&picker, "all authors"), Some(0));
        assert_eq!(author_cursor(&picker, "bob"), Some(2));
        assert_eq!(author_cursor(&picker, "carol"), None);
    }
}
