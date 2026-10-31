//! The hit map: the targets that `ui` drew, and the cells of each one.
//! `run` keeps the map of the last frame and finds the target under the
//! mouse with it.

use ratatui::layout::{Position, Rect};

use crate::bus::Hit;

/// The regions of the drawn targets, in draw order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HitMap {
    regions: Vec<(Rect, Hit)>,
}

impl HitMap {
    /// Adds the region of a target. A later region covers the regions
    /// before it.
    pub(crate) fn push(&mut self, rect: Rect, hit: Hit) {
        if !rect.is_empty() {
            self.regions.push((rect, hit));
        }
    }

    /// The target at the cell `x`, `y`: the last region that contains
    /// the cell. `None` when no region contains it.
    pub(crate) fn at(&self, x: u16, y: u16) -> Option<Hit> {
        self.regions
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(Position { x, y }))
            .map(|&(_, hit)| hit)
    }

    /// Removes every region.
    pub(crate) fn clear(&mut self) {
        self.regions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_region_wins() {
        let mut hits = HitMap::default();
        hits.push(Rect::new(0, 0, 10, 10), Hit::Row(0));
        hits.push(Rect::new(5, 5, 10, 10), Hit::Inside);
        hits.push(Rect::new(3, 3, 0, 4), Hit::Item(1));
        assert_eq!(hits.at(1, 1), Some(Hit::Row(0)));
        assert_eq!(hits.at(5, 5), Some(Hit::Inside));
        assert_eq!(
            hits.at(3, 3),
            Some(Hit::Row(0)),
            "an empty rect hits nothing"
        );
        assert_eq!(hits.at(20, 20), None);
        hits.clear();
        assert_eq!(hits.at(1, 1), None);
    }
}
