//! Pure builders for the OSC 2, OSC 7501, OSC 9;4 and OSC 52 byte sequences.
//! No IO.
//!
//! - OSC 2 (terminal title): `ESC ] 2 ; <text> ESC \`.
//! - OSC 7501 (program status):
//!   `ESC ] 7501 ; state=<state>[:kind=<kind>]:app=chaoshub[:msg=<base64>] ESC \`.
//!   A child record also has an `id` and a `title`:
//!   `ESC ] 7501 ; state=done:id=<id>:app=chaoshub:title=<base64>:msg=<base64> ESC \`.
//! - OSC 9;4 (progress bar): `ESC ] 9 ; 4 ; <s>[;<v>] ESC \`.
//! - OSC 777 (desktop notification): `ESC ] 777 ; notify ; <title> ; <body> ESC \`.
//! - OSC 52 (clipboard): `ESC ] 52 ; c ; <base64> ESC \`.
//! - OSC 7577 (program resume):
//!   `ESC ] 7577 ; pid=<pid>:time=<secs>:argv=<b64>,<b64>[:cwd=<b64>]:mac=<hex> ESC \`,
//!   or `clear=1` in place of `argv`. `mac` is the SHA-256 of the body
//!   before `:mac=`, a `:` and the token.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

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

/// Sets the terminal title. An empty text clears it. Replace control
/// characters and limit the length, as for a program status title.
pub(crate) fn title(text: &str) -> Vec<u8> {
    sequence(&format!("2;{}", field_text(text, TITLE_MAX)))
}

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

/// The OSC 777 sequence of a desktop notification with `title` and
/// `body`, as plain text. A control character becomes a space, and a
/// text that is too long is cut at a character boundary, the same as in
/// `record_done`. A `;` in `title` becomes a space too, because the
/// first `;` after the title starts the body.
pub(crate) fn desktop_notification(title: &str, body: &str) -> Vec<u8> {
    sequence(&format!(
        "777;notify;{};{}",
        field_text(title, TITLE_MAX).replace(';', " "),
        field_text(body, MSG_MAX),
    ))
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

/// The command of an OSC 7577 report.
#[allow(dead_code, reason = "the resume wiring in run uses it")]
pub(crate) enum ResumeBody<'a> {
    /// The command that starts the program again, and its directory.
    Argv {
        argv: &'a [String],
        cwd: Option<&'a str>,
    },
    /// Removes the last report.
    Clear,
}

/// The maximum length of an OSC 7577 report, in bytes. The terminal
/// always accepts a report of this length.
#[allow(dead_code, reason = "the resume wiring in run uses it")]
pub(crate) const RESUME_MAX_BYTES: usize = 2048;
/// The maximum number of `argv` elements of an OSC 7577 report.
#[allow(dead_code, reason = "the resume wiring in run uses it")]
pub(crate) const RESUME_MAX_ARGV: usize = 64;

/// The OSC 7577 sequence that reports the resume command of the process
/// `pid` at the time `time` (in seconds since the epoch). `token` signs
/// the report. The result is `None` when the terminal would discard the
/// report: an empty or too long `argv`, an `argv[0]` that is not a bare
/// name or an absolute path, a control character, a `cwd` that is not
/// absolute, or a report of more than `RESUME_MAX_BYTES` bytes.
#[allow(dead_code, reason = "the resume wiring in run uses it")]
pub(crate) fn program_resume(
    pid: u32,
    time: u64,
    body: &ResumeBody<'_>,
    token: &str,
) -> Option<Vec<u8>> {
    let mut text = format!("pid={pid}:time={time}");
    match body {
        ResumeBody::Argv { argv, cwd } => {
            let first = argv.first()?;
            if argv.len() > RESUME_MAX_ARGV
                || first.is_empty()
                || first.starts_with('-')
                || (first.contains('/') && !first.starts_with('/'))
                || argv.iter().any(|arg| has_control(arg))
            {
                return None;
            }
            let encoded: Vec<String> = argv.iter().map(|arg| base64(arg.as_bytes())).collect();
            text.push_str(":argv=");
            text.push_str(&encoded.join(","));
            if let Some(cwd) = cwd {
                if !cwd.starts_with('/') || has_control(cwd) {
                    return None;
                }
                text.push_str(":cwd=");
                text.push_str(&base64(cwd.as_bytes()));
            }
        }
        ResumeBody::Clear => text.push_str(":clear=1"),
    }
    let mac = resume_mac(&text, token);
    let out = sequence(&format!("7577;{text}:mac={mac}"));
    (out.len() <= RESUME_MAX_BYTES).then_some(out)
}

/// True when `text` has a C0 control character, DEL or a C1 control
/// character.
fn has_control(text: &str) -> bool {
    text.chars().any(char::is_control)
}

