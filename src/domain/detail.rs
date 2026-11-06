//! The conversation of a pull request: comments, reviews and threads.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The conversation of a pull request, read at one head commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PrDetail {
    pub comments: Vec<Comment>,
    pub reviews: Vec<Review>,
    pub threads: Vec<Thread>,
    /// The head commit when the detail was read. A different head of the
    /// pull request makes the detail stale.
    pub head_oid: String,
}

/// An issue comment, or a comment in a review thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Comment {
    pub author: String,
    pub body: String,
    #[serde(default)]
    pub body_html: String,
    pub created_at: DateTime<Utc>,
    pub url: String,
}

/// A submitted review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Review {
    pub author: String,
    pub state: ReviewState,
    pub body: String,
    #[serde(default)]
    pub body_html: String,
    pub submitted_at: Option<DateTime<Utc>>,
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

/// A review thread on a line of a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Thread {
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
            author: "chaosteil".into(),
            body: "Looks good.".into(),
            body_html: String::new(),
            created_at: test_time(),
            url: "https://github.com/o/r/pull/1#issuecomment-1".into(),
        }
    }

    #[test]
    fn detail_round_trip() {
        assert_round_trip(&PrDetail {
            comments: vec![comment()],
            reviews: vec![Review {
                author: "colleague".into(),
                state: ReviewState::ChangesRequested,
                body: "Please fix.".into(),
                body_html: String::new(),
                submitted_at: Some(test_time()),
            }],
            threads: vec![Thread {
                path: "src/lib.rs".into(),
                line: Some(10),
                is_resolved: false,
                comments: vec![comment()],
            }],
            head_oid: "abc123".into(),
        });
    }
}
