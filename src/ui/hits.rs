//! The hit map: the targets that `ui` drew, and the cells of each one.
//! `run` keeps the map of the last frame and finds the target under the
//! mouse with it. The map also keeps the text areas, where a mouse drag
//! selects text.

use ratatui::layout::{Position, Rect};

use crate::bus::Hit;

/// The regions of the drawn targets and the drawn text areas, in draw
/// order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HitMap {
    regions: Vec<(Rect, Hit)>,
    text: Vec<Rect>,
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

    /// The region of a target: the last region with `hit`. `None` when no
    /// region has it.
    pub(crate) fn region(&self, hit: Hit) -> Option<Rect> {
        self.regions
            .iter()
            .rev()
            .find(|&&(_, h)| h == hit)
            .map(|&(rect, _)| rect)
    }

    /// Adds a text area. A later area covers the areas before it.
    pub(crate) fn push_text(&mut self, rect: Rect) {
        if !rect.is_empty() {
            self.text.push(rect);
        }
    }

    /// The text area at the cell `x`, `y`: the last area that contains
    /// the cell. `None` when no area contains it.
    pub(crate) fn text_at(&self, x: u16, y: u16) -> Option<Rect> {
        self.text
            .iter()
            .rev()
            .find(|rect| rect.contains(Position { x, y }))
            .copied()
    }

    /// Removes every region and every text area.
    pub(crate) fn clear(&mut self) {
        self.regions.clear();
        self.text.clear();
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

    #[test]
    fn the_last_text_area_wins() {
        let mut hits = HitMap::default();
        let below = Rect::new(0, 0, 10, 10);
        let above = Rect::new(5, 5, 10, 10);
        hits.push_text(below);
        hits.push_text(above);
        hits.push_text(Rect::new(1, 1, 0, 3));
        assert_eq!(hits.text_at(1, 1), Some(below), "an empty rect is no area");
        assert_eq!(hits.text_at(6, 6), Some(above));
        assert_eq!(hits.text_at(20, 20), None);
        assert_eq!(hits.at(6, 6), None, "a text area is no hit");
        hits.clear();
        assert_eq!(hits.text_at(6, 6), None);
    }
}
