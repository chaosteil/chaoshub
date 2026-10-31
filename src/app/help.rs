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
/// order. The tab bindings `tab_1` to `tab_9` come from the tabs.
const APP_KEYS: &[(&str, &str)] = &[
    ("next_tab", "next tab"),
    ("down", "down"),
    ("up", "up"),
    ("top", "top"),
    ("bottom", "bottom"),
    ("page_down", "page down"),
    ("page_up", "page up"),
    ("open", "open the popup"),
    ("open_url", "open the url in the popup"),
    ("close_overlay", "close the popup"),
    ("confirm", "confirm"),
    ("filter", "filter"),
    ("authors", "pick an author"),
    ("toggle_all", "show all open"),
    ("refresh", "refresh"),
    ("help", "help"),
    ("quit", "quit"),
];

/// The help model: the app keys, the keys of each tab that has its own,
/// and the actions. A binding without a chord is not in the app keys.
#[allow(dead_code, reason = "the help overlay opens with this model")]
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
    sections.push(keymap::action_help(keymap, actions));
    HelpModel { sections }
}

/// The app keys: one tab key per tab, then the bindings of `APP_KEYS`.
fn app_section(keymap: &Keymap, tabs: &[Box<dyn Tab>]) -> HelpSection {
    let tab_keys = tabs
        .iter()
        .take(9)
        .enumerate()
        .map(|(i, tab)| (format!("tab_{}", i + 1), format!("tab {}", tab.title())));
    let app_keys = APP_KEYS
        .iter()
        .map(|&(name, label)| (name.to_owned(), label.to_owned()));
    let entries = tab_keys
        .chain(app_keys)
        .filter_map(|(name, label)| {
            let chords = keymap.chords(&name);
            if chords.is_empty() {
                return None;
            }
            let keys: Vec<String> = chords.iter().map(|&c| keymap::chord_text(c)).collect();
            Some((keys.join(" "), label))
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
        assert_eq!(titles, ["keys", "actions"]);
        let keys = &model.sections[0].entries;
        assert_eq!(keys[0], entry("1", "tab one"));
        assert_eq!(keys[1], entry("tab", "next tab"));
        assert!(keys.contains(&entry("F5 r", "refresh")));
        assert!(keys.contains(&entry("q ctrl-c", "quit")));
        assert!(keys.contains(&entry("esc q", "close the popup")));
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
            Some("actions")
        );
    }
}
