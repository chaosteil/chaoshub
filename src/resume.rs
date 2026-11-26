//! OSC 7577 (program resume): the token, the state files and the
//! reports. The terminal starts chaoshub again with the command of the
//! last report, and the state file gives back the UI state.
//!
//! Each instance has a 16-hex id and a state file in
//! `<state dir>/resume/`: `list-<id>.json` for a list and `pr-<id>.json`
//! for a PR view. The file is JSON, for example
//! `{"v":1,"kind":"list",...}`. The directory has mode 0700 and the file
//! mode 0600. A save writes a `.tmp` file and renames it.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read as _, Write};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::domain::keys::PrKey;
use crate::domain::resume::{
    ListResume, PaneResume, PrViewResume, ResumeState, STATE_VERSION, UiResume,
};
use crate::osc::{self, ResumeBody};

/// The age after the last save or load at which `prune` deletes a state
/// file: 30 days.
pub(crate) const MAX_AGE: Duration = Duration::from_hours(30 * 24);

/// The shortest and the longest valid token.
const TOKEN_LEN: std::ops::RangeInclusive<usize> = 22..=64;
/// The suffix of a state file and of its temporary file.
const SUFFIX: &str = ".json";
const TMP_SUFFIX: &str = ".tmp";
/// The number of hex digits of an instance id.
const ID_LEN: usize = 16;

/// The token of `OSC7577_TOKEN`. It signs the reports.
pub(crate) struct Token(String);

impl Token {
    /// The token of the env value, when it has 22 to 64 characters of
    /// `[A-Za-z0-9_-]`. Else `None`.
    pub(crate) fn parse(value: Option<OsString>) -> Option<Self> {
        let value = value?.into_string().ok()?;
        let valid = TOKEN_LEN.contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        valid.then_some(Self(value))
    }
}

/// The directory of the state files in the state directory.
pub(crate) fn dir(state_dir: &Path) -> PathBuf {
    state_dir.join("resume")
}

/// The kind of an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    List,
    PrView,
}

impl Kind {
    /// The prefix of the state file name.
    fn prefix(self) -> &'static str {
        match self {
            Self::List => "list-",
            Self::PrView => "pr-",
        }
    }
}

/// The state file of one instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StateFile {
    path: PathBuf,
    id: String,
}

/// A state file on disk: the version and the state.
#[derive(Serialize)]
struct StoredRef<'a> {
    v: u32,
    #[serde(flatten)]
    state: &'a ResumeState,
}

#[derive(Deserialize)]
struct Stored {
    v: u32,
    #[serde(flatten)]
    state: ResumeState,
}

impl StateFile {
    /// A new state file in `dir`, with a new random id.
    pub(crate) fn new(dir: &Path, kind: Kind) -> Self {
        let id = format!("{:016x}", rand::random::<u64>());
        let path = dir.join(format!("{}{id}{SUFFIX}", kind.prefix()));
        Self { path, id }
    }

    /// The state file of `arg`, when `arg` is in `dir` and its name is
    /// `list-<id>.json` or `pr-<id>.json` for `kind`. Else `None`, so
    /// chaoshub never writes over another file.
    pub(crate) fn from_arg(dir: &Path, kind: Kind, arg: &Path) -> Option<Self> {
        if arg.parent() != Some(dir) {
            return None;
        }
        let name = arg.file_name()?.to_str()?;
        let id = name.strip_prefix(kind.prefix())?.strip_suffix(SUFFIX)?;
        let valid = id.len() == ID_LEN
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        valid.then(|| Self {
            path: arg.to_path_buf(),
            id: id.to_owned(),
        })
    }

