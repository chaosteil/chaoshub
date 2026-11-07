//! The reader of the codeview comment files.
//!
//! codeview keeps one markdown file per review in
//! `<comments dir>/<repo slug>/<key>.md`. The frontmatter holds `version`,
//! `repo` (the absolute repo root) and `range`. For a pull request, the
//! range is `pr-<N>-<head sha>`, so each push of the head gets a new file.
//! Each comment is a section `## codeview-comment <id>`, with a block of
//! `- key: value` fields and a markdown body. A body line that starts
//! with `\` loses that first character. A comment is unsent while its
//! `synced_at` field is empty.
//!
//! chaoshub reads the files and never writes them.

use std::path::{Path, PathBuf};

use crate::config::paths;
use crate::domain::codeview::{Comment, CommentKind, Comments, Push, VERSION};

/// The text of a comment heading, before the id.
const MARKER: &str = "## codeview-comment ";

/// The fields of a comment section.
const FIELDS: &[&str] = &[
    "file",
    "start_line",
    "end_line",
    "side",
    "commit",
    "state",
    "synced_at",
    "review_id",
    "created_at",
    "updated_at",
    "remote_id",
    "reply_to",
];

/// The directory of the comment files: `config` when it is set, else
/// `$XDG_DATA_HOME/<app>/codeview/review`, else
/// `~/.local/share/<app>/codeview/review`. `<app>` is `$NVIM_APPNAME`, or
/// `nvim`. `None` when the home directory is unknown.
pub(crate) fn comments_dir(config: Option<&Path>) -> Option<PathBuf> {
    let var = |name: &str| std::env::var(name).ok();
    dir_from(
        config,
        var("XDG_DATA_HOME"),
        var("NVIM_APPNAME"),
        paths::home_dir().as_deref(),
    )
}

/// `comments_dir` with the environment as arguments.
fn dir_from(
    config: Option<&Path>,
    data_home: Option<String>,
    app: Option<String>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(config) = config {
        return Some(match home {
            Some(home) => paths::expand_tilde(config, home),
            None => config.to_path_buf(),
        });
    }
    let app = app
        .filter(|app| !app.is_empty())
        .unwrap_or_else(|| "nvim".to_owned());
    let base = match data_home.filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home?.join(".local").join("share"),
    };
    Some(base.join(app).join("codeview").join("review"))
}

/// The unsent comments of the pull request `number` of the repo at
/// `repo`, from the files in `dir`, per push. A missing directory gives no
/// comments. A file of another version is an error.
pub(crate) fn load(dir: &Path, repo: &Path, number: u64) -> Result<Comments, String> {
    let home = paths::home_dir();
    let repo = canonical(repo, home.as_deref());
    let prefix = format!("pr-{number}-");
    let mut pushes = Vec::new();
    for path in files(dir) {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "cannot read a codeview file");
                continue;
            }
        };
        let Some(file) = parse(&text) else {
            continue;
        };
        if canonical(Path::new(&file.repo), home.as_deref()) != repo {
            continue;
        }
        let Some(head) = file.range.strip_prefix(&prefix) else {
            continue;
        };
        if file.version != VERSION {
            return Err(format!(
                "{}: version {} is not {VERSION}",
                path.display(),
                file.version
            ));
        }
        let comments = file
            .comments
            .into_iter()
            .filter(|section| section.synced_at.is_empty())
            .map(|section| section.comment)
            .collect();
        pushes.push(Push {
            head: head.to_owned(),
            comments,
        });
    }
    Ok(Comments { pushes })
}

/// The `*.md` files one level under `dir`, sorted.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(inner) = std::fs::read_dir(entry.path()) else {
            continue;
        };
        out.extend(
            inner
                .flatten()
                .map(|file| file.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
                .filter(|path| path.is_file()),
        );
    }
    out.sort();
    out
}

/// `path` with `~` expanded, canonical when it exists.
fn canonical(path: &Path, home: Option<&Path>) -> PathBuf {
    let path = match home {
        Some(home) => paths::expand_tilde(path, home),
        None => path.to_path_buf(),
    };
    std::fs::canonicalize(&path).unwrap_or(path)
}

/// One comment file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct File {
    version: u32,
    repo: String,
    range: String,
    comments: Vec<Section>,
}

/// One comment section, with its sync time.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    comment: Comment,
    /// The `synced_at` value. Empty while the comment is unsent.
    synced_at: String,
}

