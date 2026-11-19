//! The questions that the command palette of Rex asks in place of an
//! overlay of chaoshub (see `rex.ui` and `launch::palette`).

use crate::actions::{Action, Applicability};
use crate::app::frame;
use crate::app::keymap::Keymap;
use crate::app::state::{ActionMenu, AppData, AuthorPicker, OverlayState};
use crate::bus::{PaletteAsk, PaletteItem, Target};

/// The text of the status bar while a question waits for its answer.
pub(crate) const WAITING: &str = "waiting for an answer in Rex";

/// The question of `overlay` for the palette. `None` when the overlay
/// has no palette form.
pub(crate) fn ask(overlay: &OverlayState) -> Option<PaletteAsk> {
    match overlay {
        OverlayState::Authors(picker) => Some(authors(picker)),
        OverlayState::Prompt { spec, .. } => Some(PaletteAsk::Input {
            title: spec.title.clone(),
            placeholder: spec.placeholder.clone(),
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
        title: "author".to_owned(),
        placeholder: "filter the authors".to_owned(),
        items,
    }
}

/// The prefix of the id of a pane action in the actions menu. The id of
/// an action has no `:`.
const PANE_PREFIX: &str = "pane:";

/// The detail of a pane action with `mine`: a reaction that you gave.
const MINE: &str = "yours";

/// The actions menu as a pick, in the order of the menu. The id of an
/// action item is the action id, and its detail is its first key. A
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
            icon: None,
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
    fn an_answer_is_an_author_or_all_authors() {
        let picker = picker(0);
        assert_eq!(author_cursor(&picker, "all authors"), Some(0));
        assert_eq!(author_cursor(&picker, "bob"), Some(2));
        assert_eq!(author_cursor(&picker, "carol"), None);
    }
}
