//! GitHub native stacks of pull requests.

use serde::{Deserialize, Serialize};

use crate::domain::keys::PrKey;
use crate::domain::pr::PrState;

/// The stack fields of one pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StackRef {
    /// The GraphQL node id of the stack.
    pub id: String,
    pub number: u64,
    pub size: u32,
    /// The position of the pull request. Position 1 is closest to the base.
    pub position: u32,
    pub base_ref: String,
    /// The positions of the merged entries, position 1 first, as GitHub
    /// gave them with this pull request. Empty when not known, for
    /// example in a cache row of an older version.
    #[serde(default)]
    pub merged: Vec<u32>,
    /// The open entries, position 1 first, as GitHub gave them with this
    /// pull request. The sync worker uses them to find an entry with an
    /// old place in its cache. Empty when not known, for example in a
    /// cache row of an older version.
    #[serde(default)]
    pub open: Vec<OpenEntry>,
}

/// One open entry of a stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OpenEntry {
    pub position: u32,
    pub number: u64,
    /// The GraphQL node id of the pull request.
    pub node_id: String,
}

impl StackRef {
    /// The place of the pull request in the stack.
    pub(crate) fn place(&self) -> StackPlace {
        StackPlace {
            id: self.id.clone(),
            position: self.position,
        }
    }
}

/// The place of a pull request in a stack: the stack id and the
/// position. A search result gives it, so the sync worker can find a
/// pull request that joined or left a stack. GitHub does not change
/// `updatedAt` when it adds a pull request to a stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StackPlace {
    /// The GraphQL node id of the stack.
    pub id: String,
    pub position: u32,
}

/// A stack with its loaded entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stack {
    pub id: String,
    pub base_ref: String,
    /// Position 1 first.
    pub entries: Vec<StackEntry>,
}

/// One pull request of a stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StackEntry {
    pub position: u32,
    pub number: u64,
    pub title: String,
    pub state: PrState,
    pub is_draft: bool,
}

/// The pull requests of one stack, or one pull request without a stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StackGroup {
    /// `None` for a pull request without a stack.
    pub stack: Option<StackRef>,
    /// The top of the stack first.
    pub prs: Vec<PrKey>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::assert_round_trip;
    use crate::domain::keys::RepoKey;

    fn stack_ref() -> StackRef {
        StackRef {
            id: "PRS_1".into(),
            number: 7,
            size: 2,
            position: 2,
            base_ref: "main".into(),
            merged: Vec::new(),
            open: vec![OpenEntry {
                position: 2,
                number: 42,
                node_id: "PR_42".into(),
            }],
        }
    }

    #[test]
    fn stack_round_trip() {
        assert_round_trip(&Stack {
            id: "PRS_1".into(),
            base_ref: "main".into(),
            entries: vec![StackEntry {
                position: 1,
                number: 41,
                title: "Add the base".into(),
                state: PrState::Open,
                is_draft: false,
            }],
        });
    }

    #[test]
    fn stack_group_round_trip() {
        assert_round_trip(&StackGroup {
            stack: Some(stack_ref()),
            prs: vec![PrKey {
                repo: RepoKey {
                    owner: "o".into(),
                    name: "r".into(),
                },
                number: 42,
            }],
        });
    }
}
