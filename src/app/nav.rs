//! Cursor and scroll rules for lists: which rows the cursor can stand
//! on, the moves of the navigation keys, and the scroll that keeps the
//! cursor visible.

use crate::app::models::{Row, RowItem};

/// A move of the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Motion {
    Down,
    Up,
    Top,
    Bottom,
    /// Half a page down.
    PageDown,
    /// Half a page up.
    PageUp,
}

/// True for the rows that the cursor can stand on: pull requests and
/// local changes, not headers or separators.
pub(crate) fn selectable(row: &Row) -> bool {
    matches!(row.item, RowItem::Pr(_) | RowItem::Local { .. })
}

/// True when two items name the same thing. A pull request stays the same
/// when its head moves.
pub(crate) fn same_item(a: &RowItem, b: &RowItem) -> bool {
    match (a, b) {
        (RowItem::Pr(a), RowItem::Pr(b)) => a.key == b.key,
        (a, b) => a == b,
    }
}

/// The selectable row nearest to `index`: `index` itself, else the next
/// one below, else the next one above. `None` when no row is selectable.
fn nearest(rows: &[Row], index: usize) -> Option<usize> {
    let index = index.min(rows.len().checked_sub(1)?);
    (index..rows.len())
        .find(|&i| selectable(&rows[i]))
        .or_else(|| (0..index).rev().find(|&i| selectable(&rows[i])))
}

/// The scroll offset that keeps `cursor` in a window of `height` rows,
/// as close to `scroll` as possible, and never past the end.
pub(crate) fn keep_visible(len: usize, cursor: usize, scroll: usize, height: usize) -> usize {
    let height = height.max(1);
    let max = len.saturating_sub(height);
    let scroll = scroll.min(max);
    if cursor < scroll {
        cursor
    } else if cursor >= scroll + height {
        cursor + 1 - height
    } else {
        scroll
    }
}

/// The cursor and the scroll of a list from the remembered selection: the
/// row of `selected` when it is still there, else the selectable row
/// nearest to the old `cursor`.
pub(crate) fn place(
    rows: &[Row],
    selected: Option<&RowItem>,
    cursor: usize,
    scroll: usize,
    height: usize,
) -> (usize, usize) {
    let found = selected.and_then(|item| rows.iter().position(|row| same_item(&row.item, item)));
    let cursor = found.or_else(|| nearest(rows, cursor)).unwrap_or(0);
    (cursor, keep_visible(rows.len(), cursor, scroll, height))
}

/// The cursor after a move from `cursor`, on a selectable row. `None`
/// when no row is selectable.
pub(crate) fn step(rows: &[Row], cursor: usize, motion: Motion, height: usize) -> Option<usize> {
    let half = (height / 2).max(1);
    let next_down = |from: usize| (from..rows.len()).find(|&i| selectable(&rows[i]));
    let next_up = |from: usize| {
        (0..=from.min(rows.len().checked_sub(1)?))
            .rev()
            .find(|&i| selectable(&rows[i]))
    };
    let target = match motion {
        Motion::Down => next_down(cursor + 1),
        Motion::Up => cursor.checked_sub(1).and_then(next_up),
        Motion::Top => next_down(0),
        Motion::Bottom => next_up(rows.len().checked_sub(1)?),
        Motion::PageDown => next_down(cursor + half).or_else(|| next_up(rows.len() - 1)),
        Motion::PageUp => next_up(cursor.saturating_sub(half)).or_else(|| next_down(0)),
    };
    target.or_else(|| nearest(rows, cursor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::RowStyle;
    use crate::bus::PrRef;
    use crate::domain::keys::{PrKey, RepoKey};

    fn pr_item(number: u64, oid: &str) -> RowItem {
        RowItem::Pr(PrRef {
            key: PrKey {
                repo: RepoKey {
                    owner: "o".into(),
                    name: "r".into(),
                },
                number,
            },
            node_id: String::new(),
            head_oid: oid.into(),
            base_ref: "main".into(),
        })
    }

    fn row(item: RowItem) -> Row {
        Row {
            item,
            cells: Vec::new(),
            depth: 0,
            glyph: None,
            style: RowStyle::Normal,
        }
    }

    /// A header, then pull requests 1 to `n`, and a header after #2.
    fn rows(n: u64) -> Vec<Row> {
        let header = || {
            row(RowItem::StackHeader {
                stack_id: "S".into(),
            })
        };
        let mut rows = vec![header()];
        for number in 1..=n {
            rows.push(row(pr_item(number, "a")));
            if number == 2 {
                rows.push(header());
            }
        }
        rows
    }

    #[test]
    fn step_skips_headers() {
        // Rows: 0 header, 1 #1, 2 #2, 3 header, 4 #3, 5 #4, 6 #5.
        let rows = rows(5);
        let cases = [
            (1, Motion::Down, 2),
            (2, Motion::Down, 4),
            (6, Motion::Down, 6),
            (4, Motion::Up, 2),
            (1, Motion::Up, 1),
            (5, Motion::Top, 1),
            (1, Motion::Bottom, 6),
            (1, Motion::PageDown, 4),
            (5, Motion::PageDown, 6),
            (6, Motion::PageUp, 2),
            (2, Motion::PageUp, 1),
        ];
        for (from, motion, to) in cases {
            assert_eq!(step(&rows, from, motion, 6), Some(to), "{from} {motion:?}");
        }
        assert_eq!(step(&rows[..1], 0, Motion::Down, 6), None);
        assert_eq!(step(&[], 0, Motion::Down, 6), None);
    }

    #[test]
    fn keep_visible_cases() {
        let cases = [
            ((20, 0, 0, 5), 0),
            ((20, 4, 0, 5), 0),
            ((20, 5, 0, 5), 1),
            ((20, 2, 4, 5), 2),
            ((20, 19, 0, 5), 15),
            ((3, 1, 10, 5), 0),
            ((0, 0, 3, 5), 0),
        ];
        for ((len, cursor, scroll, height), expected) in cases {
            assert_eq!(
                keep_visible(len, cursor, scroll, height),
                expected,
                "{len} {cursor} {scroll} {height}"
            );
        }
    }

    #[test]
    fn place_follows_the_selected_item_after_a_change() {
        let before = rows(5);
        let selected = before[4].item.clone();
        let mut after = rows(5);
        after.insert(1, row(pr_item(9, "a")));
        if let RowItem::Pr(pr) = &mut after[5].item {
            pr.head_oid = "moved".into();
        }
        assert_eq!(place(&after, Some(&selected), 4, 0, 10), (5, 0));
        let gone = pr_item(77, "a");
        assert_eq!(
            place(&after, Some(&gone), 4, 0, 10),
            (5, 0),
            "a missing item keeps the old index, on the next selectable row"
        );
        assert_eq!(place(&after, None, 99, 0, 3), (7, 5));
        assert_eq!(place(&[], None, 3, 2, 3), (0, 0));
    }
}
