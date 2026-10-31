//! The key map: defaults, overrides and action lookup.
//!
//! A binding is a name, for example `quit`, and one or more key chords. A
//! chord is written like `q`, `A`, `ctrl-d`, `alt-x`, `F5`, `enter` or
//! `shift-tab`. The overrides map a binding name to chords separated by
//! spaces. An override replaces every default chord of that name. An
//! empty value removes the binding.

use std::collections::BTreeMap;

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
    ("open", &["enter"]),
    ("close_overlay", &["esc", "q"]),
    ("filter", &["/"]),
    ("authors", &["a"]),
    ("toggle_all", &["t"]),
    ("refresh", &["r"]),
    ("help", &["?"]),
    ("quit", &["q", "ctrl-c"]),
];

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
    /// The default bindings.
    fn default() -> Self {
        let bindings = DEFAULTS
            .iter()
            .map(|(name, keys)| {
                let chords = keys.iter().filter_map(|key| parse_chord(key)).collect();
                ((*name).to_owned(), chords)
            })
            .collect();
        Self { bindings }
    }
}

impl Keymap {
    /// The default bindings with `overrides` (binding name to chords)
    /// applied. A name that has no default adds a binding.
    pub(crate) fn new(overrides: &BTreeMap<String, String>) -> Result<Self, KeymapError> {
        let mut keymap = Self::default();
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
    fn quit_defaults_to_q_and_ctrl_c() {
        let keymap = Keymap::default();
        assert!(keymap.matches("quit", key(KeyCode::Char('q'), KeyModifiers::NONE)));
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
