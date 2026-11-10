//! The key map: defaults, overrides and action lookup.
//!
//! A binding is a name, for example `quit`, and one or more key chords. A
//! chord is written like `q`, `A`, `ctrl-d`, `alt-x`, `F5`, `enter` or
//! `shift-tab`. The overrides map a binding name to chords separated by
//! spaces. An override replaces every default chord of that name. An
//! empty value removes the binding. The binding name of an action is its
//! id, and its default chord is `Action::default_key`.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use crate::actions::{Action, REGISTRY};
use crate::app::models::HelpSection;
use crate::bus::{KeyChord, KeyCode, KeyEvent, KeyModifiers};

/// The default bindings of the app keys (section 1.8 of the plan). The
/// action keys come from the action registry.
const DEFAULTS: &[(&str, &[&str])] = &[
    ("tab_1", &["1"]),
    ("tab_2", &["2"]),
    ("tab_3", &["3"]),
    ("tab_4", &["4"]),
    ("tab_5", &["5"]),
    ("tab_6", &["6"]),
    ("tab_7", &["7"]),
    ("tab_8", &["8"]),
    ("tab_9", &["9"]),
    ("next_tab", &["tab"]),
    ("down", &["j"]),
    ("up", &["k"]),
    ("top", &["g"]),
    ("bottom", &["G"]),
    ("page_down", &["ctrl-d"]),
    ("page_up", &["ctrl-u"]),
    ("scroll_down", &["D"]),
    ("scroll_up", &["U"]),
    ("open", &["enter"]),
    ("open_url", &["o"]),
    ("actions", &["space"]),
    ("close_overlay", &["esc", "q"]),
    ("confirm", &["y"]),
    ("cancel", &["n"]),
    ("filter", &["/"]),
    ("authors", &["a"]),
    ("toggle_all", &["t"]),
    ("refresh", &["r"]),
    ("column_headers", &["H"]),
    ("help", &["?"]),
    ("quit", &["q", "esc", "ctrl-c"]),
];

/// The actions whose own keys also work directly in the pull request
/// detail. They change nothing on GitHub. In the detail, the other
/// actions run from the actions menu, or from a `detail_<id>` binding.
pub(crate) const DETAIL_ACTIONS: &[&str] = &["review", "browser", "copy_url", "copy_change_id"];

/// The prefix of the binding names that run an action directly in the
/// pull request detail, for example `detail_merge`. They have no
/// default chord.
const DETAIL_PREFIX: &str = "detail_";

/// The default bindings, for the code that builds models and needs no
/// key.
pub(crate) static DEFAULT: LazyLock<Keymap> = LazyLock::new(Keymap::default);

/// A chord in the key map that does not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid key `{key}` for `{name}` in [keys]")]
pub(crate) struct KeymapError {
    pub name: String,
    pub key: String,
}

/// The chords of each binding name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Keymap {
    bindings: BTreeMap<String, Vec<KeyChord>>,
}

impl Default for Keymap {
    /// The default bindings of the app keys and of the actions of
    /// `REGISTRY`.
    fn default() -> Self {
        Self::defaults(&REGISTRY)
    }
}

impl Keymap {
    /// The default bindings of the app keys and of `actions`. An action
    /// without a default key has no chord.
    fn defaults(actions: &[Box<dyn Action>]) -> Self {
        let mut bindings: BTreeMap<String, Vec<KeyChord>> = DEFAULTS
            .iter()
            .map(|(name, keys)| {
                let chords = keys.iter().filter_map(|key| parse_chord(key)).collect();
                ((*name).to_owned(), chords)
            })
            .collect();
        for action in actions {
            let chords = action.default_key().into_iter().collect();
            bindings.insert(action.id().to_owned(), chords);
        }
        Self { bindings }
    }

    /// The default bindings of the app keys and of the actions of
    /// `REGISTRY`, with `overrides` (binding name to chords) applied. A
    /// name that has no default adds a binding.
    pub(crate) fn new(overrides: &BTreeMap<String, String>) -> Result<Self, KeymapError> {
        Self::with_actions(&REGISTRY, overrides)
    }

