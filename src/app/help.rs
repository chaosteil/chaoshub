//! The help model: the key bindings of the app, of each tab and of each
//! action, and the symbols of the list, in sections.
//!
//! `app` builds the model from the key map, `TABS` and `REGISTRY`, so `ui`
//! renders it without a look at the tabs or the actions.

use crate::actions::Action;
use crate::app::keymap::{self, Keymap};
use crate::app::models::{Cell, CellStyle, HelpModel, HelpSection};
use crate::app::tabs::Tab;
use crate::app::tabs::cells::{self, cell};
use crate::domain::pr::{CiState, ReviewDecision};

/// The app bindings that the help shows, with their labels, in help
/// order. One entry can show more than one binding. The tab bindings
/// `tab_1` to `tab_9` come from the tabs. The keys of the popups (for
/// example `confirm`) show in the popups.
const APP_KEYS: &[(&[&str], &str)] = &[
    (&["next_tab"], "next tab"),
    (&["down", "up"], "down, up"),
    (&["top", "bottom"], "top, bottom"),
    (&["page_down", "page_up"], "half a page down, up"),
    (&["scroll_down", "scroll_up"], "scroll down, up"),
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

/// The title of the section of the list symbols.
const SYMBOLS_TITLE: &str = "list symbols";

/// The title of the actions section of the help.
const ACTIONS_TITLE: &str = "actions (in the actions menu)";

/// The title of the section of the action keys that work directly in the
/// list and in the pull request detail.
const DETAIL_TITLE: &str = "actions in the list and the detail";

/// The help model: the app keys, the list symbols, the keys of each tab
/// that has its own, the actions, and the direct action keys of the list
/// and the pull request detail. A binding without a chord is not in the
/// help. The titles of the action sections tell where the action keys
/// work.
pub(crate) fn help_model(
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    actions: &[Box<dyn Action>],
) -> HelpModel {
    let mut sections = vec![app_section(keymap, tabs), symbols_section()];
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
                symbols: Vec::new(),
            });
        }
    }
    let mut action_section = keymap::action_help(keymap, actions);
    ACTIONS_TITLE.clone_into(&mut action_section.title);
    sections.push(action_section);
    let detail: Vec<(String, String)> = actions
        .iter()
        .filter_map(|action| {
            let keys: Vec<String> = keymap
                .detail_chords(action.id())
                .into_iter()
                .map(keymap::chord_text)
                .collect();
            (!keys.is_empty()).then(|| (keys.join(" "), action.label().to_owned()))
        })
        .collect();
    if !detail.is_empty() {
        sections.push(HelpSection {
            title: DETAIL_TITLE.to_owned(),
            entries: detail,
            symbols: Vec::new(),
        });
    }
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
        symbols: Vec::new(),
    }
}

/// The list symbols, in the styles of the list: the glyphs of the CHECKS,
/// REV and FLG columns, the CHANGE colors and the stack tree of the Mine
/// tab, and the ready rule of the Review and the Team tab. A column name
/// in the style of the column header starts each group. The labels are
/// short, so that they fit the help overlay at its minimum width.
fn symbols_section() -> HelpSection {
    let heading = |name: &str| cell(name, CellStyle::Dim);
    let checks = |state, counts: &str| {
        let glyph = cells::ci(state);
        cell(format!("{} {counts}", glyph.text), glyph.style)
    };
    let symbols: [(Cell, &str); 27] = [
        (heading("CHECKS"), "CI state, passed/all checks"),
        (cell("·", CellStyle::Dim), "no checks"),
        (checks(CiState::Success, "5/5"), "CI green, 5 of 5 pass"),
        (checks(CiState::Failure, "3/5"), "CI failed, 3 of 5 pass"),
        (checks(CiState::Pending, "2/5"), "CI pending, 2 of 5 pass"),
        (heading("REV"), "approvals and change requests"),
        (cell("✓2", CellStyle::Success), "2 approvals"),
        (
            cell("✓2 ✗1", CellStyle::Danger),
            "2 approvals, 1 change request",
        ),
        (cells::review(Some(ReviewDecision::Approved)), "approved"),
        (
            cells::review(Some(ReviewDecision::ChangesRequested)),
            "changes requested",
        ),
        (
            cells::review(Some(ReviewDecision::ReviewRequired)),
            "review required",
        ),
        (cells::review(None), "no review needed"),
        (heading("FLG"), "flags"),
        (cell("D", CellStyle::Warn), "draft"),
        (cell("C", CellStyle::Warn), "merge conflicts"),
        (cell("Q", CellStyle::Warn), "in the merge queue"),
        (cell("◆", CellStyle::Warn), "a pane of the PR is open"),
        (heading("CHANGE"), "the local change (Mine tab)"),
        (
            cell("wqnv", CellStyle::Primary),
            "synced: same commit as the PR",
        ),
        (
            cell("wqnv", CellStyle::Warn),
            "diverged: a different commit",
        ),
        (heading("TITLE"), "the stack tree (Mine tab)"),
        (cell("─", CellStyle::Dim), "a PR without a stack"),
        (cell("┌", CellStyle::Dim), "the top PR of a stack"),
        (cell("├", CellStyle::Dim), "a middle PR of a stack"),
        (cell("└", CellStyle::Dim), "the bottom PR of a stack"),
        (heading("READY"), "what Review and Team show"),
        (cell("ready", CellStyle::Dim), "open, not a draft, CI green"),
    ];
    HelpSection {
        title: SYMBOLS_TITLE.to_owned(),
        entries: Vec::new(),
        symbols: symbols
            .into_iter()
            .map(|(symbol, label)| (symbol, label.to_owned()))
            .collect(),
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
        assert_eq!(titles, ["keys", SYMBOLS_TITLE, ACTIONS_TITLE, DETAIL_TITLE]);
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
            model.sections[2].entries,
            [entry("m", "merge"), entry("v", "review")]
        );
        assert_eq!(
            model.sections[3].entries,
            [entry("v", "review")],
            "only review works directly in the list and the detail"
        );
        let overrides = BTreeMap::from([("detail_merge".to_owned(), "m".to_owned())]);
        let keymap = Keymap::with_actions(&actions, &overrides).unwrap();
        let model = help_model(&keymap, &tabs, &actions);
        assert_eq!(
            model.sections[3].entries,
            [entry("m", "merge"), entry("v", "review")],
            "a detail binding adds a direct key"
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
