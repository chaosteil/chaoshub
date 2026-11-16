//! The files pane of the pull request popup: the changed files as a
//! directory tree with `+N −M` counts per file and per directory. Enter
//! opens and closes a directory or a file; an open file shows its patch
//! under it, with the syntax colors of its language. `o` opens the files
//! page of the pull request.
//!
//! The highlight worker gives the syntax colors (see
//! `workers::highlight`). An open file shows its diff colors at once, and
//! its syntax colors when the result of the worker comes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use crate::app::models::{Cell, CellStyle, Change, PaneModel, TreeNode};
use crate::app::panes::{Pane, PaneState};
use crate::app::tabs::cells::cell;
use crate::app::tabs::{Handled, KeyCx, tree_toggled};
use crate::bus::{Cmd, HighlightJob, HighlightReq, Highlighted, KeyChord, Msg, Size, Target};
use crate::domain::diff::{LineKind, Tokens, numbered};
use crate::domain::files::{ChangedFile, FileStatus, PrFiles};
use crate::domain::keys::PrKey;
use crate::domain::pr::PullRequest;

/// The files pane.
pub(crate) struct FilesPane;

/// The own state of the pane, by path, so a reload keeps it. Directories
/// are open by default, files are closed by default.
#[derive(Debug, Default)]
pub(crate) struct FilesState {
    pub closed_dirs: HashSet<String>,
    pub open_files: HashSet<String>,
    /// The syntax tokens that the highlight worker sent, by head commit
    /// and path.
    highlights: HashMap<(String, String), Highlight>,
    /// The files with a job on the highlight worker, by head commit and
    /// path.
    waiting: HashSet<(String, String)>,
    /// The patch nodes of each open file, by head commit and path. A
    /// model build after a key press clones them, so it does not parse
    /// and style a large patch again.
    patches: Mutex<HashMap<(String, String), PatchNodes>>,
}

/// The patch nodes of one file at one depth, with or without the syntax
/// colors.
#[derive(Debug)]
struct PatchNodes {
    depth: u8,
    colored: bool,
    nodes: Arc<Vec<TreeNode>>,
}

/// The syntax tokens of the code lines of one file, `None` for a file
/// without syntax colors.
type Highlight = Option<Arc<Vec<Tokens>>>;

impl FilesState {
    /// The syntax tokens of the file at `path` at the head commit `head`.
    /// `None` until the worker sends them, and for a file without syntax
    /// colors.
    fn highlight(&self, head: &str, path: &str) -> Option<&[Tokens]> {
        let tokens = self.highlights.get(&(head.to_owned(), path.to_owned()))?;
        tokens.as_deref().map(Vec::as_slice)
    }

    /// The patch nodes of `file` at `depth` and the head commit `head`,
    /// from the cache or a new build. The cache keeps only the nodes of
    /// `head`.
    fn patch_nodes(&self, head: &str, depth: u8, file: &ChangedFile) -> Arc<Vec<TreeNode>> {
        let tokens = self.highlight(head, &file.path);
        let colored = tokens.is_some();
        let key = (head.to_owned(), file.path.clone());
        let mut cache = self.patches.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(hit) = cache.get(&key)
            && hit.depth == depth
            && hit.colored == colored
        {
            return Arc::clone(&hit.nodes);
        }
        let nodes = Arc::new(patch(depth, file, tokens));
        cache.retain(|(h, _), _| h == head);
        cache.insert(
            key,
            PatchNodes {
                depth,
                colored,
                nodes: Arc::clone(&nodes),
            },
        );
        nodes
    }

    /// The highlight jobs for the open files of `files` of the pull
    /// request `key` that have no tokens and no job yet. The tokens and
    /// the jobs of the other heads go away.
    fn jobs(&mut self, key: &PrKey, files: &PrFiles) -> Vec<Cmd> {
        let head = &files.head_oid;
        self.highlights.retain(|(h, _), _| h == head);
        self.waiting.retain(|(h, _)| h == head);
        files
            .files
            .iter()
            .filter(|file| self.open_files.contains(&file.path))
            .filter_map(|file| {
                let patch = file.patch.as_ref()?;
                let id = (head.clone(), file.path.clone());
                if self.highlights.contains_key(&id) || !self.waiting.insert(id) {
                    return None;
                }
                Some(Cmd::Highlight(HighlightReq::File(HighlightJob {
                    key: key.clone(),
                    head: head.clone(),
                    path: file.path.clone(),
                    patch: patch.clone(),
                })))
            })
            .collect()
    }

