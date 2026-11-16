//! The lines of a patch, and the roles of the code tokens in them.

/// The kind of a patch line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineKind {
    /// A hunk header, as `@@ -1,3 +1,4 @@`.
    Hunk,
    Added,
    Removed,
    Context,
    /// `\ No newline at end of file`.
    Note,
}

/// The kind of each line of `patch` and its line number: the new number
/// for an added or a context line, the old number for a removed line,
/// and `None` for a hunk header or a note.
pub(crate) fn numbered(patch: &str) -> Vec<(LineKind, Option<u32>, &str)> {
    let (mut old, mut new) = (0_u32, 0_u32);
    patch
        .lines()
        .map(|text| {
            let (kind, number) = if text.starts_with("@@") {
                if let Some((o, n)) = hunk_starts(text) {
                    (old, new) = (o, n);
                }
                (LineKind::Hunk, None)
            } else if text.starts_with('+') {
                new = new.saturating_add(1);
                (LineKind::Added, Some(new - 1))
            } else if text.starts_with('-') {
                old = old.saturating_add(1);
                (LineKind::Removed, Some(old - 1))
            } else if text.starts_with('\\') {
                (LineKind::Note, None)
            } else {
                old = old.saturating_add(1);
                new = new.saturating_add(1);
                (LineKind::Context, Some(new - 1))
            };
            (kind, number, text)
        })
        .collect()
}

/// The code of a patch line: the line without its `+`, `-` or space.
pub(crate) fn code(line: &str) -> &str {
    line.get(1..).unwrap_or_default()
}

/// The first old and new line numbers of a hunk header, as in
/// `@@ -12,5 +14,7 @@ fn main`.
pub(crate) fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.split_whitespace().skip(1);
    let start = |part: Option<&str>, sign: char| -> Option<u32> {
        let range = part?.strip_prefix(sign)?;
        range.split(',').next()?.parse().ok()
    };
    let old = start(parts.next(), '-')?;
    let new = start(parts.next(), '+')?;
    Some((old, new))
}

/// The role of a code token. The palette gives each role a color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Syntax {
    /// A token without a role, in the text color.
    Plain,
    Comment,
    String,
    /// A number or a constant of the language, as `true`.
    Number,
    /// Another constant, as `MAX_LEN`.
    Constant,
    Keyword,
    Operator,
    Function,
    Type,
    Namespace,
    Variable,
    Parameter,
    Punctuation,
}

/// The tokens of one code line, each with its role. Two tokens next to
/// each other have different roles.
pub(crate) type Tokens = Vec<(Syntax, String)>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_numbers_count_from_each_hunk_header() {
        let text = "@@ -8,2 +9,3 @@ fn main\n a\n-b\n+c\n+d\n@@ -98 +100,2 @@\n e\n+f";
        let numbers: Vec<(LineKind, Option<u32>)> =
            numbered(text).into_iter().map(|(k, n, _)| (k, n)).collect();
        assert_eq!(
            numbers,
            [
                (LineKind::Hunk, None),
                (LineKind::Context, Some(9)),
                (LineKind::Removed, Some(9)),
                (LineKind::Added, Some(10)),
                (LineKind::Added, Some(11)),
                (LineKind::Hunk, None),
                (LineKind::Context, Some(100)),
                (LineKind::Added, Some(101)),
            ]
        );
        assert_eq!(hunk_starts("@@ -98 +100,2 @@"), Some((98, 100)));
        assert_eq!(hunk_starts("@@ broken @@"), None);
        assert_eq!(code("+let x = 1;"), "let x = 1;");
        assert_eq!(code(""), "");
    }
}