/// The lowercase hex of the SHA-256 of `body`, a `:` and `token`.
fn resume_mac(body: &str, token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    hasher.update(b":");
    hasher.update(token.as_bytes());
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
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
    fn desktop_notification_has_a_title_and_a_body() {
        assert_eq!(
            desktop_notification("chaoshub", "#12 fix: a; b (alice)"),
            "\x1b]777;notify;chaoshub;#12 fix: a; b (alice)\x1b\\".as_bytes()
        );
    }

    #[test]
    fn desktop_notification_cannot_end_the_sequence_early() {
        assert_eq!(
            desktop_notification("a;b\x1b", "x\x1b\\y\u{9c}z\n\u{7}"),
            "\x1b]777;notify;a b ;x \\y z  \x1b\\".as_bytes()
        );
    }

    #[test]
    fn desktop_notification_respects_the_length_limits() {
        let out = desktop_notification(&"ü".repeat(200), &"€".repeat(1000));
        let expected = format!(
            "\x1b]777;notify;{};{}\x1b\\",
            "ü".repeat(96),
            "€".repeat(682)
        );
        assert_eq!(out, expected.as_bytes());
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

    /// The token of the spec vectors.
    const TOKEN: &str = "dummy-token-replace-me";

    fn args(argv: &[&str]) -> Vec<String> {
        argv.iter().map(|&arg| arg.to_owned()).collect()
    }

    fn resume(argv: &[String], cwd: Option<&str>) -> Option<Vec<u8>> {
        program_resume(74877, 1_795_543_200, &ResumeBody::Argv { argv, cwd }, TOKEN)
    }

    #[test]
    fn program_resume_clear_matches_the_spec_vector() {
        assert_eq!(
            program_resume(74877, 1_795_543_200, &ResumeBody::Clear, TOKEN).unwrap(),
            b"\x1b]7577;pid=74877:time=1795543200:clear=1:mac=c30603369f98fef581a4098be889175b0c74f0641e49f8b25f4326cb38f97c93\x1b\\"
        );
    }

    #[test]
    fn program_resume_argv_matches_the_spec_vector() {
        let argv = args(&[
            "nvim",
            "-S",
            "/home/user/.local/state/nvim/sessions/8f2c1d.vim",
        ]);
        assert_eq!(
            resume(&argv, None).unwrap(),
            b"\x1b]7577;pid=74877:time=1795543200:argv=bnZpbQ==,LVM=,L2hvbWUvdXNlci8ubG9jYWwvc3RhdGUvbnZpbS9zZXNzaW9ucy84ZjJjMWQudmlt:mac=9dbb99bfb856749a87563839d7ce9062818d78cf25909db93f309e0cd3b1909b\x1b\\"
        );
    }

    #[test]
    fn program_resume_cwd_matches_the_spec_vector() {
        let argv = args(&["lazygit"]);
        assert_eq!(
            resume(&argv, Some("/home/user/src/project")).unwrap(),
            b"\x1b]7577;pid=74877:time=1795543200:argv=bGF6eWdpdA==:cwd=L2hvbWUvdXNlci9zcmMvcHJvamVjdA==:mac=35cb909805c380336efd3689a125acc774a9dec8422a1d21492ebec06ca758e1\x1b\\"
        );
    }

    #[test]
    fn program_resume_rejects_bad_commands() {
        let too_many: Vec<String> = (0..=RESUME_MAX_ARGV).map(|i| format!("a{i}")).collect();
        let cases: [(Vec<String>, Option<&str>); 7] = [
            (args(&["./run.sh"]), None),
            (args(&["-x"]), None),
            (Vec::new(), None),
            (args(&["nvim", "a\nb"]), None),
            (args(&["nvim", "a\u{85}b"]), None),
            (args(&["nvim"]), Some("project")),
            (too_many, None),
        ];
        for (argv, cwd) in cases {
            assert_eq!(resume(&argv, cwd), None, "{argv:?} {cwd:?}");
        }
        assert!(resume(&args(&["/usr/bin/nvim", "-x"]), Some("/tmp")).is_some());
    }

    #[test]
    fn program_resume_respects_the_length_limit() {
        // Each 3 bytes of the first element add 4 bytes, and each empty
        // element adds 1 byte (a ","), so the empty elements fill the gap.
        let len = |argv: &[String]| resume(argv, None).map(|out| out.len());
        let mut n = 1;
        while len(&["a".repeat(n + 3)]).is_some() {
            n += 3;
        }
        let mut argv = vec!["a".repeat(n)];
        while len(&argv).unwrap() < RESUME_MAX_BYTES {
            argv.push(String::new());
        }
        assert_eq!(len(&argv), Some(RESUME_MAX_BYTES));
        argv.push(String::new());
        assert_eq!(len(&argv), None);
    }

    #[test]
    fn program_resume_has_only_value_characters() {
        let argv = args(&["/usr/bin/chaoshub", "--repo", "a/b", "ü€ \u{1f600}", "?>~"]);
        let out = resume(&argv, Some("/home/user/ü")).unwrap();
        let text = String::from_utf8(out).unwrap();
        let body = text
            .strip_prefix("\x1b]7577;")
            .unwrap()
            .strip_suffix("\x1b\\")
            .unwrap();
        for pair in body.split(':') {
            let (key, value) = pair.split_once('=').unwrap();
            assert!(key.bytes().all(|b| b.is_ascii_lowercase()), "{key}");
            assert!(
                value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.,+/=-".contains(&b)),
                "{pair}"
            );
        }
    }

    #[test]
    fn title_sequences_set_and_clear_the_title() {
        assert_eq!(title("chaoshub"), b"\x1b]2;chaoshub\x1b\\");
        assert_eq!(title(""), b"\x1b]2;\x1b\\");
    }

    #[test]
    fn terminal_titles_cannot_add_control_sequences() {
        assert_eq!(
            title("a\x1b]52;c;bad\x07\nc"),
            b"\x1b]2;a ]52;c;bad  c\x1b\\"
        );
        let long = "ü".repeat(200);
        assert_eq!(
            title(&long),
            format!("\x1b]2;{}\x1b\\", "ü".repeat(96)).into_bytes()
        );
    }
}