/// Parses the text of a comment file. `None` when the text has no
/// frontmatter.
fn parse(text: &str) -> Option<File> {
    let mut lines = text.split('\n').map(|line| line.trim_end_matches('\r'));
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut version = VERSION;
    let mut repo = String::new();
    let mut range = String::new();
    let mut ended = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            ended = true;
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim() {
            "version" => version = value.parse().unwrap_or(0),
            "repo" => value.clone_into(&mut repo),
            "range" => value.clone_into(&mut range),
            _ => {}
        }
    }
    if !ended {
        return None;
    }
    let mut comments = Vec::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in lines {
        if let Some(id) = heading(line) {
            if let Some((id, body)) = current.take() {
                comments.push(section(id, &body));
            }
            current = Some((id.to_owned(), Vec::new()));
        } else if let Some((_, body)) = &mut current {
            body.push(line);
        }
    }
    if let Some((id, body)) = current {
        comments.push(section(id, &body));
    }
    Some(File {
        version,
        repo,
        range,
        comments,
    })
}

/// The id of a comment heading line. `None` for another line.
fn heading(line: &str) -> Option<&str> {
    let id = line.strip_prefix(MARKER)?.trim_end();
    let valid = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    valid.then_some(id)
}

/// A field line `- name: value` of a known field.
fn field(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix('-')?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let (name, value) = rest.trim_start().split_once(':')?;
    FIELDS.contains(&name).then(|| (name, value.trim()))
}

