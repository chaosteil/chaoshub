//! The help model: the key bindings of the app, of each tab and of each
//! action, in sections.
//!
//! `app` builds the model from the key map, `TABS` and `REGISTRY`, so `ui`
//! renders it without a look at the tabs or the actions.

use crate::actions::Action;
use crate::app::keymap::{self, Keymap};
use crate::app::models::{HelpModel, HelpSection};
use crate::app::tabs::Tab;

/// The app bindings that the help shows, with their labels, in help
/// order. One entry can show more than one binding. The tab bindings
/// `tab_1` to `tab_9` come from the tabs. The keys of the popups (for
/// example `confirm`) show in the popups.
const APP_KEYS: &[(&[&str], &str)] = &[
    (&["next_tab"], "next tab"),
    (&["down", "up"], "down, up"),
    (&["top", "bottom"], "top, bottom"),
    (&["page_down", "page_up"], "page down, page up"),
    (&["open"], "open the popup"),
    (&["actions"], "open the actions menu"),
    (&["close_overlay"], "close the popup"),
    (&["filter"], "filter"),
    (&["authors"], "pick an author"),
    (&["toggle_all"], "show all open"),
    (&["refresh"], "refresh"),
    (&["column_headers"], "show or hide the column names"),
    (&["quit"], "quit"),
];

/// The title of the actions section of the help.
const ACTIONS_TITLE: &str = "actions (in the sidebar, the detail and the actions menu)";

/// The help model: the app keys, the keys of each tab that has its own,
/// and the actions. A binding without a chord is not in the app keys.
/// The title of the actions section tells where the action keys work.
pub(crate) fn help_model(
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    actions: &[Box<dyn Action>],
) -> HelpModel {
    let mut sections = vec![app_section(keymap, tabs)];
    for tab in tabs {
        let entries: Vec<(String, String)> = tab
            .help()
            .into_iter()
            .map(|(chord, label)| (keymap::chord_text(chord), label.to_owned()))
            .collect();
        if !entries.is_empty() {
            sections.push(HelpSection {
                title: tab.title().to_owned(),
                entries,
            });
        }
    }
    let mut action_section = keymap::action_help(keymap, actions);
    ACTIONS_TITLE.clone_into(&mut action_section.title);
    sections.push(action_section);
    HelpModel { sections }
}

/// The app keys: one tab key per tab, then the bindings of `APP_KEYS`.
fn app_section(keymap: &Keymap, tabs: &[Box<dyn Tab>]) -> HelpSection {
    let tab_keys = tabs.iter().take(9).enumerate().map(|(i, tab)| {
        (
            vec![format!("tab_{}", i + 1)],
            format!("tab {}", tab.title()),
        )
    });
    let app_keys = APP_KEYS.iter().map(|&(names, label)| {
        let names = names.iter().map(|&name| name.to_owned()).collect();
        (names, label.to_owned())
    });
    let entries = tab_keys
        .chain(app_keys)
        .filter_map(|(names, label)| {
            let keys: Vec<String> = names
                .iter()
                .flat_map(|name| keymap.chords(name))
                .map(|&chord| keymap::chord_text(chord))
                .collect();
            (!keys.is_empty()).then(|| (keys.join(" "), label))
        })
        .collect();
    HelpSection {
        title: "keys".to_owned(),
        entries,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::actions::fake::FakeAction;
    use crate::app::tabs::TABS;
    use crate::app::tabs::fake::FakeTab;

    fn entry(key: &str, label: &str) -> (String, String) {
        (key.to_owned(), label.to_owned())
    }

    #[test]
    fn sections_are_keys_then_actions() {
        let tabs: Vec<Box<dyn Tab>> = vec![Box::new(FakeTab::new("one"))];
        let actions: Vec<Box<dyn Action>> = vec![
            Box::new(FakeAction::new("merge", 'm')),
            Box::new(FakeAction::new("review", 'v')),
        ];
        let overrides = BTreeMap::from([
            ("refresh".to_owned(), "F5 r".to_owned()),
            ("authors".to_owned(), String::new()),
        ]);
        let keymap = Keymap::with_actions(&actions, &overrides).unwrap();
        let model = help_model(&keymap, &tabs, &actions);
        let titles: Vec<&str> = model.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["keys", ACTIONS_TITLE]);
        let keys = &model.sections[0].entries;
        assert_eq!(keys[0], entry("1", "tab one"));
        assert_eq!(keys[1], entry("tab", "next tab"));
        assert!(keys.contains(&entry("F5 r", "refresh")));
        assert!(keys.contains(&entry("q esc ctrl-c", "quit")));
        assert!(keys.contains(&entry("esc q", "close the popup")));
        assert!(keys.contains(&entry("j k", "down, up")));
        assert!(
            !keys.iter().any(|(_, label)| label == "pick an author"),
            "an unbound key is not in the help"
        );
        assert!(!keys.iter().any(|(_, label)| label == "tab two"));
        assert_eq!(
            model.sections[1].entries,
            [entry("m", "merge"), entry("v", "review")]
        );
    }

    #[test]
    fn the_real_tabs_have_a_tab_key_each() {
        let model = help_model(&Keymap::default(), &TABS, &[]);
        let keys = &model.sections[0].entries;
        for (i, tab) in TABS.iter().enumerate() {
            assert_eq!(
                keys[i],
                entry(&(i + 1).to_string(), &format!("tab {}", tab.title()))
            );
        }
        assert_eq!(
            model.sections.last().map(|s| s.title.as_str()),
            Some(ACTIONS_TITLE)
        );
    }
}