    /// The cancel of the waiting highlight job of the file at `path`,
    /// after the user closed the file.
    fn cancel(&mut self, cx: &KeyCx<'_>, path: String) -> Option<Cmd> {
        let (key, files) = target_files(cx)?;
        let id = (files.head_oid.clone(), path);
        self.waiting.remove(&id).then(|| {
            Cmd::Highlight(HighlightReq::Cancel {
                key: key.clone(),
                path: id.1,
            })
        })
    }

    /// Keeps the result of a highlight job when it is for `head`, the
    /// head of the files now. True when it kept the result.
    fn highlighted(&mut self, done: &Highlighted, head: &str) -> bool {
        let id = (done.head.clone(), done.path.clone());
        self.waiting.remove(&id);
        if done.head != head {
            tracing::debug!(path = done.path, "a highlight for an old head, dropped");
            return false;
        }
        self.highlights.insert(id, done.lines.clone());
        true
    }
}

/// The pull request and the changed files of the target of `cx`.
fn target_files<'a>(cx: &KeyCx<'a>) -> Option<(&'a PrKey, &'a PrFiles)> {
    let Some(Target::Pr { pr, .. }) = cx.target else {
        return None;
    };
    let files = cx.data.files.get(&pr.key)?;
    Some((&pr.key, &files.value))
}

impl Pane for FilesPane {
    fn id(&self) -> &'static str {
        "files"
    }

    fn title(&self) -> &'static str {
        "Files"
    }

    fn key(&self) -> char {
        '4'
    }

    fn applies(&self, target: &Target) -> bool {
        matches!(target, Target::Pr { .. })
    }

    fn init_state(&self) -> PaneState {
        PaneState {
            ext: Box::new(FilesState::default()),
            ..PaneState::default()
        }
    }

    fn model(&self, cx: &KeyCx<'_>, ps: &PaneState, _size: Size) -> PaneModel {
        let empty = FilesState::default();
        let state = ps.ext.downcast_ref::<FilesState>().unwrap_or(&empty);
        PaneModel::Tree {
            nodes: nodes(cx, state).into_iter().map(|(node, _)| node).collect(),
        }
    }

    fn handle_key(&self, key: KeyChord, cx: &KeyCx<'_>, ps: &mut PaneState) -> Option<Handled> {
        if cx.keys.matches("open_url", key) {
            let Some(Target::Pr { pr, .. }) = cx.target else {
                return None;
            };
            return Some(Handled {
                cmds: vec![Cmd::OpenUrl(format!("{}/files", pr.head.value.url))],
                ..Handled::default()
            });
        }
        if !cx.keys.matches("open", key) {
            return None;
        }
        let (toggle, before) = {
            let state = ps.ext.downcast_ref::<FilesState>()?;
            let nodes = nodes(cx, state);
            let before = nodes.len();
            (nodes.into_iter().nth(ps.cursor)?.1?, before)
        };
        let state = ps.ext.downcast_mut::<FilesState>()?;
        let mut cmds = Vec::new();
        match toggle {
            Toggle::Dir(path) => {
                if !state.closed_dirs.remove(&path) {
                    state.closed_dirs.insert(path);
                }
            }
            Toggle::File(path) => {
                if state.open_files.remove(&path) {
                    cmds.extend(state.cancel(cx, path));
                } else {
                    state.open_files.insert(path);
                    if let Some((key, files)) = target_files(cx) {
                        cmds.extend(state.jobs(key, files));
                    }
                }
            }
        }
        let after = nodes(cx, state).len();
        let mut handled = tree_toggled(ps.cursor, before, after);
        handled.cmds = cmds;
        Some(handled)
    }

    fn jobs(&self, cx: &KeyCx<'_>, ps: &mut PaneState) -> Vec<Cmd> {
        let Some(state) = ps.ext.downcast_mut::<FilesState>() else {
            return Vec::new();
        };
        target_files(cx).map_or_else(Vec::new, |(key, files)| state.jobs(key, files))
    }

    fn receive(&self, msg: &Msg, cx: &KeyCx<'_>, ps: &mut PaneState) -> bool {
        let Msg::Highlighted(done) = msg else {
            return false;
        };
        let (Some(state), Some((key, files))) =
            (ps.ext.downcast_mut::<FilesState>(), target_files(cx))
        else {
            return false;
        };
        *key == done.key && state.highlighted(done, &files.head_oid)
    }
}

