//! The changed words of the paired lines of a diff.
//!
//! A run of removed lines that an added run follows directly is one
//! change block. The lines of a block pair up by their position: the
//! first removed line with the first added line, and so on, as GitHub
//! and the Pierre diff view do. The lines that are left over have no
//! pair. In each pair, a word diff finds the changed words.
//!
//! A block or a pair gets no marks when the marks do not help:
//!
//! - One side of the block has more than `MAX_SIZE_RATIO` times the
//!   lines of the other side. The lines then are mostly new code.
//! - One line of the pair is longer than `MAX_LINE` bytes.
//! - Less than `MIN_EQUAL` of the bytes of the pair are equal. The line
//!   tint already shows that the line changed.

use std::ops::Range;

use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::domain::diff::LineKind;

/// The byte ranges of the changed words of one line.
type Marks = Vec<Range<usize>>;

/// A block whose one side has more than this times the lines of the
/// other side gets no marks.
const MAX_SIZE_RATIO: usize = 3;

/// A pair with a line longer than this gets no marks, as in the Pierre
/// diff view (`maxLineDiffLength`).
const MAX_LINE: usize = 1000;

/// A pair with a smaller part of equal bytes than this gets no marks.
const MIN_EQUAL: f64 = 0.5;

/// The byte ranges of the changed words of each code line of a patch.
/// `lines` holds the kind and the code of each patch line. The result
/// has one entry per added, removed and context line, in order. An entry
/// without a range has no marks. A hunk header ends a block, and a note
/// (`\ No newline at end of file`) does not.
pub(crate) fn changed_words(lines: &[(LineKind, &str)]) -> Vec<Marks> {
    let mut block = Block::default();
    let mut out: Vec<Marks> = Vec::new();
    let mut code: Vec<&str> = Vec::new();
    for &(kind, text) in lines {
        match kind {
            LineKind::Removed if !block.added.is_empty() => {
                block.flush(&code, &mut out);
                block.removed.push(code.len());
            }
            LineKind::Removed => block.removed.push(code.len()),
            LineKind::Added => block.added.push(code.len()),
            LineKind::Context | LineKind::Hunk => block.flush(&code, &mut out),
            LineKind::Note => {}
        }
        if matches!(
            kind,
            LineKind::Added | LineKind::Removed | LineKind::Context
        ) {
            code.push(text);
            out.push(Vec::new());
        }
    }
    block.flush(&code, &mut out);
    out
}

/// The removed and the added lines of one change block, by their index
/// in the code lines.
#[derive(Debug, Default)]
struct Block {
    removed: Vec<usize>,
    added: Vec<usize>,
}

impl Block {
    /// Puts the marks of the pairs of the block into `out` and empties
    /// the block.
    fn flush(&mut self, code: &[&str], out: &mut [Marks]) {
        let (small, big) = (
            self.removed.len().min(self.added.len()),
            self.removed.len().max(self.added.len()),
        );
        if small > 0 && big <= small * MAX_SIZE_RATIO {
            for (&old, &new) in self.removed.iter().zip(&self.added) {
                if let Some((old_marks, new_marks)) = pair(code[old], code[new]) {
                    out[old] = old_marks;
                    out[new] = new_marks;
                }
            }
        }
        self.removed.clear();
        self.added.clear();
    }
}

/// The changed words of a removed line `old` and its added line `new`,
/// as byte ranges in each line. `None` when the pair gets no marks.
fn pair(old: &str, new: &str) -> Option<(Marks, Marks)> {
    if old.len() > MAX_LINE || new.len() > MAX_LINE {
        return None;
    }
    let (old_words, new_words) = (words(old), words(new));
    let old_texts: Vec<&str> = old_words.iter().map(|r| &old[r.clone()]).collect();
    let new_texts: Vec<&str> = new_words.iter().map(|r| &new[r.clone()]).collect();
    let mut equal = 0;
    let (mut old_marks, mut new_marks) = (Vec::new(), Vec::new());
    for op in capture_diff_slices(Algorithm::Myers, &old_texts, &new_texts) {
        match op {
            DiffOp::Equal { old_index, len, .. } => {
                equal += span(&old_words, old_index, len).len();
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => old_marks.push(span(&old_words, old_index, old_len)),
            DiffOp::Insert {
                new_index, new_len, ..
            } => new_marks.push(span(&new_words, new_index, new_len)),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                old_marks.push(span(&old_words, old_index, old_len));
                new_marks.push(span(&new_words, new_index, new_len));
            }
        }
    }
    let total = old.len() + new.len();
    #[allow(
        clippy::cast_precision_loss,
        reason = "a line has at most MAX_LINE bytes"
    )]
    let similar = (2 * equal) as f64 / total.max(1) as f64;
    if similar < MIN_EQUAL || (old_marks.is_empty() && new_marks.is_empty()) {
        return None;
    }
    Some((join(old, old_marks), join(new, new_marks)))
}

/// The byte range of `len` words from the word `start`.
fn span(words: &[Range<usize>], start: usize, len: usize) -> Range<usize> {
    let first = words.get(start).map_or(0, |r| r.start);
    let last = words.get(start + len - 1).map_or(first, |r| r.end);
    first..last
}

