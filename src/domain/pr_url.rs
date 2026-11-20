//! The parser of a GitHub pull request URL, for example
//! `https://github.com/chaosteil/chaoshub/pull/7/files`.
//!
//! A URL from a chat often has extra text around it. Slack, for example,
//! puts a link in `<...>`, and its raw text can have a label after a `|`.
//! The parser removes the spaces, the angle brackets and the label. The
//! scheme (`https://` or `http://`) and `www.` are optional. The URL can
//! end in a tail (`/files`, `/changes`, `/commits`, `/checks`), a query
//! (`?w=1`) and an anchor (`#issuecomment-1`).

use crate::domain::keys::{PrKey, RepoKey};

/// The pane of the PR view that a URL opens first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FirstPane {
    Description,
    Checks,
    Conversation,
    Files,
}

impl FirstPane {
    /// The id of the pane, as `Pane::id` gives it.
    pub(crate) fn pane_id(self) -> &'static str {
        match self {
            Self::Description => "description",
            Self::Checks => "checks",
            Self::Conversation => "conversation",
            Self::Files => "files",
        }
    }
}

/// A parsed pull request URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrUrl {
    pub key: PrKey,
    /// The pane that the tail or the anchor of the URL selects.
    pub pane: FirstPane,
    /// The URL without the spaces, the angle brackets and the label
    /// around it. `parse` gives the same result for this text.
    pub text: String,
}

/// The anchors of a comment, a review comment and a review. They open
/// the Conversation pane.
const CONVERSATION_ANCHORS: &[&str] = &["discussion_r", "issuecomment-", "pullrequestreview-"];

/// Parses a GitHub pull request URL. `None` when `text` is not one.
pub(crate) fn parse(text: &str) -> Option<PrUrl> {
    let text = unwrap(text);
    let (rest, anchor) = match text.split_once('#') {
        Some((rest, anchor)) => (rest, Some(anchor)),
        None => (text, None),
    };
    let rest = rest.split_once('?').map_or(rest, |(rest, _)| rest);
    let rest = strip_prefix_ignore_case(rest, "https://")
        .or_else(|| strip_prefix_ignore_case(rest, "http://"))
        .unwrap_or(rest);
    let rest = strip_prefix_ignore_case(rest, "www.").unwrap_or(rest);
    let path = strip_prefix_ignore_case(rest, "github.com/")?;
    let path = path.strip_suffix('/').unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next().filter(|part| is_name(part))?;
    let name = parts.next().filter(|part| is_name(part))?;
    if parts.next() != Some("pull") {
        return None;
    }
    let number = parts
        .next()
        .filter(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|part| part.parse::<u64>().ok())
        .filter(|&number| number > 0)?;
    let tail_pane = match parts.next() {
        None | Some("commits") => FirstPane::Description,
        Some("files" | "changes") => FirstPane::Files,
        Some("checks") => FirstPane::Checks,
        Some(_) => return None,
    };
    if parts.any(str::is_empty) {
        return None;
    }
    let conversation = anchor.is_some_and(|anchor| {
        CONVERSATION_ANCHORS
            .iter()
            .any(|prefix| anchor.starts_with(prefix))
    });
    Some(PrUrl {
        key: PrKey {
            repo: RepoKey {
                owner: owner.to_owned(),
                name: name.to_owned(),
            },
            number,
        },
        pane: if conversation {
            FirstPane::Conversation
        } else {
            tail_pane
        },
        text: text.to_owned(),
    })
}

/// `text` without the spaces around it, without one pair of angle
/// brackets, and without a label after `|` in the brackets.
fn unwrap(text: &str) -> &str {
    let text = text.trim();
    let Some(inner) = text
        .strip_prefix('<')
        .and_then(|inner| inner.strip_suffix('>'))
    else {
        return text;
    };
    inner.split_once('|').map_or(inner, |(url, _)| url).trim()
}

