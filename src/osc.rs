//! Pure builders for the OSC 7501, OSC 9;4 and OSC 52 byte sequences.
//! No IO.
//!
//! - OSC 7501 (program status):
//!   `ESC ] 7501 ; state=<state>[:kind=<kind>]:app=chaoshub[:msg=<base64>] ESC \`.
//!   A child record also has an `id` and a `title`:
//!   `ESC ] 7501 ; state=done:id=<id>:app=chaoshub:title=<base64>:msg=<base64> ESC \`.
//! - OSC 9;4 (progress bar): `ESC ] 9 ; 4 ; <s>[;<v>] ESC \`.
//! - OSC 52 (clipboard): `ESC ] 52 ; c ; <base64> ESC \`.

use crate::bus::{BlockedKind, Progress, TermState, TermStatus};

/// The start of an operating system command.
const OSC: &[u8] = b"\x1b]";
/// The string terminator.
const ST: &[u8] = b"\x1b\\";
/// The `app` field of every OSC 7501 record.
const APP: &str = "chaoshub";
/// The maximum length of a decoded `title`, in bytes.
const TITLE_MAX: usize = 192;
/// The maximum length of a decoded `msg`, in bytes.
const MSG_MAX: usize = 2048;

/// The OSC 7501 sequence for a program status. `msg` is base64 without
/// line breaks.
pub(crate) fn program_status(status: &TermStatus) -> Vec<u8> {
    let (state, kind) = match status.state {
        TermState::Idle => ("idle", None),
        TermState::Working => ("working", None),
        TermState::Blocked { kind } => ("blocked", Some(kind)),
        TermState::Done => ("done", None),
        TermState::Error => ("error", None),
        TermState::Clear => ("clear", None),
    };
    let mut body = format!("7501;state={state}");
    if let Some(kind) = kind {
        body.push_str(":kind=");
        body.push_str(match kind {
            BlockedKind::Permission => "permission",
            BlockedKind::Question => "question",
            BlockedKind::Auth => "auth",
        });
    }
    body.push_str(":app=");
    body.push_str(APP);
    if let Some(msg) = &status.msg {
        body.push_str(":msg=");
        body.push_str(&base64(msg.as_bytes()));
    }
    sequence(&body)
}

/// The OSC 7501 sequence that sets the child record `id` to `done`, with
/// a `title` and a `msg`. A control character becomes a space, and a
/// text that is too long is cut at a character boundary, because the
/// terminal discards a report with such a text.
pub(crate) fn record_done(id: &str, title: &str, msg: &str) -> Vec<u8> {
    sequence(&format!(
        "7501;state=done:id={id}:app={APP}:title={}:msg={}",
        base64(field_text(title, TITLE_MAX).as_bytes()),
        base64(field_text(msg, MSG_MAX).as_bytes()),
    ))
}

/// The OSC 7501 sequence that removes the child record `id` and the
/// records below it.
pub(crate) fn record_clear(id: &str) -> Vec<u8> {
    sequence(&format!("7501;state=clear:id={id}:app={APP}"))
}

/// `text` without control characters and with at most `max` bytes. The
/// cut is at a character boundary.
fn field_text(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max));
    for c in text.chars().map(|c| if c.is_control() { ' ' } else { c }) {
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
}

/// The OSC 9;4 sequence for a progress state. A percent above 100 counts
/// as 100.
pub(crate) fn progress(progress: Progress) -> Vec<u8> {
    let body = match progress {
        Progress::Off => "9;4;0".to_owned(),
        Progress::Percent(pct) => format!("9;4;1;{}", pct.min(100)),
        Progress::Error => "9;4;2".to_owned(),
        Progress::Indeterminate => "9;4;3".to_owned(),
        Progress::Paused => "9;4;4".to_owned(),
    };
    sequence(&body)
}

/// The sequences that clear the program status and the progress bar:
/// `state=clear` and `9;4;0`.
pub(crate) fn clear() -> Vec<u8> {
    let mut out = program_status(&TermStatus {
        state: TermState::Clear,
        msg: None,
        progress: Progress::Off,
    });
    out.extend(progress(Progress::Off));
    out
}

/// The OSC 52 sequence that puts `text` on the system clipboard.
#[allow(dead_code, reason = "the clipboard fallback of phase E writes it")]
pub(crate) fn clipboard(text: &str) -> Vec<u8> {
    sequence(&format!("52;c;{}", base64(text.as_bytes())))
}

/// `body` between the OSC start and the string terminator.
fn sequence(body: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(OSC.len() + body.len() + ST.len());
    out.extend_from_slice(OSC);
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(ST);
    out
}