    /// The default bindings of the app keys and of `actions`, with
    /// `overrides` applied.
    pub(crate) fn with_actions(
        actions: &[Box<dyn Action>],
        overrides: &BTreeMap<String, String>,
    ) -> Result<Self, KeymapError> {
        let mut keymap = Self::defaults(actions);
        for (name, keys) in overrides {
            let chords = keys
                .split_whitespace()
                .map(|key| {
                    parse_chord(key).ok_or_else(|| KeymapError {
                        name: name.clone(),
                        key: key.to_owned(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            keymap.bindings.insert(name.clone(), chords);
        }
        Ok(keymap)
    }

    /// True when `chord` is one of the chords of the binding `name`.
    pub(crate) fn matches(&self, name: &str, chord: KeyChord) -> bool {
        self.bindings
            .get(name)
            .is_some_and(|chords| chords.contains(&chord))
    }

    /// The chords of the binding `name`. Empty when it has none.
    pub(crate) fn chords(&self, name: &str) -> &[KeyChord] {
        self.bindings.get(name).map_or(&[], Vec::as_slice)
    }

    /// The chords that run the action `id` directly in the pull request
    /// detail: the chords of `id` for an action of `DETAIL_ACTIONS`, then
    /// the chords of the binding `detail_<id>`.
    pub(crate) fn detail_chords(&self, id: &str) -> Vec<KeyChord> {
        let own = if DETAIL_ACTIONS.contains(&id) {
            self.chords(id)
        } else {
            &[]
        };
        own.iter()
            .chain(self.chords(&format!("{DETAIL_PREFIX}{id}")))
            .copied()
            .collect()
    }
}

/// The help section of `actions`: the chords of each action and its
/// label, in registry order.
pub(crate) fn action_help(keymap: &Keymap, actions: &[Box<dyn Action>]) -> HelpSection {
    let entries = actions
        .iter()
        .map(|action| {
            let keys: Vec<String> = keymap
                .chords(action.id())
                .iter()
                .map(|&chord| chord_text(chord))
                .collect();
            (keys.join(" "), action.label().to_owned())
        })
        .collect();
    HelpSection {
        title: "actions".to_owned(),
        entries,
    }
}

/// The text of a chord in the form that `parse_chord` reads, for example
/// `ctrl-d`, `A` or `enter`.
pub(crate) fn chord_text(chord: KeyChord) -> String {
    let mut text = String::new();
    for (modifier, name) in [
        (KeyModifiers::CONTROL, "ctrl-"),
        (KeyModifiers::ALT, "alt-"),
        (KeyModifiers::SHIFT, "shift-"),
    ] {
        if chord.mods.contains(modifier) {
            text.push_str(name);
        }
    }
    let key = match chord.code {
        KeyCode::Char(' ') => "space".to_owned(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".to_owned(),
        KeyCode::Esc => "esc".to_owned(),
        KeyCode::Tab => "tab".to_owned(),
        KeyCode::BackTab => "shift-tab".to_owned(),
        KeyCode::Backspace => "backspace".to_owned(),
        KeyCode::Delete => "delete".to_owned(),
        KeyCode::Insert => "insert".to_owned(),
        KeyCode::Home => "home".to_owned(),
        KeyCode::End => "end".to_owned(),
        KeyCode::PageUp => "pageup".to_owned(),
        KeyCode::PageDown => "pagedown".to_owned(),
        KeyCode::Up => "up".to_owned(),
        KeyCode::Down => "down".to_owned(),
        KeyCode::Left => "left".to_owned(),
        KeyCode::Right => "right".to_owned(),
        KeyCode::F(n) => format!("F{n}"),
        other => format!("{other:?}").to_lowercase(),
    };
    text.push_str(&key);
    text
}

/// The chord of a key event.
pub(crate) fn chord(event: &KeyEvent) -> KeyChord {
    normalize(event.code, event.modifiers)
}

/// Parses a chord like `q`, `A`, `ctrl-d`, `ctrl-alt-x`, `F5` or `enter`.
/// Modifier and key names ignore case. A single character is the
/// character itself, so `a` and `A` differ.
pub(crate) fn parse_chord(text: &str) -> Option<KeyChord> {
    let mut mods = KeyModifiers::NONE;
    let mut rest = text;
    while let Some((head, tail)) = rest.split_once('-')
        && !tail.is_empty()
    {
        mods |= match head.to_ascii_lowercase().as_str() {
            "ctrl" => KeyModifiers::CONTROL,
            "alt" => KeyModifiers::ALT,
            "shift" => KeyModifiers::SHIFT,
            _ => break,
        };
        rest = tail;
    }
    let code = key_code(rest)?;
    Some(normalize(code, mods))
}

/// The key code of a key name or a single character.
fn key_code(name: &str) -> Option<KeyCode> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(KeyCode::Char(c));
    }
    let lower = name.to_ascii_lowercase();
    let code = match lower.as_str() {
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        _ => {
            let n: u8 = lower.strip_prefix('f')?.parse().ok()?;
            if !(1..=12).contains(&n) {
                return None;
            }
            KeyCode::F(n)
        }
    };
    Some(code)
}

/// One form for chords that terminals report in different ways: a
/// character carries its case instead of `SHIFT`, and `shift-tab` is
/// `BackTab`.
fn normalize(code: KeyCode, mods: KeyModifiers) -> KeyChord {
    let shift = mods.contains(KeyModifiers::SHIFT);
    let code = match code {
        KeyCode::Char(c) if shift => KeyCode::Char(c.to_ascii_uppercase()),
        KeyCode::Tab if shift => KeyCode::BackTab,
        code => code,
    };
    let mods = match code {
        KeyCode::Char(_) | KeyCode::BackTab => mods - KeyModifiers::SHIFT,
        _ => mods,
    };
    KeyChord { code, mods }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyChord {
        KeyChord { code, mods }
    }

    #[test]
    fn parse_chord_cases() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        let alt = KeyModifiers::ALT;
        let cases = [
            ("q", key(KeyCode::Char('q'), none)),
            ("A", key(KeyCode::Char('A'), none)),
            ("shift-a", key(KeyCode::Char('A'), none)),
            ("G", key(KeyCode::Char('G'), none)),
            ("/", key(KeyCode::Char('/'), none)),
            ("?", key(KeyCode::Char('?'), none)),
            ("-", key(KeyCode::Char('-'), none)),
            ("1", key(KeyCode::Char('1'), none)),
            ("ctrl-d", key(KeyCode::Char('d'), ctrl)),
            ("Ctrl-D", key(KeyCode::Char('D'), ctrl)),
            ("ctrl--", key(KeyCode::Char('-'), ctrl)),
            ("alt-x", key(KeyCode::Char('x'), alt)),
            ("ctrl-alt-x", key(KeyCode::Char('x'), ctrl | alt)),
            ("F5", key(KeyCode::F(5), none)),
            ("f12", key(KeyCode::F(12), none)),
            ("ctrl-f1", key(KeyCode::F(1), ctrl)),
            ("enter", key(KeyCode::Enter, none)),
            ("Esc", key(KeyCode::Esc, none)),
            ("tab", key(KeyCode::Tab, none)),
            ("shift-tab", key(KeyCode::BackTab, none)),
            ("backtab", key(KeyCode::BackTab, none)),
            ("space", key(KeyCode::Char(' '), none)),
            ("pagedown", key(KeyCode::PageDown, none)),
            ("shift-up", key(KeyCode::Up, KeyModifiers::SHIFT)),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_chord(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn parse_chord_rejects_bad_keys() {
        for text in ["", "ctrl-", "foo", "f0", "f13", "a-b", "ctrl-foo", "meta-x"] {
            assert_eq!(parse_chord(text), None, "{text}");
        }
    }

    #[test]
    fn every_default_parses() {
        let keymap = Keymap::default();
        for (name, keys) in DEFAULTS {
            assert_eq!(keymap.bindings[*name].len(), keys.len(), "{name}");
        }
    }

    #[test]
    fn quit_defaults_to_q_esc_and_ctrl_c() {
        let keymap = Keymap::default();
        assert!(keymap.matches("quit", key(KeyCode::Char('q'), KeyModifiers::NONE)));
        assert!(keymap.matches("quit", key(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(keymap.matches("quit", key(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(!keymap.matches("quit", key(KeyCode::Char('c'), KeyModifiers::NONE)));
        assert!(!keymap.matches(
            "no_such_binding",
            key(KeyCode::Char('q'), KeyModifiers::NONE)
        ));
    }

    #[test]
    fn overrides_replace_add_and_remove() {
        let overrides = BTreeMap::from([
            ("quit".to_owned(), "ctrl-q  Q".to_owned()),
            ("review".to_owned(), "v".to_owned()),
            ("help".to_owned(), String::new()),
        ]);
        let keymap = Keymap::new(&overrides).unwrap();
        assert!(!keymap.matches("quit", key(KeyCode::Char('q'), KeyModifiers::NONE)));
        assert!(keymap.matches("quit", key(KeyCode::Char('q'), KeyModifiers::CONTROL)));
        assert!(keymap.matches("quit", key(KeyCode::Char('Q'), KeyModifiers::NONE)));
        assert!(keymap.matches("review", key(KeyCode::Char('v'), KeyModifiers::NONE)));
        assert!(!keymap.matches("help", key(KeyCode::Char('?'), KeyModifiers::NONE)));
        assert!(keymap.matches("down", key(KeyCode::Char('j'), KeyModifiers::NONE)));
    }

    #[test]
    fn overrides_report_a_bad_key() {
        let overrides = BTreeMap::from([("quit".to_owned(), "q ctrl-nope".to_owned())]);
        assert_eq!(
            Keymap::new(&overrides),
            Err(KeymapError {
                name: "quit".into(),
                key: "ctrl-nope".into(),
            })
        );
    }

    #[test]
    fn action_keys_come_from_the_actions_and_the_overrides() {
        use crate::actions::fake::FakeAction;

        let mut no_key = FakeAction::new("quiet", 'z');
        no_key.key = None;
        let actions: Vec<Box<dyn Action>> = vec![
            Box::new(FakeAction::new("merge", 'm')),
            Box::new(FakeAction::new("review", 'v')),
            Box::new(no_key),
        ];
        let none = KeyModifiers::NONE;
        let keymap = Keymap::with_actions(&actions, &BTreeMap::new()).unwrap();
        assert!(keymap.matches("merge", key(KeyCode::Char('m'), none)));
        assert!(keymap.matches("review", key(KeyCode::Char('v'), none)));
        assert_eq!(keymap.chords("quiet"), []);
        let overrides = BTreeMap::from([("merge".to_owned(), "M ctrl-m".to_owned())]);
        let keymap = Keymap::with_actions(&actions, &overrides).unwrap();
        assert!(!keymap.matches("merge", key(KeyCode::Char('m'), none)));
        assert!(keymap.matches("merge", key(KeyCode::Char('M'), none)));
        assert_eq!(
            action_help(&keymap, &actions),
            HelpSection {
                title: "actions".into(),
                entries: vec![
                    ("M ctrl-m".into(), "merge".into()),
                    ("v".into(), "review".into()),
                    (String::new(), "quiet".into()),
                ],
            }
        );
    }

    #[test]
    fn chord_text_reads_back() {
        for text in [
            "q",
            "A",
            "/",
            "ctrl-d",
            "alt-x",
            "ctrl-alt-x",
            "F5",
            "enter",
            "esc",
            "tab",
            "shift-tab",
            "space",
            "pagedown",
            "shift-up",
            "backspace",
        ] {
            let chord = parse_chord(text).unwrap();
            assert_eq!(chord_text(chord), text);
            assert_eq!(parse_chord(&chord_text(chord)), Some(chord), "{text}");
        }
    }

    #[test]
    fn chord_of_an_event_is_normalized() {
        let cases = [
            (KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT), "A"),
            (KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT), "A"),
            (
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                "shift-tab",
            ),
            (
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                "ctrl-c",
            ),
            (KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE), "F5"),
        ];
        for (event, text) in cases {
            assert_eq!(Some(chord(&event)), parse_chord(text), "{text}");
        }
    }
}
