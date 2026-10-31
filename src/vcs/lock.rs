//! One async lock per repo path, so git processes in one repo run one at
//! a time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// The locks of the repo paths. Clones share the locks. A git or jj
/// process that changes a repo runs while it holds the lock of that repo.
#[derive(Debug, Clone, Default)]
pub(crate) struct RepoLocks {
    locks: Arc<Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>>,
}

impl RepoLocks {
    /// Waits for the lock of `repo_path`. The lock stays held until the
    /// guard drops.
    pub(crate) async fn lock(&self, repo_path: &Path) -> OwnedMutexGuard<()> {
        let mutex = {
            let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
            Arc::clone(locks.entry(repo_path.to_path_buf()).or_default())
        };
        mutex.lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;

    const SHORT: Duration = Duration::from_millis(50);

    #[tokio::test]
    async fn one_repo_waits_for_the_held_lock() {
        let locks = RepoLocks::default();
        let repo = Path::new("/repo");
        let guard = locks.lock(repo).await;
        assert!(timeout(SHORT, locks.lock(repo)).await.is_err());
        assert!(timeout(SHORT, locks.clone().lock(repo)).await.is_err());
        drop(guard);
        assert!(timeout(SHORT, locks.lock(repo)).await.is_ok());
    }

    #[tokio::test]
    async fn other_repos_do_not_wait() {
        let locks = RepoLocks::default();
        let _guard = locks.lock(Path::new("/repo")).await;
        assert!(
            timeout(SHORT, locks.lock(Path::new("/other")))
                .await
                .is_ok()
        );
    }
}