    /// The instance id.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// The path of the file.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the state. `None` when the file is missing, is not valid
    /// JSON or has another version. After a read, the function sets the
    /// mtime of the file to now, so `prune` keeps it.
    pub(crate) fn load(&self) -> Option<ResumeState> {
        let mut file = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
            Err(err) => {
                tracing::warn!(%err, path = %self.path.display(), "cannot open the resume state");
                return None;
            }
        };
        let mut text = String::new();
        if let Err(err) = file.read_to_string(&mut text) {
            tracing::warn!(%err, path = %self.path.display(), "cannot read the resume state");
            return None;
        }
        if let Err(err) = file.set_modified(SystemTime::now()) {
            tracing::debug!(%err, "cannot refresh the mtime of the resume state");
        }
        match serde_json::from_str::<Stored>(&text) {
            Ok(stored) if stored.v == STATE_VERSION => Some(stored.state),
            Ok(stored) => {
                tracing::warn!(v = stored.v, "the resume state has another version");
                None
            }
            Err(err) => {
                tracing::warn!(%err, path = %self.path.display(), "the resume state does not decode");
                None
            }
        }
    }

    /// Writes the state. The function makes the directory with mode
    /// 0700, writes `<path>.tmp` with mode 0600 and renames it.
    pub(crate) fn save(&self, state: &ResumeState) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
        let json = serde_json::to_vec(&StoredRef {
            v: STATE_VERSION,
            state,
        })
        .map_err(io::Error::other)?;
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(TMP_SUFFIX);
        let tmp = PathBuf::from(tmp);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&json)?;
        file.sync_all()?;
        fs::rename(&tmp, &self.path)
    }

    /// Deletes the file. A missing file is fine.
    pub(crate) fn remove(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }
}

/// Deletes the state files and the `.tmp` files in `dir` whose mtime is
/// older than `max_age` at `now`. `keep` stays. Returns the number of
/// deleted files.
pub(crate) fn prune(dir: &Path, now: SystemTime, max_age: Duration, keep: Option<&Path>) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut deleted = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let state = (name.starts_with(Kind::List.prefix())
            || name.starts_with(Kind::PrView.prefix()))
            && name.ends_with(SUFFIX);
        if !(state || name.ends_with(TMP_SUFFIX)) || keep == Some(path.as_path()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let old = meta.is_file()
            && meta
                .modified()
                .ok()
                .and_then(|mtime| now.duration_since(mtime).ok())
                .is_some_and(|age| age > max_age);
        if !old {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => deleted += 1,
            Err(err) => {
                tracing::debug!(%err, path = %path.display(), "cannot prune a resume state");
            }
        }
    }
    deleted
}

/// The PR view state files in `dir` whose list is `list_id`, the newest
/// first.
pub(crate) fn pr_views_of(dir: &Path, list_id: &str) -> Vec<(PathBuf, PrViewResume)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut views = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file) = StateFile::from_arg(dir, Kind::PrView, &path) else {
            continue;
        };
        let mtime = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(UNIX_EPOCH);
        if let Some(ResumeState::PrView(view)) = file.load()
            && view.list.as_deref() == Some(list_id)
        {
            views.push((mtime, path, view));
        }
    }
    views.sort_by_key(|(mtime, _, _)| std::cmp::Reverse(*mtime));
    views
        .into_iter()
        .map(|(_, path, view)| (path, view))
        .collect()
}

/// Writes the OSC 7577 reports of one process.
pub(crate) struct Reporter {
    token: Token,
    argv: Vec<String>,
    cwd: Option<String>,
    pid: u32,
    /// True when the terminal has a report of this process.
    shown: bool,
}

impl Reporter {
    /// A reporter for this process.
    pub(crate) fn new(token: Token, argv: Vec<String>, cwd: Option<String>) -> Self {
        Self {
            token,
            argv,
            cwd,
            pid: std::process::id(),
            shown: false,
        }
    }

    /// Writes a report with the time `now`. When the terminal would
    /// discard the report, the function writes `clear` if an earlier
    /// report is shown.
    pub(crate) fn report(&mut self, out: &mut impl Write, now: SystemTime) -> io::Result<()> {
        let body = ResumeBody::Argv {
            argv: &self.argv,
            cwd: self.cwd.as_deref(),
        };
        if let Some(report) = osc::program_resume(self.pid, secs(now), &body, &self.token.0) {
            out.write_all(&report)?;
            out.flush()?;
            self.shown = true;
            return Ok(());
        }
        tracing::warn!("the resume command is too long or not valid");
        if self.shown {
            self.clear(out, now)?;
        }
        Ok(())
    }

    /// Writes `clear`, which removes the record of the terminal.
    pub(crate) fn clear(&mut self, out: &mut impl Write, now: SystemTime) -> io::Result<()> {
        if let Some(clear) =
            osc::program_resume(self.pid, secs(now), &ResumeBody::Clear, &self.token.0)
        {
            out.write_all(&clear)?;
            out.flush()?;
        }
        self.shown = false;
        Ok(())
    }
}

