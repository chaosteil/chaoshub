//! The conversation of a pull request: comments, reviews and threads.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The conversation of a pull request, read at one head commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PrDetail {
    pub comments: Vec<Comment>,
    pub reviews: Vec<Review>,
    pub threads: Vec<Thread>,
    /// The reactions on the pull request itself, in the order of GitHub.
    /// A cached detail without them has none until the next load.
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    /// The head commit when the detail was read. A different head of the
    /// pull request makes the detail stale.
    pub head_oid: String,
}

/// An issue comment, or a comment in a review thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Comment {
    /// The GraphQL node id of the comment. A reaction goes to this id. A
    /// cached detail without the id does not parse, so the detail worker
    /// reads it from GitHub again.
    pub id: String,
    pub author: String,
    pub body: String,
    #[serde(default)]
    pub body_html: String,
    pub created_at: DateTime<Utc>,
    pub url: String,
    /// The reactions on the comment, in the order of GitHub.
    #[serde(default)]
    pub reactions: Vec<Reaction>,
}

/// A submitted review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Review {
    /// The GraphQL node id of the review. A reaction goes to this id.
    pub id: String,
    pub author: String,
    pub state: ReviewState,
    pub body: String,
    #[serde(default)]
    pub body_html: String,
    pub submitted_at: Option<DateTime<Utc>>,
    /// The reactions on the body of the review, in the order of GitHub.
    #[serde(default)]
    pub reactions: Vec<Reaction>,
}

/// The state of a review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

/// The content of a reaction. The order is the order of GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(crate) enum ReactionKind {
    ThumbsUp,
    ThumbsDown,
    Laugh,
    Hooray,
    Confused,
    Heart,
    Rocket,
    Eyes,
}

impl ReactionKind {
    /// All the reactions, in the order of GitHub.
    pub(crate) const ALL: [Self; 8] = [
        Self::ThumbsUp,
        Self::ThumbsDown,
        Self::Laugh,
        Self::Hooray,
        Self::Confused,
        Self::Heart,
        Self::Rocket,
        Self::Eyes,
    ];

    /// The emoji of the reaction. Each emoji is two cells wide.
    pub(crate) fn emoji(self) -> &'static str {
        match self {
            Self::ThumbsUp => "👍",
            Self::ThumbsDown => "👎",
            Self::Laugh => "😄",
            Self::Hooray => "🎉",
            Self::Confused => "😕",
            Self::Heart => "❤️",
            Self::Rocket => "🚀",
            Self::Eyes => "👀",
        }
    }

    /// The name of the reaction for the user, for example `thumbs up`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::ThumbsUp => "thumbs up",
            Self::ThumbsDown => "thumbs down",
            Self::Laugh => "laugh",
            Self::Hooray => "hooray",
            Self::Confused => "confused",
            Self::Heart => "heart",
            Self::Rocket => "rocket",
            Self::Eyes => "eyes",
        }
    }
}

/// The reactions of one kind on a pull request, a comment or a review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Reaction {
    pub kind: ReactionKind,
    /// The number of people who reacted. A reaction in a list is never
    /// zero.
    pub count: u32,
    /// True when the viewer is one of them.
    pub mine: bool,
    /// The logins of the first people who reacted, at most two, in the
    /// order of GitHub. Empty when GitHub did not send them: the
    /// comments of a review thread have no logins. A cached detail
    /// without them has none until the next load.
    #[serde(default)]
    pub logins: Vec<String>,
}

/// True when the viewer reacted with `kind` in `reactions`.
pub(crate) fn has_mine(reactions: &[Reaction], kind: ReactionKind) -> bool {
    reactions.iter().any(|r| r.kind == kind && r.mine)
}

/// Sets the reaction `kind` of the viewer in `reactions`: `add` adds it,
/// else it goes away. A reaction that is already in this state does not
/// change, so the call is safe to do again. A count of zero leaves the
/// list, and a new kind goes to its place in the order of GitHub.
pub(crate) fn set_mine(reactions: &mut Vec<Reaction>, kind: ReactionKind, add: bool) {
    if has_mine(reactions, kind) == add {
        return;
    }
    match reactions.iter().position(|r| r.kind == kind) {
        Some(at) => {
            let reaction = &mut reactions[at];
            reaction.mine = add;
            reaction.count = if add {
                reaction.count.saturating_add(1)
            } else {
                reaction.count.saturating_sub(1)
            };
            if reaction.count == 0 {
                reactions.remove(at);
            }
        }
        None if add => {
            let at = reactions.partition_point(|r| r.kind < kind);
            reactions.insert(
                at,
                Reaction {
                    kind,
                    count: 1,
                    mine: true,
                    logins: Vec::new(),
                },
            );
        }
        None => {}
    }
}

