//! The plural of the count texts: `1 PR`, `0 PRs`, `3 PRs`.

use std::fmt::Display;

/// `one` for a count of 1, else `many`. Use it when other words come
/// between the count and the noun: `3 team PRs`.
pub(crate) fn noun<'a, N>(n: N, one: &'a str, many: &'a str) -> &'a str
where
    N: PartialEq + From<u8> + Copy,
{
    if n == N::from(1) { one } else { many }
}

/// The count and its noun: `1 PR`, `0 PRs`, `3 PRs`.
pub(crate) fn count<N>(n: N, one: &str, many: &str) -> String
where
    N: Display + PartialEq + From<u8> + Copy,
{
    format!("{n} {}", noun(n, one, many))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_is_singular_and_every_other_count_is_plural() {
        assert_eq!(count(0_usize, "PR", "PRs"), "0 PRs");
        assert_eq!(count(1_usize, "PR", "PRs"), "1 PR");
        assert_eq!(count(2_u32, "PR", "PRs"), "2 PRs");
        assert_eq!(count(12_u64, "file", "files"), "12 files");
        assert_eq!(noun(1_u64, "comment", "comments"), "comment");
        assert_eq!(noun(3_usize, "comment", "comments"), "comments");
    }
}