/// The seconds of `now` since the epoch.
fn secs(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The part of the state that does not change while the process runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fixed {
    List,
    PrView {
        pr: PrKey,
        list: Option<String>,
        list_socket: Option<PathBuf>,
        rex_label: bool,
        block: Option<String>,
    },
}

/// The resume of one active instance: its state file and its reports.
pub(crate) struct Session {
    file: StateFile,
    reporter: Reporter,
    fixed: Fixed,
    /// The last saved state.
    last: Option<ResumeState>,
}

impl Session {
    pub(crate) fn new(file: StateFile, reporter: Reporter, fixed: Fixed) -> Self {
        Self {
            file,
            reporter,
            fixed,
            last: None,
        }
    }

    /// The state file.
    pub(crate) fn file(&self) -> &StateFile {
        &self.file
    }

    /// Builds the state from `ui` and `panes`, and saves it when it
    /// changed. Returns true after a save.
    pub(crate) fn sync(&mut self, ui: UiResume, panes: Vec<PaneResume>) -> io::Result<bool> {
        let state = match (&self.fixed, ui) {
            (Fixed::List, UiResume::List { tab, tabs }) => {
                ResumeState::List(ListResume { tab, tabs, panes })
            }
            (
                Fixed::PrView {
                    pr,
                    list,
                    list_socket,
                    rex_label,
                    block,
                },
                UiResume::PrView { pane },
            ) => ResumeState::PrView(PrViewResume {
                pr: pr.clone(),
                pane,
                list: list.clone(),
                list_socket: list_socket.clone(),
                rex_label: *rex_label,
                block: block.clone(),
            }),
            _ => return Ok(false),
        };
        if self.last.as_ref() == Some(&state) {
            return Ok(false);
        }
        self.file.save(&state)?;
        self.last = Some(state);
        Ok(true)
    }

    /// Writes a report with the current time.
    pub(crate) fn report(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.reporter.report(out, SystemTime::now())
    }

    /// Ends the session. When the user ended it (`by_user`), the function
    /// writes `clear` and deletes the state file. A list also deletes the
    /// files of its PR views in `dir`, because it closes their panes.
    /// Else the record and the file stay, so the terminal can resume.
    pub(crate) fn finish(&mut self, out: &mut impl Write, by_user: bool, dir: &Path) {
        if !by_user {
            return;
        }
        if let Err(err) = self.reporter.clear(out, SystemTime::now()) {
            tracing::warn!(%err, "cannot clear the resume record");
        }
        if let Err(err) = self.file.remove() {
            tracing::warn!(%err, "cannot delete the resume state");
        }
        if self.fixed == Fixed::List {
            for (path, _) in pr_views_of(dir, self.file.id()) {
                if let Err(err) = fs::remove_file(&path) {
                    tracing::debug!(%err, path = %path.display(), "cannot delete a PR view state");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::domain::keys::RepoKey;
    use crate::domain::resume::TabResume;

    const TOKEN: &str = "dummy-token-replace-me";
    const DAY: Duration = Duration::from_hours(24);

    fn token() -> Token {
        Token::parse(Some(TOKEN.into())).unwrap()
    }

    fn pr(number: u64) -> PrKey {
        PrKey {
            repo: RepoKey {
                owner: "chaosteil".into(),
                name: "chaoshub".into(),
            },
            number,
        }
    }

    fn list_state() -> ResumeState {
        ResumeState::List(ListResume {
            tab: "review".into(),
            tabs: vec![TabResume {
                id: "review".into(),
                selected: Some(pr(7)),
                filter: "fix".into(),
            }],
            panes: vec![PaneResume {
                pr: pr(7),
                block_id: "block:02".into(),
                label: "#7".into(),
            }],
        })
    }

    fn view_state(list: &str) -> ResumeState {
        ResumeState::PrView(PrViewResume {
            pr: pr(7),
            pane: "checks".into(),
            list: Some(list.into()),
            list_socket: None,
            rex_label: false,
            block: Some("block:02".into()),
        })
    }

    fn list_ui(tab: &str) -> UiResume {
        UiResume::List {
            tab: tab.into(),
            tabs: Vec::new(),
        }
    }

    fn reporter(argv: &[&str]) -> Reporter {
        Reporter::new(
            token(),
            argv.iter().map(|&arg| arg.to_owned()).collect(),
            Some("/home/user".into()),
        )
    }

    fn set_mtime(path: &Path, mtime: SystemTime) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
    }

    fn mtime(path: &Path) -> SystemTime {
        fs::metadata(path).unwrap().modified().unwrap()
    }

    #[test]
    fn token_parse_cases() {
        let parse = |value: &str| Token::parse(Some(value.into())).is_some();
        assert!(!parse(&"a".repeat(21)));
        assert!(parse(&"a".repeat(22)));
        assert!(parse(&"a".repeat(64)));
        assert!(!parse(&"a".repeat(65)));
        assert!(!parse(&format!("{}+", "a".repeat(22))));
        assert!(!parse(""));
        assert!(parse(TOKEN));
        assert!(Token::parse(None).is_none());
    }

    #[test]
    fn state_file_round_trip_and_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = dir(tmp.path());
        let file = StateFile::new(&dir, Kind::List);
        file.save(&list_state()).unwrap();
        assert_eq!(file.load(), Some(list_state()));
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(file.path()), 0o600);
        assert_eq!(mode(&dir), 0o700);
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.contains(r#""v":1"#), "{text}");
        assert!(text.contains(r#""kind":"list""#), "{text}");
        assert!(
            file.path()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("list-")
        );
        assert_eq!(file.id().len(), 16);
    }

    #[test]
    fn a_missing_or_bad_state_file_loads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        let file = StateFile::new(tmp.path(), Kind::List);
        assert_eq!(file.load(), None);
        fs::write(file.path(), "{not json").unwrap();
        assert_eq!(file.load(), None);
        fs::write(
            file.path(),
            r#"{"v":2,"kind":"list","tab":"mine","tabs":[],"panes":[]}"#,
        )
        .unwrap();
        assert_eq!(file.load(), None);
    }

    #[test]
    fn load_is_idempotent_and_refreshes_the_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let file = StateFile::new(tmp.path(), Kind::PrView);
        file.save(&view_state("0123456789abcdef")).unwrap();
        let old = SystemTime::now() - 10 * DAY;
        set_mtime(file.path(), old);
        assert_eq!(file.load(), Some(view_state("0123456789abcdef")));
        assert!(mtime(file.path()) > old + 9 * DAY);
        assert_eq!(file.load(), Some(view_state("0123456789abcdef")));
    }

    #[test]
    fn from_arg_accepts_only_state_files_of_the_dir() {
        let dir = Path::new("/s/resume");
        let file = StateFile::from_arg(
            dir,
            Kind::List,
            Path::new("/s/resume/list-0123456789abcdef.json"),
        )
        .unwrap();
        assert_eq!(file.id(), "0123456789abcdef");
        assert_eq!(
            file.path(),
            Path::new("/s/resume/list-0123456789abcdef.json")
        );
        assert!(
            StateFile::from_arg(
                dir,
                Kind::PrView,
                Path::new("/s/resume/pr-0123456789abcdef.json")
            )
            .is_some()
        );
        for (kind, bad) in [
            (Kind::List, "/s/other/list-0123456789abcdef.json"),
            (Kind::List, "/s/resume/sub/list-0123456789abcdef.json"),
            (Kind::List, "/etc/passwd"),
            (Kind::List, "/s/resume/list-0123456789ABCDEF.json"),
            (Kind::List, "/s/resume/list-0123456789abcde.json"),
            (Kind::List, "/s/resume/list-0123456789abcdef.toml"),
            (Kind::List, "/s/resume/list-../../etc/passwd.json"),
            (Kind::PrView, "/s/resume/list-0123456789abcdef.json"),
            (Kind::List, "/s/resume/pr-0123456789abcdef.json"),
        ] {
            assert_eq!(
                StateFile::from_arg(dir, kind, Path::new(bad)),
                None,
                "{bad}"
            );
        }
    }

    #[test]
    fn prune_deletes_files_older_than_the_max_age() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let now = SystemTime::now();
        let write = |name: &str, age: Duration| {
            let path = dir.join(name);
            fs::write(&path, "{}").unwrap();
            set_mtime(&path, now - age);
            path
        };
        let old_list = write("list-0000000000000001.json", 31 * DAY);
        let old_view = write("pr-0000000000000002.json", 31 * DAY);
        let old_tmp = write("pr-0000000000000003.json.tmp", 31 * DAY);
        let young = write("list-0000000000000004.json", 29 * DAY);
        let keep = write("list-0000000000000005.json", 31 * DAY);
        let other = write("notes.txt", 31 * DAY);
        assert_eq!(prune(dir, now, MAX_AGE, Some(&keep)), 3);
        assert!(!old_list.exists());
        assert!(!old_view.exists());
        assert!(!old_tmp.exists());
        assert!(young.exists());
        assert!(keep.exists());
        assert!(other.exists());
    }

    #[test]
    fn reporter_writes_a_fresh_report_each_time() {
        let mut reporter = reporter(&["/usr/bin/chaoshub"]);
        let mut first = Vec::new();
        reporter
            .report(&mut first, UNIX_EPOCH + Duration::from_hours(498_762))
            .unwrap();
        let mut second = Vec::new();
        reporter
            .report(&mut second, UNIX_EPOCH + Duration::from_mins(29_925_721))
            .unwrap();
        let first = String::from_utf8(first).unwrap();
        let second = String::from_utf8(second).unwrap();
        assert!(first.contains(":time=1795543200:argv="), "{first}");
        assert!(second.contains(":time=1795543260:argv="), "{second}");
        assert!(first.contains(":cwd=L2hvbWUvdXNlcg==:mac="), "{first}");
    }

    #[test]
    fn a_too_long_command_sends_clear_after_a_report() {
        let now = SystemTime::now();
        let long = "a".repeat(osc::RESUME_MAX_BYTES);
        let mut out = Vec::new();
        let mut fresh = reporter(&["/usr/bin/chaoshub", &long]);
        fresh.report(&mut out, now).unwrap();
        assert_eq!(out, [] as [u8; 0]);

        let mut shown = reporter(&["/usr/bin/chaoshub"]);
        shown.report(&mut out, now).unwrap();
        assert!(String::from_utf8_lossy(&out).contains(":argv="));
        out.clear();
        shown.argv.push(long);
        shown.report(&mut out, now).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains(":clear=1:mac="), "{text}");
        assert!(!text.contains(":argv="), "{text}");
    }

    #[test]
    fn session_saves_only_a_changed_state() {
        let tmp = tempfile::tempdir().unwrap();
        let file = StateFile::new(tmp.path(), Kind::List);
        let path = file.path().to_path_buf();
        let mut session = Session::new(file, reporter(&["/usr/bin/chaoshub"]), Fixed::List);
        assert!(session.sync(list_ui("mine"), Vec::new()).unwrap());
        fs::remove_file(&path).unwrap();
        assert!(!session.sync(list_ui("mine"), Vec::new()).unwrap());
        assert!(!path.exists());
        assert!(session.sync(list_ui("review"), Vec::new()).unwrap());
        assert!(path.exists());
        assert!(
            !session
                .sync(
                    UiResume::PrView {
                        pane: "checks".into()
                    },
                    Vec::new()
                )
                .unwrap()
        );
    }

    #[test]
    fn finish_by_the_user_clears_and_removes_the_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let file = StateFile::new(dir, Kind::List);
        let id = file.id().to_owned();
        let path = file.path().to_path_buf();
        let own = StateFile::new(dir, Kind::PrView);
        own.save(&view_state(&id)).unwrap();
        let other = StateFile::new(dir, Kind::PrView);
        other.save(&view_state("ffffffffffffffff")).unwrap();
        let mut session = Session::new(file, reporter(&["/usr/bin/chaoshub"]), Fixed::List);
        session.sync(list_ui("mine"), Vec::new()).unwrap();
        let mut out = Vec::new();
        session.report(&mut out).unwrap();
        out.clear();
        session.finish(&mut out, true, dir);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("\x1b]7577;pid="), "{text}");
        assert!(text.contains(":clear=1:mac="), "{text}");
        assert!(!path.exists());
        assert!(!own.path().exists());
        assert!(other.path().exists());
    }

    #[test]
    fn finish_after_a_signal_keeps_the_record_and_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let file = StateFile::new(dir, Kind::List);
        let path = file.path().to_path_buf();
        let own = StateFile::new(dir, Kind::PrView);
        own.save(&view_state(file.id())).unwrap();
        let mut session = Session::new(file, reporter(&["/usr/bin/chaoshub"]), Fixed::List);
        session.sync(list_ui("mine"), Vec::new()).unwrap();
        let mut out = Vec::new();
        session.finish(&mut out, false, dir);
        assert_eq!(out, [] as [u8; 0]);
        assert!(path.exists());
        assert!(own.path().exists());
    }
}
