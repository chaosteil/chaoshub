//! The files pane of the pull request popup: the changed files as a
//! directory tree with `+N −M` counts per file and per directory. Enter
//! opens and closes a directory or a file; an open file shows its patch
//! under it. `o` opens the files page of the pull request.

use std::collections::{BTreeMap, HashSet};

use crate::app::models::{Cell, CellStyle, PaneModel, TreeNode};
use crate::app::panes::{Pane, PaneState};
use crate::app::tabs::cells::cell;
use crate::app::tabs::{Handled, KeyCx};
use crate::bus::{Cmd, KeyChord, Size, Target};
use crate::domain::files::{ChangedFile, FileStatus, PrFiles};
use crate::domain::pr::PullRequest;

/// The files pane.
pub(crate) struct FilesPane;

/// The own state of the pane, by path, so a reload keeps it. Directories
/// are open by default, files are closed by default.
#[derive(Debug, Default)]
pub(crate) struct FilesState {
    pub closed_dirs: HashSet<String>,
    pub open_files: HashSet<String>,
}

impl Pane for FilesPane {
    fn id(&self) -> &'static str {
        "files"
    }

    fn title(&self) -> &'static str {
        "Files"
    }

    fn key(&self) -> char {
        '5'
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
        let toggle = {
            let state = ps.ext.downcast_ref::<FilesState>()?;
            nodes(cx, state).into_iter().nth(ps.cursor)?.1?
        };
        let state = ps.ext.downcast_mut::<FilesState>()?;
        match toggle {
            Toggle::Dir(path) => {
                if !state.closed_dirs.remove(&path) {
                    state.closed_dirs.insert(path);
                }
            }
            Toggle::File(path) => {
                if !state.open_files.remove(&path) {
                    state.open_files.insert(path);
                }
            }
        }
        Some(Handled {
            changed: true,
            ..Handled::default()
        })
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
    emit(&tree(files), "", 0, state, &mut out);
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
/// trailing `/`, or empty for the root.
fn emit(
    dir: &Dir<'_>,
    prefix: &str,
    depth: u8,
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
            emit(child, &format!("{path}/"), deeper, state, out);
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
            out.extend(patch(deeper, file).into_iter().map(|n| (n, None)));
        }
    }
}

/// `name +N −M`, with a tag for a status other than `Modified`.
fn file_node(depth: u8, open: bool, name: &str, file: &ChangedFile) -> TreeNode {
    let mut cells = vec![
        cell(name, CellStyle::Normal),
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
fn patch(depth: u8, file: &ChangedFile) -> Vec<TreeNode> {
    let Some(patch) = &file.patch else {
        return vec![node(
            depth,
            None,
            vec![cell("no diff: binary or too large", CellStyle::Dim)],
        )];
    };
    patch
        .lines()
        .map(|text| {
            let style = if text.starts_with("@@") {
                CellStyle::Info
            } else if text.starts_with('+') {
                CellStyle::Success
            } else if text.starts_with('-') {
                CellStyle::Danger
            } else if text.starts_with('\\') {
                CellStyle::Dim
            } else {
                CellStyle::Normal
            };
            node(depth, None, vec![cell(text, style)])
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
    use crate::bus::{KeyCode, KeyModifiers};

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
        assert!(handled.is_some_and(|h| h.changed));
        assert_eq!(
            outline(&model)[8..16],
            [
                "      ▾ files.rs +2 −1",
                "        @@ -1,3 +1,4 @@",
                "         fn main() {",
                "        -    println!(\"old\");",
                "        +    println!(\"new\");",
                "        +    println!(\"more\");",
                "         }",
                "        \\ No newline at end of file",
            ]
        );
        let styles: Vec<CellStyle> = tree_nodes(&model)[9..16]
            .iter()
            .map(|n| n.cells[0].style)
            .collect();
        assert_eq!(
            styles,
            [
                CellStyle::Info,
                CellStyle::Normal,
                CellStyle::Danger,
                CellStyle::Success,
                CellStyle::Success,
                CellStyle::Normal,
                CellStyle::Dim,
            ]
        );
        assert!(
            tree_nodes(&model)
                .iter()
                .all(|n| n.cells.iter().all(|c| c.style != CellStyle::Markdown))
        );
        assert!(tree_nodes(&model)[9..16].iter().all(|n| n.depth == 4));
        let (model, _) = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
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
        let (model, _) = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        assert_eq!(outline(&model)[6], "  ▾ app/ +2 −1");
        ps.cursor = 0;
        let (_, handled) = model_and_keys(&state, &mut ps, &[KeyCode::Enter]);
        assert_eq!(handled, None, "Enter on the summary does nothing");
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