/// What Enter toggles on a node.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Toggle {
    /// A directory, by its path without the trailing `/`.
    Dir(String),
    /// A file, by its path.
    File(String),
}

/// A directory of the tree: its subdirectories by name, its files and
/// the summed counts of every file under it.
#[derive(Default)]
struct Dir<'a> {
    dirs: BTreeMap<&'a str, Dir<'a>>,
    files: Vec<(&'a str, &'a ChangedFile)>,
    additions: u32,
    deletions: u32,
}

/// The directory tree of `files`, one directory per path part.
fn tree(files: &PrFiles) -> Dir<'_> {
    let mut sorted: Vec<&ChangedFile> = files.files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut root = Dir::default();
    for file in sorted {
        let mut parts: Vec<&str> = file.path.split('/').collect();
        let name = parts.pop().unwrap_or_default();
        let mut dir = &mut root;
        for part in parts {
            dir = dir.dirs.entry(part).or_default();
            dir.additions = dir.additions.saturating_add(file.additions);
            dir.deletions = dir.deletions.saturating_add(file.deletions);
        }
        dir.files.push((name, file));
    }
    root
}

/// A node at `depth` with `cells`.
fn node(depth: u8, expanded: Option<bool>, cells: Vec<Cell>) -> TreeNode {
    TreeNode {
        depth,
        expanded,
        cells,
    }
}

/// One line of text in `style` at depth 0.
fn line(text: impl Into<String>, style: CellStyle) -> TreeNode {
    node(0, None, vec![cell(text, style)])
}

/// The nodes of the pane, each with what Enter toggles on it.
fn nodes(cx: &KeyCx<'_>, state: &FilesState) -> Vec<(TreeNode, Option<Toggle>)> {
    let Some(Target::Pr { pr, .. }) = cx.target else {
        return Vec::new();
    };
    let mut out: Vec<(TreeNode, Option<Toggle>)> = Vec::new();
    let error = cx.data.files_errors.get(&pr.key);
    if let Some(reason) = error {
        out.push((
            line(
                format!("cannot load the files: {reason}"),
                CellStyle::Danger,
            ),
            None,
        ));
    }
    let Some(files) = cx.data.files.get(&pr.key) else {
        // After a failed load, the error is the whole content, until a
        // new load runs.
        if error.is_none() || cx.data.files_loading.contains(&pr.key) {
            out.push((line("loading the files…", CellStyle::Dim), None));
        }
        return out;
    };
    let files = &files.value;
    if is_stale(files, pr) {
        out.push((
            line(
                "stale: read at an older head, a new load is on its way",
                CellStyle::Warn,
            ),
            None,
        ));
    }
    out.push((summary(files), None));
    if files.files.is_empty() {
        out.push((line("no changed files", CellStyle::Dim), None));
        return out;
    }
    emit(&tree(files), "", 0, &files.head_oid, state, &mut out);
    out
}

/// True when the files were read at another head than the pull request
/// has now.
fn is_stale(files: &PrFiles, pr: &PullRequest) -> bool {
    files.head_oid != pr.head.value.head_oid
}

/// `12 files · +340 −56`, and a hint when GitHub has more files.
fn summary(files: &PrFiles) -> TreeNode {
    let count = files.files.len();
    let noun = if count == 1 { "file" } else { "files" };
    let additions: u32 = files.files.iter().map(|f| f.additions).sum();
    let deletions: u32 = files.files.iter().map(|f| f.deletions).sum();
    let mut cells = vec![
        cell(format!("{count} {noun} · "), CellStyle::Dim),
        cell(format!("+{additions}"), CellStyle::Success),
        cell(format!(" −{deletions}"), CellStyle::Danger),
    ];
    if files.truncated {
        cells.push(cell(
            " · more files on GitHub, o opens them",
            CellStyle::Warn,
        ));
    }
    node(0, None, cells)
}

