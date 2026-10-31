//! Clipboard access through arboard, with an OSC 52 fallback.
//!
//! arboard writes to the system clipboard. It can fail, for example over
//! ssh or under a terminal multiplexer without a display. Then the text
//! goes to the terminal as an OSC 52 sequence, and a terminal that
//! supports it puts the text on its clipboard.

use std::io::{self, Write};

use crate::osc;

/// How the text reached the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Copied {
    /// Through arboard.
    System,
    /// Through OSC 52 on the terminal.
    Terminal,
}

/// Puts `text` on the clipboard: through arboard, else through OSC 52 on
/// stdout.
pub(crate) fn copy(text: &str) -> io::Result<Copied> {
    copy_with(text, system, &mut io::stdout().lock())
}

/// Puts `text` on the system clipboard with arboard.
fn system(text: &str) -> Result<(), arboard::Error> {
    arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text))
}

/// Puts `text` on the clipboard with `set`. When `set` fails, writes the
/// OSC 52 sequence of `text` to `out` in one write.
fn copy_with<E: std::fmt::Display>(
    text: &str,
    set: impl FnOnce(&str) -> Result<(), E>,
    out: &mut impl Write,
) -> io::Result<Copied> {
    match set(text) {
        Ok(()) => Ok(Copied::System),
        Err(err) => {
            tracing::debug!(%err, "no system clipboard, using OSC 52");
            out.write_all(&osc::clipboard(text))?;
            out.flush()?;
            Ok(Copied::Terminal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clipboard_comes_first() {
        let mut out = Vec::new();
        let copied = copy_with("abc", |_| Ok::<(), String>(()), &mut out).unwrap();
        assert_eq!(copied, Copied::System);
        assert_eq!(out, b"");
    }

    #[test]
    fn a_failed_system_clipboard_writes_osc_52() {
        let mut out = Vec::new();
        let copied = copy_with(
            "https://github.com/chaosteil/chaoshub/pull/7",
            |_| Err("no display"),
            &mut out,
        )
        .unwrap();
        assert_eq!(copied, Copied::Terminal);
        assert_eq!(
            out,
            b"\x1b]52;c;aHR0cHM6Ly9naXRodWIuY29tL2NoYW9zdGVpbC9jaGFvc2h1Yi9wdWxsLzc=\x1b\\"
        );
    }

    #[test]
    fn a_change_id_in_osc_52() {
        let mut out = Vec::new();
        copy_with("zzmqxkrtzzmq", |_| Err("no display"), &mut out).unwrap();
        assert_eq!(out, b"\x1b]52;c;enptcXhrcnR6em1x\x1b\\");
    }
}
