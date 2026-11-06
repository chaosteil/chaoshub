//! The changed files of a pull request.

use serde::{Deserialize, Serialize};

/// The changed files of a pull request, read at one head commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PrFiles {
    pub files: Vec<ChangedFile>,
    /// True when the pull request has more files than the read pages.
    pub truncated: bool,
    /// The head commit when the files were read. A different head of the
    /// pull request makes the files stale.
    pub head_oid: String,
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChangedFile {
    pub path: String,
    /// The old path of a renamed or copied file.
    pub previous_path: Option<String>,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    /// The diff text. `None` for a binary file or a very large diff.
    pub patch: Option<String>,
}

/// The change of a file, as GitHub reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FileStatus {
    Added,
    Removed,
    Modified,
    Renamed,
    Copied,
    Changed,
    Unchanged,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::assert_round_trip;

    #[test]
    fn files_round_trip() {
        assert_round_trip(&PrFiles {
            files: vec![
                ChangedFile {
                    path: "src/lib.rs".into(),
                    previous_path: None,
                    status: FileStatus::Modified,
                    additions: 3,
                    deletions: 1,
                    patch: Some("@@ -1,2 +1,4 @@\n+use std::fmt;".into()),
                },
                ChangedFile {
                    path: "src/new.rs".into(),
                    previous_path: Some("src/old.rs".into()),
                    status: FileStatus::Renamed,
                    additions: 0,
                    deletions: 0,
                    patch: None,
                },
            ],
            truncated: false,
            head_oid: "abc123".into(),
        });
    }
}