/// A review thread on a line of a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Thread {
    /// The GraphQL node id of the thread. A reply goes to this id. A
    /// cached detail without the id does not parse, so the detail worker
    /// reads it from GitHub again.
    pub id: String,
    pub path: String,
    pub line: Option<u32>,
    pub is_resolved: bool,
    pub comments: Vec<Comment>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{assert_round_trip, test_time};

    fn comment() -> Comment {
        Comment {
            id: "IC_1".into(),
            author: "chaosteil".into(),
            body: "Looks good.".into(),
            body_html: String::new(),
            created_at: test_time(),
            url: "https://github.com/o/r/pull/1#issuecomment-1".into(),
            reactions: vec![Reaction {
                kind: ReactionKind::Heart,
                count: 2,
                mine: true,
                logins: vec!["alice".into(), "chaosteil".into()],
            }],
        }
    }

    #[test]
    fn detail_round_trip() {
        assert_round_trip(&PrDetail {
            comments: vec![comment()],
            reviews: vec![Review {
                id: "PRR_1".into(),
                author: "colleague".into(),
                state: ReviewState::ChangesRequested,
                body: "Please fix.".into(),
                body_html: String::new(),
                submitted_at: Some(test_time()),
                reactions: Vec::new(),
            }],
            threads: vec![Thread {
                id: "PRRT_1".into(),
                path: "src/lib.rs".into(),
                line: Some(10),
                is_resolved: false,
                comments: vec![comment()],
            }],
            reactions: vec![Reaction {
                kind: ReactionKind::Rocket,
                count: 1,
                mine: false,
                logins: vec!["bob".into()],
            }],
            head_oid: "abc123".into(),
        });
    }

    #[test]
    fn a_detail_without_reactions_parses() {
        let old = r#"{"comments":[],"reviews":[],"threads":[],"head_oid":"abc123"}"#;
        let detail: PrDetail = serde_json::from_str(old).unwrap();
        assert_eq!(detail.reactions, []);
    }

    #[test]
    fn a_thread_without_an_id_does_not_parse() {
        let old = r#"{"path":"src/lib.rs","line":10,"is_resolved":false,"comments":[]}"#;
        assert!(serde_json::from_str::<Thread>(old).is_err());
    }

    #[test]
    fn a_comment_without_an_id_does_not_parse_and_reactions_are_optional() {
        let old = r#"{"author":"a","body":"","created_at":"2026-01-01T00:00:00Z","url":""}"#;
        assert!(serde_json::from_str::<Comment>(old).is_err());
        let new =
            r#"{"id":"IC_1","author":"a","body":"","created_at":"2026-01-01T00:00:00Z","url":""}"#;
        let comment: Comment = serde_json::from_str(new).unwrap();
        assert_eq!(comment.reactions, []);
    }

    fn reaction(kind: ReactionKind, count: u32, mine: bool) -> Reaction {
        Reaction {
            kind,
            count,
            mine,
            logins: Vec::new(),
        }
    }

    #[test]
    fn a_reaction_without_logins_parses() {
        let old = r#"{"kind":"Heart","count":2,"mine":true}"#;
        let reaction: Reaction = serde_json::from_str(old).unwrap();
        assert_eq!(reaction, self::reaction(ReactionKind::Heart, 2, true));
    }

    #[test]
    fn set_mine_adds_and_removes_the_reaction_of_the_viewer() {
        use ReactionKind::{Eyes, Heart, Hooray, ThumbsUp};
        let mut reactions = vec![reaction(ThumbsUp, 2, false), reaction(Eyes, 1, true)];
        set_mine(&mut reactions, ThumbsUp, true);
        assert_eq!(reactions[0], reaction(ThumbsUp, 3, true));
        set_mine(&mut reactions, ThumbsUp, true);
        assert_eq!(reactions[0].count, 3, "a second add changes nothing");
        set_mine(&mut reactions, Heart, true);
        set_mine(&mut reactions, Hooray, true);
        assert_eq!(
            reactions,
            [
                reaction(ThumbsUp, 3, true),
                reaction(Hooray, 1, true),
                reaction(Heart, 1, true),
                reaction(Eyes, 1, true),
            ],
            "a new kind goes to its place in the order of GitHub"
        );
        set_mine(&mut reactions, Eyes, false);
        assert!(!has_mine(&reactions, Eyes));
        assert_eq!(reactions.len(), 3, "a count of zero leaves the list");
        set_mine(&mut reactions, Eyes, false);
        set_mine(&mut reactions, ThumbsUp, false);
        assert_eq!(reactions[0], reaction(ThumbsUp, 2, false));
    }

    #[test]
    fn the_reactions_have_the_order_and_the_emoji_of_github() {
        let emoji: Vec<&str> = ReactionKind::ALL.iter().map(|k| k.emoji()).collect();
        assert_eq!(emoji, ["👍", "👎", "😄", "🎉", "😕", "❤️", "🚀", "👀"]);
        let mut sorted = ReactionKind::ALL;
        sorted.sort();
        assert_eq!(sorted, ReactionKind::ALL);
    }
}