/// Standard base64 with padding and without line breaks.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = (n >> (18 - 6 * i)) & 0x3f;
                out.push(char::from(ALPHABET[index as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: TermState, msg: Option<&str>) -> TermStatus {
        TermStatus {
            state,
            msg: msg.map(str::to_owned),
            progress: Progress::Off,
        }
    }

    #[test]
    fn base64_cases() {
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("foobar", "Zm9vYmFy"),
            ("merged #42", "bWVyZ2VkICM0Mg=="),
        ];
        for (input, expected) in cases {
            assert_eq!(base64(input.as_bytes()), expected, "{input:?}");
        }
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn program_status_for_each_state() {
        let cases: [(TermStatus, &[u8]); 8] = [
            (
                status(TermState::Idle, None),
                b"\x1b]7501;state=idle:app=chaoshub\x1b\\",
            ),
            (
                status(TermState::Working, Some("syncing 3 repos")),
                b"\x1b]7501;state=working:app=chaoshub:msg=c3luY2luZyAzIHJlcG9z\x1b\\",
            ),
            (
                status(
                    TermState::Blocked {
                        kind: BlockedKind::Question,
                    },
                    Some("merge #42"),
                ),
                b"\x1b]7501;state=blocked:kind=question:app=chaoshub:msg=bWVyZ2UgIzQy\x1b\\",
            ),
            (
                status(
                    TermState::Blocked {
                        kind: BlockedKind::Permission,
                    },
                    Some("x"),
                ),
                b"\x1b]7501;state=blocked:kind=permission:app=chaoshub:msg=eA==\x1b\\",
            ),
            (
                status(
                    TermState::Blocked {
                        kind: BlockedKind::Auth,
                    },
                    Some("x"),
                ),
                b"\x1b]7501;state=blocked:kind=auth:app=chaoshub:msg=eA==\x1b\\",
            ),
            (
                status(TermState::Done, Some("merged #42")),
                b"\x1b]7501;state=done:app=chaoshub:msg=bWVyZ2VkICM0Mg==\x1b\\",
            ),
            (
                status(TermState::Error, Some("offline")),
                b"\x1b]7501;state=error:app=chaoshub:msg=b2ZmbGluZQ==\x1b\\",
            ),
            (
                status(TermState::Clear, None),
                b"\x1b]7501;state=clear:app=chaoshub\x1b\\",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(
                program_status(&input),
                expected,
                "{input:?}: {:?}",
                String::from_utf8_lossy(&program_status(&input))
            );
        }
    }

    #[test]
    fn program_status_msg_has_no_line_breaks() {
        let long = "a".repeat(200);
        let out = program_status(&status(TermState::Working, Some(&long)));
        assert!(!out.contains(&b'\n'));
        assert!(!out.contains(&b'\r'));
    }

    #[test]
    fn record_done_has_an_id_a_title_and_a_msg() {
        assert_eq!(
            record_done("new-prs", "chaoshub", "3 new PRs in team"),
            b"\x1b]7501;state=done:id=new-prs:app=chaoshub:title=Y2hhb3NodWI=:msg=MyBuZXcgUFJzIGluIHRlYW0=\x1b\\"
        );
    }

    #[test]
    fn record_clear_names_the_id() {
        assert_eq!(
            record_clear("new-prs"),
            b"\x1b]7501;state=clear:id=new-prs:app=chaoshub\x1b\\"
        );
    }

    #[test]
    fn field_text_replaces_control_characters() {
        assert_eq!(field_text("a\tb\nc\u{7f}d", 100), "a b c d");
    }

    #[test]
    fn field_text_cuts_at_a_character_boundary() {
        assert_eq!(field_text("abc", 3), "abc");
        assert_eq!(field_text("abcd", 3), "abc");
        // "é" is 2 bytes: a cut after 3 bytes keeps "aé" only.
        assert_eq!(field_text("aéé", 4), "aé");
        assert_eq!(field_text("aéé", 3), "aé");
        assert_eq!(field_text("aéé", 2), "a");
    }

    #[test]
    fn record_done_respects_the_length_limits() {
        let title = "ü".repeat(200);
        let msg = "€".repeat(1000);
        let out = record_done("new-prs", &title, &msg);
        let text = String::from_utf8(out.clone()).unwrap();
        let field = |key: &str| {
            text.split(':')
                .find_map(|pair| pair.strip_prefix(key))
                .unwrap()
                .trim_end_matches("\x1b\\")
                .to_owned()
        };
        assert_eq!(field("title="), base64("ü".repeat(96).as_bytes()));
        assert_eq!(field("msg="), base64("€".repeat(682).as_bytes()));
        assert!(out.len() <= 4096);
    }

    #[test]
    fn progress_for_each_state() {
        let cases: [(Progress, &[u8]); 8] = [
            (Progress::Off, b"\x1b]9;4;0\x1b\\"),
            (Progress::Percent(0), b"\x1b]9;4;1;0\x1b\\"),
            (Progress::Percent(42), b"\x1b]9;4;1;42\x1b\\"),
            (Progress::Percent(100), b"\x1b]9;4;1;100\x1b\\"),
            (Progress::Percent(250), b"\x1b]9;4;1;100\x1b\\"),
            (Progress::Error, b"\x1b]9;4;2\x1b\\"),
            (Progress::Indeterminate, b"\x1b]9;4;3\x1b\\"),
            (Progress::Paused, b"\x1b]9;4;4\x1b\\"),
        ];
        for (input, expected) in cases {
            assert_eq!(progress(input), expected, "{input:?}");
        }
    }

    #[test]
    fn clear_clears_both() {
        assert_eq!(
            clear(),
            b"\x1b]7501;state=clear:app=chaoshub\x1b\\\x1b]9;4;0\x1b\\"
        );
    }

    #[test]
    fn clipboard_sequence() {
        assert_eq!(
            clipboard("https://github.com/chaosteil/chaoshub/pull/1"),
            b"\x1b]52;c;aHR0cHM6Ly9naXRodWIuY29tL2NoYW9zdGVpbC9jaGFvc2h1Yi9wdWxsLzE=\x1b\\"
        );
        assert_eq!(clipboard(""), b"\x1b]52;c;\x1b\\");
    }
}