/// The nodes of `dir` at `depth`: the subdirectories first, then the
/// files, each group by name. `prefix` is the path of `dir` with a
/// trailing `/`, or empty for the root. `head` is the head commit of the
/// files.
fn emit(
    dir: &Dir<'_>,
    prefix: &str,
    depth: u8,
    head: &str,
    state: &FilesState,
    out: &mut Vec<(TreeNode, Option<Toggle>)>,
) {
    let deeper = depth.saturating_add(1);
    for (name, child) in &dir.dirs {
        let path = format!("{prefix}{name}");
        let open = !state.closed_dirs.contains(&path);
        out.push((
            node(
                depth,
                Some(open),
                vec![
                    cell(format!("{name}/"), CellStyle::Info),
                    cell(
                        format!(" +{} −{}", child.additions, child.deletions),
                        CellStyle::Dim,
                    ),
                ],
            ),
            Some(Toggle::Dir(path.clone())),
        ));
        if open {
            emit(child, &format!("{path}/"), deeper, head, state, out);
        }
    }
    let mut files = dir.files.clone();
    files.sort_by_key(|(name, _)| *name);
    for (name, file) in files {
        let open = state.open_files.contains(&file.path);
        out.push((
            file_node(depth, open, name, file),
            Some(Toggle::File(file.path.clone())),
        ));
        if open {
            let nodes = state.patch_nodes(head, deeper, file);
            out.extend(nodes.iter().map(|n| (n.clone(), None)));
        }
    }
}

/// `name +N −M`, with a tag for a status other than `Modified`. An open
/// file is the header of its diff, on the surface band.
fn file_node(depth: u8, open: bool, name: &str, file: &ChangedFile) -> TreeNode {
    let name_style = if open {
        CellStyle::DiffFile
    } else {
        CellStyle::Normal
    };
    let mut cells = vec![
        cell(name, name_style),
        cell(format!(" +{}", file.additions), CellStyle::Success),
        cell(format!(" −{}", file.deletions), CellStyle::Danger),
    ];
    let tag = match file.status {
        FileStatus::Modified => None,
        FileStatus::Added => Some("added".to_owned()),
        FileStatus::Removed => Some("removed".to_owned()),
        FileStatus::Renamed => Some(match &file.previous_path {
            Some(previous) => format!("renamed from {previous}"),
            None => "renamed".to_owned(),
        }),
        FileStatus::Copied => Some("copied".to_owned()),
        FileStatus::Changed => Some("changed".to_owned()),
        FileStatus::Unchanged => Some("unchanged".to_owned()),
    };
    if let Some(tag) = tag {
        cells.push(cell(format!(" {tag}"), CellStyle::Dim));
    }
    node(depth, Some(open), cells)
}

