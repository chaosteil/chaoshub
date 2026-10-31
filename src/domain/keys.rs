//! Keys that identify repos and pull requests.

use serde::{Deserialize, Serialize};

/// Identifies a GitHub repo, for example `chaosteil/chaoshub`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct RepoKey {
    pub owner: String,
    pub name: String,
}

/// Identifies a pull request in a repo.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct PrKey {
    pub repo: RepoKey,
    pub number: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::assert_round_trip;

    #[test]
    fn keys_round_trip() {
        let repo = RepoKey {
            owner: "chaosteil".into(),
            name: "chaoshub".into(),
        };
        assert_round_trip(&repo);
        assert_round_trip(&PrKey { repo, number: 42 });
    }
}
