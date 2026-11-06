//! The review comments of codeview, as plain data.
//!
//! codeview keeps the comments of one review in one markdown file. For a
//! pull request, each push of the head gets a new file. `codeview::store`
//! reads the files. These types carry the comments that went to GitHub
//! not yet, so `bus` can carry them between the runtime and `app`.

/// The format version of the comment file. A reader refuses other
/// versions.
pub(crate) const VERSION: u32 = 1;

/// The kind of a comment, from its `file` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommentKind {
    /// A comment on lines of a file. `file` is a path in the repo.
    Line,
    /// A comment on the pull request document (`codeview://pr/...`). It
    /// goes into the text of the review.
    Pr,
    /// A comment on a commit message (another `codeview://` path).
    /// GitHub takes no such comment.
    Message,
}

impl CommentKind {
    /// The kind of the `file` field of a comment.
    pub(crate) fn of(file: &str) -> Self {
        if file.starts_with("codeview://pr") {
            Self::Pr
        } else if file.starts_with("codeview://") {
            Self::Message
        } else {
            Self::Line
        }
    }
}

/// One comment that went to GitHub not yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Comment {
    /// The id of the comment, unique in its file.
    pub id: String,
    /// The path of the file, or a `codeview://` path.
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
    /// `old` or `new`.
    pub side: String,
    /// The markdown text of the comment.
    pub body: String,
    pub kind: CommentKind,
}

impl Comment {
    /// True when the comment can go to GitHub. A commit message comment
    /// cannot.
    pub(crate) fn sendable(&self) -> bool {
        self.kind != CommentKind::Message
    }

    /// The place of the comment: `path:L12-14 (new)`, `PR` for a pull
    /// request comment, `commit message` for a commit message comment.
    pub(crate) fn place(&self) -> String {
        match self.kind {
            CommentKind::Pr => "PR".to_owned(),
            CommentKind::Message => "commit message".to_owned(),
            CommentKind::Line if self.start_line == self.end_line => {
                format!("{}:L{} ({})", self.file, self.start_line, self.side)
            }
            CommentKind::Line => format!(
                "{}:L{}-{} ({})",
                self.file, self.start_line, self.end_line, self.side
            ),
        }
    }

    /// The first line of the body.
    pub(crate) fn summary(&self) -> &str {
        self.body.lines().next().unwrap_or_default()
    }
}

/// The unsent comments of one comment file: one push of the head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Push {
    /// The head commit of the push.
    pub head: String,
    /// The unsent comments, in the order of the file.
    pub comments: Vec<Comment>,
}

/// The unsent comments of one pull request, per push.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Comments {
    pub pushes: Vec<Push>,
}

impl Comments {
    /// The unsent comments of the push of `head`.
    pub(crate) fn current(&self, head: &str) -> &[Comment] {
        self.pushes
            .iter()
            .find(|push| push.head == head)
            .map_or(&[], |push| push.comments.as_slice())
    }

    /// The number of unsent comments of the other pushes.
    pub(crate) fn older(&self, head: &str) -> usize {
        self.pushes
            .iter()
            .filter(|push| push.head != head)
            .map(|push| push.comments.len())
            .sum()
    }

    /// The push that holds an unsent comment with the id `id`.
    pub(crate) fn push_of(&self, id: &str) -> Option<&Push> {
        self.pushes
            .iter()
            .find(|push| push.comments.iter().any(|comment| comment.id == id))
    }
}
