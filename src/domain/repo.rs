//! A repo, its local path and its merge settings.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::keys::RepoKey;

/// A repo that chaoshub tracks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Repo {
    pub key: RepoKey,
    /// The local checkout, when the configuration names one.
    pub path: Option<PathBuf>,
    /// The merge settings, when a sync has read them.
    pub settings: Option<RepoSettings>,
}

/// The merge methods that a repo permits, and its default branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RepoSettings {
    pub squash: bool,
    pub merge: bool,
    pub rebase: bool,
    /// `None` when the source of the settings does not include it.
    pub default_branch: Option<String>,
    /// The default title of a squash commit. `None` when the source of
    /// the settings does not include it.
    #[serde(default)]
    pub squash_title_default: Option<SquashTitleDefault>,
    /// The default body of a squash commit. `None` when the source of the
    /// settings does not include it.
    #[serde(default)]
    pub squash_message_default: Option<SquashMessageDefault>,
}

/// The default title that GitHub gives a squash commit
/// (`squashMergeCommitTitle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SquashTitleDefault {
    /// The title of the pull request.
    PrTitle,
    /// The title of the commit when the pull request has one commit, else
    /// the title of the pull request.
    CommitOrPrTitle,
}

/// The default body that GitHub gives a squash commit
/// (`squashMergeCommitMessage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SquashMessageDefault {
    /// The body of the pull request.
    PrBody,
    /// The messages of the commits of the branch.
    CommitMessages,
    /// An empty body.
    Blank,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::assert_round_trip;

    #[test]
    fn repo_round_trip() {
        assert_round_trip(&Repo {
            key: RepoKey {
                owner: "chaosteil".into(),
                name: "chaoshub".into(),
            },
            path: Some(PathBuf::from("/home/me/code/chaoshub")),
            settings: Some(RepoSettings {
                squash: true,
                merge: false,
                rebase: true,
                default_branch: Some("main".into()),
                squash_title_default: Some(SquashTitleDefault::PrTitle),
                squash_message_default: Some(SquashMessageDefault::Blank),
            }),
        });
    }
}
