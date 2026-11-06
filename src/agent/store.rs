//! The result files of pr-agent.
//!
//! Each pull request has a directory
//! `<state_dir>/agent/<owner>/<repo>/<number>/` with one file per tool:
//! `review.json`, `improve.json`, `describe.json` and `ask.json`. A write
//! goes to a temporary file in the same directory first, and a rename
//! then replaces the old file. So a reader sees the old file or the new
//! file, never a part of a file.

use std::io;
use std::path::{Path, PathBuf};

use crate::domain::agent::{Record, ToolKind, VERSION};
use crate::domain::keys::PrKey;

/// The name of the ask script in the state directory.
const SHIM: &str = "chaoshub-agent-ask";

/// The result files under one state directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Store {
    state_dir: PathBuf,
}

impl Store {
    /// The store in `state_dir`, the state directory of chaoshub.
    pub(crate) fn new(state_dir: PathBuf) -> Self {
        Self { state_dir }
    }

    /// The root of the result files: `<state_dir>/agent`.
    pub(crate) fn root(&self) -> PathBuf {
        self.state_dir.join("agent")
    }

    /// The path of the ask script: `<state_dir>/chaoshub-agent-ask`.
    pub(crate) fn shim_path(&self) -> PathBuf {
        self.state_dir.join(SHIM)
    }

    /// The directory of a pull request.
    pub(crate) fn pr_dir(&self, key: &PrKey) -> PathBuf {
        self.root()
            .join(&key.repo.owner)
            .join(&key.repo.name)
            .join(key.number.to_string())
    }

    /// The result file of one tool.
    pub(crate) fn path(&self, key: &PrKey, kind: ToolKind) -> PathBuf {
        self.pr_dir(key).join(format!("{}.json", kind.name()))
    }

    /// A new temporary path for the JSON output of pr-agent, in the
    /// directory of the pull request. It makes the directory.
    pub(crate) fn output_path(&self, key: &PrKey, kind: ToolKind) -> io::Result<PathBuf> {
        let dir = self.pr_dir(key);
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join(format!(
            ".{}.out-{}-{:08x}.json",
            kind.name(),
            std::process::id(),
            rand::random::<u32>()
        )))
    }

    /// The result of one tool. `None` when the file does not exist. A
    /// file of another version is an error.
    pub(crate) fn read(&self, key: &PrKey, kind: ToolKind) -> io::Result<Option<Record>> {
        let text = match std::fs::read_to_string(self.path(key, kind)) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        let record: Record = serde_json::from_str(&text).map_err(io::Error::other)?;
        if record.version != VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("version {} is not {VERSION}", record.version),
            ));
        }
        Ok(Some(record))
    }

    /// Writes the result of one tool: first a temporary file, then a
    /// rename over the old file.
    pub(crate) fn write(&self, key: &PrKey, kind: ToolKind, record: &Record) -> io::Result<()> {
        let path = self.path(key, kind);
        let mut text = serde_json::to_string_pretty(record).map_err(io::Error::other)?;
        text.push('\n');
        write_atomic(&path, text.as_bytes())
    }

    /// Writes the ask script, unless the file has the same text already.
    pub(crate) fn write_shim(&self, text: &str) -> io::Result<PathBuf> {
        let path = self.shim_path();
        if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
            return Ok(path);
        }
        write_atomic(&path, text.as_bytes())?;
        Ok(path)
    }
}

/// Writes `bytes` to a temporary file next to `path` and renames it to
/// `path`. It makes the directory. On an error, it removes the temporary
/// file.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("the path has no directory"))?;
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("the path has no file name"))?
        .to_string_lossy();
    let tmp = dir.join(format!(
        ".{name}.tmp-{}-{:08x}",
        std::process::id(),
        rand::random::<u32>()
    ));
    let result = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use chrono::DateTime;

    use super::*;
    use crate::domain::keys::RepoKey;

    fn key() -> PrKey {
        PrKey {
            repo: RepoKey {
                owner: "chaosteil".into(),
                name: "chaoshub-test".into(),
            },
            number: 7,
        }
    }

    fn record(head: &str) -> Record {
        Record {
            version: VERSION,
            tool: "review".into(),
            pr: 7,
            head_sha: head.into(),
            model: "anthropic/claude-sonnet-5-5".into(),
            started_at: DateTime::UNIX_EPOCH,
            finished_at: DateTime::UNIX_EPOCH,
            usage: serde_json::json!({"total_tokens": 12000}),
            data: serde_json::json!({"review": {}}),
        }
    }

    /// The names of the files in `dir`.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn paths_sit_under_the_state_dir() {
        let store = Store::new(PathBuf::from("/state/chaoshub"));
        assert_eq!(store.root(), Path::new("/state/chaoshub/agent"));
        assert_eq!(
            store.path(&key(), ToolKind::Improve),
            Path::new("/state/chaoshub/agent/chaosteil/chaoshub-test/7/improve.json")
        );
        assert_eq!(
            store.shim_path(),
            Path::new("/state/chaoshub/chaoshub-agent-ask")
        );
    }

    #[test]
    fn write_then_read_round_trips_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        assert_eq!(store.read(&key(), ToolKind::Review).unwrap(), None);
        store
            .write(&key(), ToolKind::Review, &record("3a1f2c0"))
            .unwrap();
        store
            .write(&key(), ToolKind::Review, &record("9bd2e11"))
            .unwrap();
        let read = store.read(&key(), ToolKind::Review).unwrap().unwrap();
        assert_eq!(read, record("9bd2e11"));
        assert_eq!(read.total_tokens(), Some(12000));
        assert_eq!(names(&store.pr_dir(&key())), ["review.json"]);
    }

    #[test]
    fn a_failed_write_keeps_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store
            .write(&key(), ToolKind::Review, &record("3a1f2c0"))
            .unwrap();
        // A directory in place of the target makes the rename fail.
        let path = store.path(&key(), ToolKind::Improve);
        std::fs::create_dir_all(path.join("x")).unwrap();
        assert!(
            store
                .write(&key(), ToolKind::Improve, &record("9bd2e11"))
                .is_err()
        );
        assert_eq!(
            names(&store.pr_dir(&key())),
            ["improve.json", "review.json"]
        );
        let read = store.read(&key(), ToolKind::Review).unwrap().unwrap();
        assert_eq!(read.head_sha, "3a1f2c0");
    }

    #[test]
    fn read_refuses_another_version() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let mut other = record("3a1f2c0");
        other.version = 2;
        store.write(&key(), ToolKind::Review, &other).unwrap();
        let err = store.read(&key(), ToolKind::Review).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn write_shim_writes_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let path = store.write_shim("print(1)\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "print(1)\n");
        store.write_shim("print(2)\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "print(2)\n");
    }
}