/// One leaf per patch line of `file` at `depth`, or one `no diff` leaf.
/// Each line but a hunk header starts with its line number in a dim
/// gutter. An added line is green and a removed line red, each on its
/// tint. A hunk header is muted on its own tint. With `tokens`, one per
/// code line, the `+` or `-` keeps the color of the change and the code
/// gets the syntax colors.
fn patch(depth: u8, file: &ChangedFile, tokens: Option<&[Tokens]>) -> Vec<TreeNode> {
    let Some(patch) = &file.patch else {
        return vec![node(
            depth,
            None,
            vec![cell("no diff: binary or too large", CellStyle::Dim)],
        )];
    };
    let lines = numbered(patch);
    let width = lines
        .iter()
        .filter_map(|(_, number, _)| *number)
        .max()
        .map_or(1, |max| max.to_string().len());
    let mut tokens = tokens.map(<[Tokens]>::iter);
    lines
        .into_iter()
        .map(|(kind, number, text)| {
            let gutter = match number {
                Some(n) => format!("{n:>width$} "),
                None => " ".repeat(width + 1),
            };
            let (style, change) = match kind {
                LineKind::Hunk => return node(depth, None, vec![cell(text, CellStyle::DiffHunk)]),
                LineKind::Note => {
                    let cells = vec![
                        cell(gutter, CellStyle::DiffGutter),
                        cell(text, CellStyle::Dim),
                    ];
                    return node(depth, None, cells);
                }
                LineKind::Added => (CellStyle::DiffAdd, Change::Added),
                LineKind::Removed => (CellStyle::DiffDel, Change::Removed),
                LineKind::Context => (CellStyle::Normal, Change::Context),
            };
            let mut cells = vec![cell(gutter, CellStyle::DiffGutter)];
            match tokens.as_mut().and_then(Iterator::next) {
                Some(line) => {
                    cells.push(cell(text.get(..1).unwrap_or_default(), style));
                    cells.extend(line.iter().map(|(syntax, piece)| {
                        cell(piece.as_str(), CellStyle::Code(*syntax, change))
                    }));
                }
                None => cells.push(cell(text, style)),
            }
            node(depth, None, cells)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::keymap::Keymap;
    use crate::app::layout::layout;
    use crate::app::state::AppState;
    use crate::app::target::pr_target;
    use crate::app::testdata;
    use crate::bus::{KeyCode, KeyModifiers, UiEvent};
    use crate::domain::diff::Syntax;

    /// The text of each node, with its depth and expand marker.
    fn outline(model: &PaneModel) -> Vec<String> {
        let PaneModel::Tree { nodes } = model else {
            panic!("not a tree");
        };
        nodes
            .iter()
            .map(|n| {
                let marker = match n.expanded {
                    Some(true) => "▾ ",
                    Some(false) => "▸ ",
                    None => "",
                };
                let text: String = n.cells.iter().map(|c| c.text.as_str()).collect();
                format!("{}{marker}{text}", "  ".repeat(usize::from(n.depth)))
            })
            .collect()
    }

    /// The nodes of a tree model.
    fn tree_nodes(model: &PaneModel) -> &[TreeNode] {
        let PaneModel::Tree { nodes } = model else {
            panic!("not a tree");
        };
        nodes
    }

    fn state_with(files: Option<PrFiles>, error: Option<&str>, loading: bool) -> AppState {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        let pr = testdata::pr("chaoshub", 7, "Seven");
        if let Some(files) = files {
            state
                .data
                .files
                .insert(pr.key.clone(), testdata::observed(files));
        }
        if let Some(error) = error {
            state.data.files_errors.insert(pr.key.clone(), error.into());
        }
        if loading {
            state.data.files_loading.insert(pr.key.clone());
        }
        state.data.prs.insert(pr.key.clone(), pr);
        state
    }

    /// Sends `keys` to the pane, then returns its model and the result of
    /// the last key.
    fn model_and_keys(
        state: &AppState,
        ps: &mut PaneState,
        keys: &[KeyCode],
    ) -> (PaneModel, Option<Handled>) {
        let key = testdata::pr_key("chaoshub", 7);
        let target = pr_target(&state.data, &key).unwrap();
        let areas = layout(state.data.size);
        let keymap = Keymap::default();
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &keymap,
        };
        let mut handled = None;
        for &code in keys {
            let chord = KeyChord {
                code,
                mods: KeyModifiers::NONE,
            };
            handled = FilesPane.handle_key(chord, &cx, ps);
        }
        let model = FilesPane.model(
            &cx,
            ps,
            Size {
                width: 80,
                height: 20,
            },
        );
        (model, handled)
    }

    /// The highlight job of the file at `path` of the test files at the
    /// head `head`.
    fn job(path: &str, head: &str) -> Cmd {
        let file = testdata::files()
            .files
            .into_iter()
            .find(|f| f.path == path)
            .unwrap();
        Cmd::Highlight(HighlightReq::File(HighlightJob {
            key: testdata::pr_key("chaoshub", 7),
            head: head.into(),
            path: path.into(),
            patch: file.patch.unwrap(),
        }))
    }

    /// Gives the pane the result of the highlight worker for the file at
    /// `path`, read at the head `head`. True when the pane took it.
    fn deliver(state: &AppState, ps: &mut PaneState, path: &str, head: &str) -> bool {
        let file = testdata::files()
            .files
            .into_iter()
            .find(|f| f.path == path)
            .unwrap();
        let done = Highlighted {
            key: testdata::pr_key("chaoshub", 7),
            head: head.into(),
            path: path.into(),
            lines: crate::highlight::patch(path, file.patch.as_deref().unwrap()).map(Arc::new),
        };
        let key = testdata::pr_key("chaoshub", 7);
        let target = pr_target(&state.data, &key).unwrap();
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &Keymap::default(),
        };
        FilesPane.receive(&Msg::Highlighted(done), &cx, ps)
    }

    #[test]
    fn the_tree_shows_nested_directories_with_counts() {
        let state = state_with(Some(testdata::files()), None, false);
        let mut ps = FilesPane.init_state();
        let (model, _) = model_and_keys(&state, &mut ps, &[]);
        assert_eq!(
            outline(&model),
            [
                "5 files · +6 −4",
                "▾ assets/ +0 −0",
                "  ▸ logo.png +0 −0",
                "▾ docs/ +3 −0",
                "  ▸ usage.md +3 −0 added",
                "▾ src/ +3 −2",
                "  ▾ app/ +2 −1",
                "    ▾ panes/ +2 −1",
                "      ▸ files.rs +2 −1",
                "  ▸ main.rs +1 −1 renamed from src/start.rs",
                "▸ old.txt +0 −2 removed",
            ]
        );
        let nodes = tree_nodes(&model);
        let styles = |index: usize| -> Vec<CellStyle> {
            nodes[index].cells.iter().map(|c| c.style).collect()
        };
        assert_eq!(
            styles(0),
            [CellStyle::Dim, CellStyle::Success, CellStyle::Danger]
        );
        assert_eq!(styles(5), [CellStyle::Info, CellStyle::Dim]);
        assert_eq!(
            styles(4),
            [
                CellStyle::Normal,
                CellStyle::Success,
                CellStyle::Danger,
                CellStyle::Dim
            ]
        );
        assert_eq!(
            styles(8),
            [CellStyle::Normal, CellStyle::Success, CellStyle::Danger]
        );
    }

    #[test]
    fn enter_on_a_file_shows_and_hides_its_patch() {
        let state = state_with(Some(testdata::files()), None, false);
        let mut ps = FilesPane.init_state();
        ps.cursor = 8;
        let (model, handled) = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        let handled = handled.unwrap();
        assert!(handled.changed);
        assert_eq!(
            handled.events,
            [UiEvent::TreeExpanded {
                first_row: 9,
                rows: 7
            }],
            "the patch unfolds"
        );
        assert_eq!(
            outline(&model)[8..16],
            [
                "      ▾ files.rs +2 −1",
                "        @@ -1,3 +1,4 @@",
                "        1  fn main() {",
                "        2 -    println!(\"old\");",
                "        2 +    println!(\"new\");",
                "        3 +    println!(\"more\");",
                "        4  }",
                "          \\ No newline at end of file",
            ],
            "a removed line has its old number, the others their new number"
        );
        assert_eq!(
            tree_nodes(&model)[8].cells[0].style,
            CellStyle::DiffFile,
            "the open file is the header of its diff"
        );
        let lines = &tree_nodes(&model)[9..16];
        let styles: Vec<Vec<CellStyle>> = lines
            .iter()
            .map(|n| n.cells.iter().take(2).map(|c| c.style).collect())
            .collect();
        let gutter = CellStyle::DiffGutter;
        assert_eq!(
            styles,
            [
                vec![CellStyle::DiffHunk],
                vec![gutter, CellStyle::Normal],
                vec![gutter, CellStyle::DiffDel],
                vec![gutter, CellStyle::DiffAdd],
                vec![gutter, CellStyle::DiffAdd],
                vec![gutter, CellStyle::Normal],
                vec![gutter, CellStyle::Dim],
            ],
            "the gutter, then the +, - or space in the color of the change"
        );
        assert!(
            lines.iter().all(|n| n.cells.len() <= 2),
            "no syntax colors before the worker sends them"
        );
        let file = &testdata::files().files[2];
        assert_eq!(
            handled.cmds,
            [job(&file.path, "oid7")],
            "the open file goes to the highlight worker"
        );
        assert!(deliver(&state, &mut ps, &file.path, "oid7"));
        let (model, _) = model_and_keys(&state, &mut ps, &[]);
        let lines = &tree_nodes(&model)[9..16];
        // The code after the sign has the syntax colors of Rust.
        let has = |index: usize, syntax, change, text: &str| {
            lines[index].cells[2..]
                .iter()
                .any(|c| c.style == CellStyle::Code(syntax, change) && c.text.contains(text))
        };
        assert!(has(1, Syntax::Keyword, Change::Context, "fn"));
        assert!(has(1, Syntax::Function, Change::Context, "main"));
        assert!(has(2, Syntax::String, Change::Removed, "\"old\""));
        assert!(has(3, Syntax::String, Change::Added, "\"new\""));
        assert_eq!(lines[6].cells.len(), 2, "the note gets no syntax colors");
        assert!(tree_nodes(&model).iter().all(|n| {
            n.cells
                .iter()
                .all(|c| !matches!(c.style, CellStyle::Markdown | CellStyle::Html))
        }));
        assert!(tree_nodes(&model)[9..16].iter().all(|n| n.depth == 4));
        let (model, handled) = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        let handled = handled.unwrap();
        assert_eq!(handled.events, [], "a collapse is instant");
        assert_eq!(handled.cmds, [], "no job waits for the file");
        assert_eq!(outline(&model)[8], "      ▸ files.rs +2 −1");
        assert_eq!(
            outline(&model)[9],
            "  ▸ main.rs +1 −1 renamed from src/start.rs"
        );
    }

    #[test]
    fn enter_on_a_directory_hides_its_children() {
        let state = state_with(Some(testdata::files()), None, false);
        let mut ps = FilesPane.init_state();
        ps.cursor = 5;
        let (model, _) = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        assert_eq!(
            outline(&model)[5..],
            ["▸ src/ +3 −2", "▸ old.txt +0 −2 removed"]
        );
        let (model, handled) = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        assert_eq!(outline(&model)[6], "  ▾ app/ +2 −1");
        assert!(matches!(
            handled.unwrap().events.as_slice(),
            [UiEvent::TreeExpanded { first_row: 6, .. }]
        ));
        ps.cursor = 0;
        let (_, handled) = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        assert_eq!(handled, None, "Enter on the summary does nothing");
    }

    #[test]
    fn the_gutter_counts_from_each_hunk_header() {
        let text = "@@ -8,2 +9,3 @@ fn main\n a\n-b\n+c\n+d\n@@ -98 +100,2 @@\n e\n+f";
        let file = ChangedFile {
            path: "a.rs".into(),
            previous_path: None,
            status: FileStatus::Modified,
            additions: 3,
            deletions: 1,
            patch: Some(text.into()),
        };
        let gutters: Vec<String> = patch(0, &file, None)
            .iter()
            .map(|n| n.cells[0].text.clone())
            .collect();
        assert_eq!(
            gutters[..3],
            ["@@ -8,2 +9,3 @@ fn main", "  9 ", "  9 "],
            "the widest number sets the gutter width"
        );
    }

    #[test]
    fn a_file_without_a_known_language_keeps_the_diff_colors() {
        let file = ChangedFile {
            path: "notes.unknown".into(),
            previous_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 0,
            patch: Some("@@ -1 +1,2 @@\n fn a\n+fn b".into()),
        };
        let nodes = patch(0, &file, None);
        let styles: Vec<CellStyle> = nodes[2].cells.iter().map(|c| c.style).collect();
        assert_eq!(styles, [CellStyle::DiffGutter, CellStyle::DiffAdd]);
        assert_eq!(nodes[2].cells[1].text, "+fn b");
    }

    #[test]
    fn a_result_for_an_old_head_is_dropped() {
        let mut state = state_with(Some(testdata::files()), None, false);
        let mut ps = FilesPane.init_state();
        ps.cursor = 8;
        model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        let path = "src/app/panes/files.rs";
        assert!(!deliver(&state, &mut ps, path, "an old head"));
        let (model, _) = model_and_keys(&state, &mut ps, &[]);
        assert!(
            tree_nodes(&model)[9..16].iter().all(|n| n.cells.len() <= 2),
            "no syntax colors from the old head"
        );
        assert!(deliver(&state, &mut ps, path, "oid7"));
        // A load at a new head drops the tokens and asks for new ones.
        let key = testdata::pr_key("chaoshub", 7);
        let mut files = testdata::files();
        files.head_oid = "oid8".into();
        state
            .data
            .files
            .insert(key.clone(), testdata::observed(files));
        let target = pr_target(&state.data, &key).unwrap();
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &Keymap::default(),
        };
        assert_eq!(FilesPane.jobs(&cx, &mut ps), [job(path, "oid8")]);
        assert_eq!(FilesPane.jobs(&cx, &mut ps), [], "one job per file");
        let model = FilesPane.model(&cx, &ps, state.data.size);
        assert!(
            tree_nodes(&model)
                .iter()
                .flat_map(|n| &n.cells)
                .all(|c| !matches!(c.style, CellStyle::Code(..))),
            "the tokens of the old head are gone"
        );
    }

    #[test]
    fn the_patch_nodes_are_built_once_per_head_and_highlight() {
        let state = state_with(Some(testdata::files()), None, false);
        let mut ps = FilesPane.init_state();
        ps.cursor = 8;
        model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        let cached = |ps: &PaneState| {
            let files = ps.ext.downcast_ref::<FilesState>().unwrap();
            let patches = files.patches.lock().unwrap();
            let hit = &patches[&("oid7".to_owned(), "src/app/panes/files.rs".to_owned())];
            (Arc::clone(&hit.nodes), hit.colored)
        };
        let (first, colored) = cached(&ps);
        assert!(!colored);
        model_and_keys(&state, &mut ps, &[KeyCode::Char('j')]);
        assert!(Arc::ptr_eq(&first, &cached(&ps).0), "a key reads the cache");
        assert!(deliver(&state, &mut ps, "src/app/panes/files.rs", "oid7"));
        model_and_keys(&state, &mut ps, &[]);
        let (colored_nodes, colored) = cached(&ps);
        assert!(colored, "the syntax colors build the nodes again");
        assert!(!Arc::ptr_eq(&first, &colored_nodes));
    }

    #[test]
    fn closing_a_file_cancels_its_waiting_job() {
        let state = state_with(Some(testdata::files()), None, false);
        let mut ps = FilesPane.init_state();
        ps.cursor = 8;
        model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        let (_, handled) = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        assert_eq!(
            handled.unwrap().cmds,
            [Cmd::Highlight(HighlightReq::Cancel {
                key: testdata::pr_key("chaoshub", 7),
                path: "src/app/panes/files.rs".into(),
            })]
        );
        let (_, handled) = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        assert_eq!(
            handled.unwrap().cmds,
            [job("src/app/panes/files.rs", "oid7")],
            "an open again asks again"
        );
    }

    #[test]
    fn a_file_without_a_patch_shows_no_diff() {
        let state = state_with(Some(testdata::files()), None, false);
        let mut ps = FilesPane.init_state();
        ps.cursor = 2;
        let (model, _) = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        assert_eq!(
            outline(&model)[2..4],
            ["  ▾ logo.png +0 −0", "    no diff: binary or too large"]
        );
        assert_eq!(tree_nodes(&model)[3].cells[0].style, CellStyle::Dim);
    }

    #[test]
    fn loading_error_empty_stale_and_truncated_lines() {
        let mut ps = FilesPane.init_state();
        let loading = state_with(None, None, true);
        assert_eq!(
            outline(&model_and_keys(&loading, &mut ps, &[]).0),
            ["loading the files…"]
        );
        let failed = state_with(None, Some("offline"), false);
        assert_eq!(
            outline(&model_and_keys(&failed, &mut ps, &[]).0),
            ["cannot load the files: offline"],
            "no loading line under the error"
        );
        let retry = state_with(None, Some("offline"), true);
        assert_eq!(
            outline(&model_and_keys(&retry, &mut ps, &[]).0),
            ["cannot load the files: offline", "loading the files…"]
        );
        let empty = PrFiles {
            files: Vec::new(),
            truncated: false,
            head_oid: "oid7".into(),
        };
        let empty = state_with(Some(empty), None, false);
        assert_eq!(
            outline(&model_and_keys(&empty, &mut ps, &[]).0),
            ["0 files · +0 −0", "no changed files"]
        );
        let mut old = testdata::files();
        old.head_oid = "an older head".into();
        let stale = state_with(Some(old), None, false);
        assert_eq!(
            outline(&model_and_keys(&stale, &mut ps, &[]).0)[0],
            "stale: read at an older head, a new load is on its way"
        );
        let mut more = testdata::files();
        more.truncated = true;
        let truncated = state_with(Some(more), None, false);
        let (model, _) = model_and_keys(&truncated, &mut ps, &[]);
        assert_eq!(
            outline(&model)[0],
            "5 files · +6 −4 · more files on GitHub, o opens them"
        );
        assert_eq!(tree_nodes(&model)[0].cells[3].style, CellStyle::Warn);
    }

    #[test]
    fn o_opens_the_files_page() {
        let state = state_with(Some(testdata::files()), None, false);
        let mut ps = FilesPane.init_state();
        let (_, handled) = model_and_keys(&state, &mut ps, &[KeyCode::Char('o')]);
        assert_eq!(
            handled.map(|h| h.cmds),
            Some(vec![Cmd::OpenUrl(
                "https://github.com/chaosteil/chaoshub/pull/7/files".into()
            )])
        );
    }
}
