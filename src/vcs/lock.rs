//! One async lock per repo path, so git processes in one repo run one at
//! a time.

/// The locks of the repo paths. A placeholder: phase B replaces it with the
/// real locks.
#[derive(Debug, Clone)]
pub(crate) struct RepoLocks;