/// The word ranges of `text`: each run of letters, digits and `_`, each
/// run of white space, and each other char alone.
fn words(text: &str) -> Vec<Range<usize>> {
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            1
        } else if c.is_whitespace() {
            2
        } else {
            0
        }
    };
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut last = None;
    for (at, c) in text.char_indices() {
        let kind = class(c);
        let end = at + c.len_utf8();
        match out.last_mut() {
            Some(range) if kind != 0 && last == Some(kind) => range.end = end,
            _ => out.push(at..end),
        }
        last = Some(kind);
    }
    out
}

/// The ranges of `marks` in order, with the ranges that touch or that
/// only one char divides joined into one. Thus `a b` that changed to
/// `c d` is one mark, not two.
fn join(text: &str, marks: Marks) -> Marks {
    let mut out: Marks = Vec::new();
    for mark in marks.into_iter().filter(|m| !m.is_empty()) {
        match out.last_mut() {
            Some(last)
                if text
                    .get(last.end..mark.start)
                    .is_some_and(|gap| gap.chars().count() <= 1) =>
            {
                last.end = mark.end;
            }
            _ => out.push(mark),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// The marked text of each code line.
    fn marked(lines: &[(LineKind, &str)]) -> Vec<Vec<String>> {
        let code: Vec<&str> = lines
            .iter()
            .filter(|(kind, _)| !matches!(kind, LineKind::Hunk | LineKind::Note))
            .map(|(_, text)| *text)
            .collect();
        changed_words(lines)
            .into_iter()
            .zip(code)
            .map(|(ranges, text)| ranges.into_iter().map(|r| text[r].to_owned()).collect())
            .collect()
    }

    #[test]
    fn the_lines_of_a_block_pair_by_position() {
        use LineKind::{Added, Context, Removed};
        let lines = [
            (Context, "fn main() {"),
            (Removed, "    let x = 1;"),
            (Removed, "    let name = old_name();"),
            (Added, "    let x = 2;"),
            (Added, "    let name = new_name();"),
            (Added, "    println!(\"{name}\");"),
            (Context, "}"),
        ];
        assert_eq!(
            marked(&lines),
            [
                vec![],
                vec!["1"],
                vec!["old_name"],
                vec!["2"],
                vec!["new_name"],
                vec![],
                vec![],
            ],
            "the third added line has no pair"
        );
    }

    #[test]
    fn one_char_between_two_changes_joins_them() {
        use LineKind::{Added, Removed};
        let lines = [
            (Removed, "call(alpha beta, gamma)"),
            (Added, "call(delta epsilon, gamma)"),
        ];
        assert_eq!(marked(&lines), [vec!["alpha beta"], vec!["delta epsilon"]]);
        let lines = [(Removed, "f(a, b)"), (Added, "f(a, b, c)")];
        assert_eq!(marked(&lines), [vec![], vec![", c"]], "an insert only");
    }

    #[test]
    fn a_hunk_header_or_a_context_line_ends_a_block_and_a_note_does_not() {
        use LineKind::{Added, Context, Hunk, Note, Removed};
        let lines = [
            (Removed, "let a = 1;"),
            (Hunk, "@@ -9 +9 @@"),
            (Added, "let a = 2;"),
            (Removed, "let b = 1;"),
            (Context, "x"),
            (Added, "let b = 2;"),
            (Removed, "let c = 1;"),
            (Note, "\\ No newline at end of file"),
            (Added, "let c = 2;"),
        ];
        assert_eq!(
            marked(&lines),
            [vec![], vec![], vec![], vec![], vec![], vec!["1"], vec!["2"],]
        );
    }

    #[test]
    fn different_lines_long_lines_and_uneven_blocks_get_no_marks() {
        use LineKind::{Added, Removed};
        let different = [
            (Removed, "use std::fmt;"),
            (Added, "    return Ok(value * 2)"),
        ];
        assert_eq!(marked(&different), [Vec::<String>::new(), vec![]]);
        let long_old = format!("let s = \"{}\";", "a".repeat(MAX_LINE));
        let long_new = format!("let s = \"{}\";", "b".repeat(MAX_LINE));
        let long = [(Removed, long_old.as_str()), (Added, long_new.as_str())];
        assert_eq!(marked(&long), [Vec::<String>::new(), vec![]]);
        let uneven = [
            (Removed, "let x = 1;"),
            (Added, "let x = 2;"),
            (Added, "let y = 3;"),
            (Added, "let z = 4;"),
            (Added, "let w = 5;"),
        ];
        assert!(
            marked(&uneven).iter().all(Vec::is_empty),
            "1 removed and 4 added lines"
        );
        let three = &uneven[..4];
        assert_eq!(marked(three)[1], ["2"], "1 removed and 3 added lines");
    }

    #[test]
    fn words_are_runs_of_letters_or_spaces_and_single_signs() {
        let text = "a_b  = f(x1);é";
        let parts: Vec<&str> = words(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(
            parts,
            ["a_b", "  ", "=", " ", "f", "(", "x1", ")", ";", "é"]
        );
    }
}
