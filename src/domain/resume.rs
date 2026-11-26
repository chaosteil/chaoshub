//! The state that a resumed chaoshub reads back, for OSC 7577 (program
//! resume). No IO.

/// The env that the terminal sets when it supports OSC 7577. chaoshub
/// never gives it to a child.
pub(crate) const TOKEN_ENV: &str = "OSC7577_TOKEN";