/// `text` without `prefix`, when `text` starts with `prefix` in any case.
fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// True for an owner or a repo name: letters, digits, `-`, `_` and `.`.
fn is_name(part: &str) -> bool {
    !part.is_empty()
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(owner: &str, name: &str, number: u64) -> PrKey {
        PrKey {
            repo: RepoKey {
                owner: owner.into(),
                name: name.into(),
            },
            number,
        }
    }

    /// The key and the pane of `text`.
    fn parsed(text: &str) -> Option<(PrKey, FirstPane)> {
        parse(text).map(|url| (url.key, url.pane))
    }

    #[test]
    fn a_plain_url_opens_the_description() {
        let base = key("chaosteil", "chaoshub", 7);
        for text in [
            "https://github.com/chaosteil/chaoshub/pull/7",
            "https://github.com/chaosteil/chaoshub/pull/7/",
            "http://github.com/chaosteil/chaoshub/pull/7",
            "github.com/chaosteil/chaoshub/pull/7",
            "www.github.com/chaosteil/chaoshub/pull/7",
            "https://www.github.com/chaosteil/chaoshub/pull/7",
            "HTTPS://GitHub.com/chaosteil/chaoshub/pull/7",
            "  https://github.com/chaosteil/chaoshub/pull/7 \n",
            "<https://github.com/chaosteil/chaoshub/pull/7>",
            " < https://github.com/chaosteil/chaoshub/pull/7 > ",
            "<https://github.com/chaosteil/chaoshub/pull/7|chaoshub#7>",
            "https://github.com/chaosteil/chaoshub/pull/7?w=1",
            "https://github.com/chaosteil/chaoshub/pull/7#top",
            "https://github.com/chaosteil/chaoshub/pull/7/commits",
            "https://github.com/chaosteil/chaoshub/pull/7/commits/0a1b2c3",
        ] {
            assert_eq!(
                parsed(text),
                Some((base.clone(), FirstPane::Description)),
                "{text}"
            );
        }
    }

    #[test]
    fn the_tail_picks_the_pane() {
        let base = key("chaosteil", "chaoshub", 7);
        let cases = [
            ("/files", FirstPane::Files),
            ("/files/", FirstPane::Files),
            ("/files?diff=split", FirstPane::Files),
            ("/files#diff-0123abcd", FirstPane::Files),
            ("/files/0a1b..2c3d", FirstPane::Files),
            ("/changes", FirstPane::Files),
            ("/changes/0a1b2c3", FirstPane::Files),
            ("/checks", FirstPane::Checks),
            ("/checks?check_run_id=12", FirstPane::Checks),
            ("#discussion_r1234", FirstPane::Conversation),
            ("#issuecomment-1234", FirstPane::Conversation),
            ("#pullrequestreview-1234", FirstPane::Conversation),
            ("/files#discussion_r1234", FirstPane::Conversation),
            (
                "?notification_referrer_id=x#issuecomment-1",
                FirstPane::Conversation,
            ),
            ("#issue-1234", FirstPane::Description),
        ];
        for (tail, pane) in cases {
            let text = format!("https://github.com/chaosteil/chaoshub/pull/7{tail}");
            assert_eq!(parsed(&text), Some((base.clone(), pane)), "{text}");
        }
    }

    #[test]
    fn names_keep_their_case_and_their_signs() {
        assert_eq!(
            parsed("https://github.com/Some-Org/my_repo.rs/pull/1234"),
            Some((key("Some-Org", "my_repo.rs", 1234), FirstPane::Description))
        );
    }

    #[test]
    fn the_text_is_the_url_without_the_wrapping() {
        let url = parse(" <github.com/a/b/pull/3/files|label> ").unwrap();
        assert_eq!(url.text, "github.com/a/b/pull/3/files");
        assert_eq!(parse(&url.text), Some(url));
    }

    #[test]
    fn other_text_is_not_a_pr_url() {
        for text in [
            "",
            "   ",
            "7",
            "chaosteil/chaoshub",
            "chaosteil/chaoshub 7",
            "chaosteil/chaoshub#7",
            "https://github.com",
            "https://github.com/chaosteil/chaoshub",
            "https://github.com/chaosteil/chaoshub/pulls",
            "https://github.com/chaosteil/chaoshub/pull",
            "https://github.com/chaosteil/chaoshub/pull/",
            "https://github.com/chaosteil/chaoshub/pull/0",
            "https://github.com/chaosteil/chaoshub/pull/-7",
            "https://github.com/chaosteil/chaoshub/pull/+7",
            "https://github.com/chaosteil/chaoshub/pull/7x",
            "https://github.com/chaosteil/chaoshub/pull/99999999999999999999999",
            "https://github.com/chaosteil/chaoshub/issues/7",
            "https://github.com/chaosteil/chaoshub/pull/7/review",
            "https://github.com/chaosteil/chaoshub/pull/7/files//x",
            "https://github.com/chaosteil/chaoshub/pull/7//",
            "https://github.com//chaoshub/pull/7",
            "https://github.com/chaos teil/chaoshub/pull/7",
            "https://gitlab.com/chaosteil/chaoshub/pull/7",
            "https://github.company.com/chaosteil/chaoshub/pull/7",
            "https://notgithub.com/chaosteil/chaoshub/pull/7",
            "https://api.github.com/repos/chaosteil/chaoshub/pulls/7",
            "ftp://github.com/chaosteil/chaoshub/pull/7",
            "see https://github.com/chaosteil/chaoshub/pull/7",
            "https://github.com/chaosteil/chaoshub/pull/7 and more",
            "<https://github.com/chaosteil/chaoshub/pull/7",
            "<>",
            "https://github.com/chaosteil/chaoshub/pull/7\nhttps://github.com/a/b/pull/1",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn each_pane_has_the_id_of_its_pane() {
        assert_eq!(FirstPane::Description.pane_id(), "description");
        assert_eq!(FirstPane::Checks.pane_id(), "checks");
        assert_eq!(FirstPane::Conversation.pane_id(), "conversation");
        assert_eq!(FirstPane::Files.pane_id(), "files");
    }
}
