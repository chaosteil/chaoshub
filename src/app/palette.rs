//! The questions that the command palette of Rex asks in place of an
//! overlay of chaoshub (see `rex.ui` and `launch::palette`).

use crate::app::state::{AuthorPicker, OverlayState};
use crate::bus::{PaletteAsk, PaletteItem};

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