/// Reads one comment section: the field block, then the body.
fn section(id: String, lines: &[&str]) -> Section {
    let mut at = 0;
    while lines.get(at).is_some_and(|line| line.trim().is_empty()) {
        at += 1;
    }
    let mut comment = Comment {
        id,
        file: String::new(),
        start_line: 1,
        end_line: 1,
        side: "new".to_owned(),
        body: String::new(),
        kind: CommentKind::Line,
    };
    let mut synced_at = String::new();
    while let Some((name, value)) = lines.get(at).and_then(|line| field(line)) {
        match name {
            "file" => value.clone_into(&mut comment.file),
            "start_line" => comment.start_line = value.parse().unwrap_or(1),
            "end_line" => comment.end_line = value.parse().unwrap_or(1),
            "side" => value.clone_into(&mut comment.side),
            "synced_at" => value.clone_into(&mut synced_at),
            _ => {}
        }
        at += 1;
    }
    let body = lines.get(at..).unwrap_or_default();
    let first = body.iter().position(|line| !line.trim().is_empty());
    let last = body.iter().rposition(|line| !line.trim().is_empty());
    if let (Some(first), Some(last)) = (first, last) {
        comment.body = body[first..=last]
            .iter()
            .map(|line| line.strip_prefix('\\').unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
    }
    comment.start_line = comment.start_line.max(1);
    comment.end_line = comment.end_line.max(comment.start_line);
    if comment.side != "old" {
        "new".clone_into(&mut comment.side);
    }
    comment.kind = CommentKind::of(&comment.file);
    Section { comment, synced_at }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// A comment directory with the fixture files for the repo at `repo`:
    /// the current push `bbbb2222` and an older push `aaaa1111` of #14, a
    /// file of #1, and a file of another repo.
    fn fixture(dir: &Path, repo: &Path) {
        let slug = dir.join("repo-1a2b3c4d");
        std::fs::create_dir_all(&slug).unwrap();
        let repo = repo.to_string_lossy();
        let current = include_str!("testdata/current.md").replace("REPO", &repo);
        let older = include_str!("testdata/older.md").replace("REPO", &repo);
        std::fs::write(slug.join("current.md"), &current).unwrap();
        std::fs::write(slug.join("older.md"), &older).unwrap();
        std::fs::write(
            slug.join("other-pr.md"),
            current.replace("pr-14-bbbb2222", "pr-1-bbbb2222"),
        )
        .unwrap();
        std::fs::write(slug.join("notes.txt"), "not a comment file").unwrap();
        let other = dir.join("other-5e6f7a8b");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            other.join("current.md"),
            current.replace(&*repo, "/elsewhere/repo"),
        )
        .unwrap();
    }

    fn comment(id: &str, file: &str, lines: (u32, u32), side: &str, body: &str) -> Comment {
        Comment {
            id: id.into(),
            file: file.into(),
            start_line: lines.0,
            end_line: lines.1,
            side: side.into(),
            body: body.into(),
            kind: CommentKind::of(file),
        }
    }

    #[test]
    fn load_keeps_the_unsent_comments_of_the_pull_request_per_push() {
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fixture(dir.path(), repo.path());
        let comments = load(dir.path(), repo.path(), 14).unwrap();
        assert_eq!(
            comments.current("bbbb2222"),
            [
                comment(
                    "68a1c2f30001",
                    "src/lib.rs",
                    (12, 14),
                    "new",
                    "Trim the name.\n\n## codeview-comment looks like a heading\n\\ starts with a backslash",
                ),
                comment(
                    "68a1c2f30003",
                    "codeview://pr/14",
                    (1, 1),
                    "new",
                    "Looks good overall."
                ),
                comment(
                    "68a1c2f30004",
                    "codeview://commit/bbbb2222",
                    (1, 1),
                    "new",
                    "Fix the typo in the title."
                ),
            ]
        );
        assert_eq!(comments.older("bbbb2222"), 1);
        assert_eq!(
            comments.current("aaaa1111")[0].body,
            "A note on the first push."
        );
        let push = comments.push_of("68a1c2f30003").unwrap();
        assert_eq!(push.head, "bbbb2222");
        assert_eq!(comments.push_of("68a1c2f30002"), None, "the sent comment");
    }

    #[test]
    fn the_kinds_and_the_places_of_the_comments() {
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fixture(dir.path(), repo.path());
        let comments = load(dir.path(), repo.path(), 14).unwrap();
        let current = comments.current("bbbb2222");
        let kinds: Vec<_> = current.iter().map(|c| (c.kind, c.sendable())).collect();
        assert_eq!(
            kinds,
            [
                (CommentKind::Line, true),
                (CommentKind::Pr, true),
                (CommentKind::Message, false),
            ]
        );
        let places: Vec<_> = current.iter().map(Comment::place).collect();
        assert_eq!(places, ["src/lib.rs:L12-14 (new)", "PR", "commit message"]);
        assert_eq!(current[0].summary(), "Trim the name.");
    }

    #[test]
    fn the_reply_to_field_does_not_go_into_the_body() {
        let text = "---\nversion: 1\nrepo: /repo\nrange: pr-14-h1\n---\n\n\
                    ## codeview-comment c1\n\n- file: src/a.rs\n- start_line: 2\n\
                    - end_line: 2\n- side: new\n- synced_at:\n- reply_to: 123\n\nA reply\n";
        let file = parse(text).unwrap();
        assert_eq!(
            file.comments[0].comment,
            comment("c1", "src/a.rs", (2, 2), "new", "A reply")
        );
    }

    #[test]
    fn a_file_of_another_version_is_an_error_for_the_pull_request() {
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        fixture(dir.path(), repo.path());
        let path = dir.path().join("repo-1a2b3c4d").join("current.md");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("version: 1", "version: 2")).unwrap();
        let err = load(dir.path(), repo.path(), 14).unwrap_err();
        assert!(err.ends_with("current.md: version 2 is not 1"), "{err}");
        assert!(
            load(dir.path(), repo.path(), 1).is_ok(),
            "another pull request"
        );
    }

    #[test]
    fn a_missing_directory_gives_no_comments() {
        let dir = tempfile::tempdir().unwrap();
        let comments = load(&dir.path().join("none"), dir.path(), 14).unwrap();
        assert_eq!(comments, Comments::default());
    }

    #[test]
    fn the_comment_directory_follows_the_config_and_the_environment() {
        /// The config, `$XDG_DATA_HOME`, `$NVIM_APPNAME` and the directory.
        type Case = (
            Option<&'static str>,
            Option<String>,
            Option<String>,
            &'static str,
        );
        let home = Path::new("/home/me");
        let s = |text: &str| Some(text.to_owned());
        let cases: [Case; 5] = [
            (Some("~/reviews"), s("/data"), s("lazy"), "/home/me/reviews"),
            (
                None,
                None,
                None,
                "/home/me/.local/share/nvim/codeview/review",
            ),
            (None, s("/data"), None, "/data/nvim/codeview/review"),
            (
                None,
                s(""),
                s("lazy"),
                "/home/me/.local/share/lazy/codeview/review",
            ),
            (None, s("/data"), s("lazy"), "/data/lazy/codeview/review"),
        ];
        for (config, data, app, expected) in cases {
            assert_eq!(
                dir_from(config.map(Path::new), data, app, Some(home)),
                Some(PathBuf::from(expected)),
                "{config:?}"
            );
        }
        assert_eq!(dir_from(None, None, None, None), None);
    }
}
