use std::collections::HashSet;

use serde::{Deserialize, Serialize};

pub const MAX_PANES: usize = 16;
const MAX_DEPTH: usize = 16;
const MIN_RATIO: u16 = 100;
const MAX_RATIO: u16 = 900;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug)]
pub enum PaneAction {
    Split(Axis),
    Focus(Direction),
    Zoom,
    Close,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaneNode {
    Leaf(u64),
    Split { axis: Axis, ratio: u16, first: Box<PaneNode>, second: Box<PaneNode> },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl PaneRect {
    pub fn contains(&self, x: usize, y: usize) -> bool {
        x >= self.x && y >= self.y && x - self.x < self.width && y - self.y < self.height
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneDivider {
    pub path: Vec<bool>,
    pub axis: Axis,
    pub rect: PaneRect,
    pub area: PaneRect,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneLayout {
    pub panes: Vec<(u64, PaneRect)>,
    pub dividers: Vec<PaneDivider>,
}

impl PaneLayout {
    pub fn hit_divider(&self, x: usize, y: usize) -> Option<&PaneDivider> {
        self.dividers.iter().find(|divider| divider.rect.contains(x, y))
    }
}

impl PaneNode {
    pub fn leaf_ids(&self) -> Vec<u64> {
        let mut ids = Vec::new();
        self.collect_ids(&mut ids);
        ids
    }

    fn collect_ids(&self, ids: &mut Vec<u64>) {
        match self {
            Self::Leaf(id) => ids.push(*id),
            Self::Split { first, second, .. } => {
                first.collect_ids(ids);
                second.collect_ids(ids);
            },
        }
    }

    pub fn validate(&self) -> bool {
        self.validate_node(0, &mut HashSet::new())
    }

    fn validate_node(&self, depth: usize, ids: &mut HashSet<u64>) -> bool {
        if depth > MAX_DEPTH {
            return false;
        }

        match self {
            Self::Leaf(id) => ids.insert(*id) && ids.len() <= MAX_PANES,
            Self::Split { ratio, first, second, .. } => {
                (MIN_RATIO..=MAX_RATIO).contains(ratio)
                    && first.validate_node(depth + 1, ids)
                    && second.validate_node(depth + 1, ids)
            },
        }
    }

    pub fn split(&mut self, target: u64, new: u64, axis: Axis) -> bool {
        if !self.validate() {
            return false;
        }
        let ids = self.leaf_ids();
        if ids.len() >= MAX_PANES || ids.contains(&new) {
            return false;
        }

        self.split_node(target, new, axis, 0)
    }

    fn split_node(&mut self, target: u64, new: u64, axis: Axis, depth: usize) -> bool {
        match self {
            Self::Leaf(id) if *id == target && depth < MAX_DEPTH => {
                *self = Self::Split {
                    axis,
                    ratio: 500,
                    first: Box::new(Self::Leaf(target)),
                    second: Box::new(Self::Leaf(new)),
                };
                true
            },
            Self::Split { first, second, .. } => {
                first.split_node(target, new, axis, depth + 1)
                    || second.split_node(target, new, axis, depth + 1)
            },
            Self::Leaf(_) => false,
        }
    }

    pub fn remove(&mut self, target: u64) -> bool {
        let Self::Split { first, second, .. } = self else { return false };

        if matches!(first.as_ref(), Self::Leaf(id) if *id == target) {
            *self = std::mem::replace(second.as_mut(), Self::Leaf(target));
            true
        } else if matches!(second.as_ref(), Self::Leaf(id) if *id == target) {
            *self = std::mem::replace(first.as_mut(), Self::Leaf(target));
            true
        } else {
            first.remove(target) || second.remove(target)
        }
    }

    pub fn remap_ids(&self, mut remap: impl FnMut(u64) -> u64) -> Self {
        self.map_node(&mut remap)
    }

    fn map_node(&self, remap: &mut impl FnMut(u64) -> u64) -> Self {
        match self {
            Self::Leaf(id) => Self::Leaf(remap(*id)),
            Self::Split { axis, ratio, first, second } => Self::Split {
                axis: *axis,
                ratio: *ratio,
                first: Box::new(first.map_node(remap)),
                second: Box::new(second.map_node(remap)),
            },
        }
    }

    pub fn layout(&self, columns: usize, lines: usize) -> PaneLayout {
        let mut layout = PaneLayout::default();
        let area = PaneRect { x: 0, y: 0, width: columns, height: lines };
        self.layout_node(area, &mut Vec::new(), &mut layout);
        layout
    }

    fn minimum_size(&self) -> (usize, usize) {
        match self {
            Self::Leaf(_) => (2, 1),
            Self::Split { axis, first, second, .. } => {
                let (first_width, first_height) = first.minimum_size();
                let (second_width, second_height) = second.minimum_size();
                match axis {
                    Axis::Horizontal => (
                        first_width.max(second_width),
                        first_height.saturating_add(second_height).saturating_add(1),
                    ),
                    Axis::Vertical => (
                        first_width.saturating_add(second_width).saturating_add(1),
                        first_height.max(second_height),
                    ),
                }
            },
        }
    }

    fn split_area(&self, area: PaneRect) -> Option<(PaneRect, PaneRect, PaneRect)> {
        let Self::Split { axis, ratio, first, second } = self else { return None };
        let (first_width, first_height) = first.minimum_size();
        let (second_width, second_height) = second.minimum_size();
        let (extent, first_min, second_min) = match axis {
            Axis::Horizontal => (area.height, first_height, second_height),
            Axis::Vertical => (area.width, first_width, second_width),
        };
        let separator = usize::from(extent >= 3);
        let available = extent - separator;
        let preferred = ((available as u128 * (*ratio).clamp(MIN_RATIO, MAX_RATIO) as u128 + 500)
            / 1000) as usize;
        let first_extent = if available >= first_min.saturating_add(second_min) {
            preferred.clamp(first_min, available - second_min)
        } else if available >= 2 {
            preferred.clamp(1, available - 1)
        } else {
            available
        };
        let second_extent = available - first_extent;

        Some(match axis {
            Axis::Horizontal => (
                PaneRect { height: first_extent, ..area },
                PaneRect { y: area.y + first_extent, height: separator, ..area },
                PaneRect { y: area.y + first_extent + separator, height: second_extent, ..area },
            ),
            Axis::Vertical => (
                PaneRect { width: first_extent, ..area },
                PaneRect { x: area.x + first_extent, width: separator, ..area },
                PaneRect { x: area.x + first_extent + separator, width: second_extent, ..area },
            ),
        })
    }

    fn layout_node(&self, area: PaneRect, path: &mut Vec<bool>, layout: &mut PaneLayout) {
        match self {
            Self::Leaf(id) => layout.panes.push((*id, area)),
            Self::Split { axis, first, second, .. } => {
                let (first_area, rect, second_area) = self.split_area(area).unwrap();
                if rect.width != 0 && rect.height != 0 {
                    layout.dividers.push(PaneDivider {
                        path: path.clone(),
                        axis: *axis,
                        rect,
                        area,
                    });
                }
                path.push(false);
                first.layout_node(first_area, path, layout);
                *path.last_mut().unwrap() = true;
                second.layout_node(second_area, path, layout);
                path.pop();
            },
        }
    }

    pub fn resize_divider(
        &mut self,
        path: &[bool],
        x: usize,
        y: usize,
        columns: usize,
        lines: usize,
    ) -> bool {
        let layout = self.layout(columns, lines);
        let Some(divider) = layout.dividers.iter().find(|divider| divider.path == path) else {
            return false;
        };
        let (offset, extent) = match divider.axis {
            Axis::Horizontal => (y.saturating_sub(divider.area.y), divider.area.height),
            Axis::Vertical => (x.saturating_sub(divider.area.x), divider.area.width),
        };
        let available = extent - 1;
        let next_ratio = ((offset.min(available) as u128 * 1000 + available as u128 / 2)
            / available as u128) as u16;
        let next_ratio = next_ratio.clamp(MIN_RATIO, MAX_RATIO);
        let mut node = self;
        for second_child in path {
            let Self::Split { first, second, .. } = node else { return false };
            node = if *second_child { second } else { first };
        }
        let Self::Split { ratio, .. } = node else { return false };
        let changed = *ratio != next_ratio;
        *ratio = next_ratio;
        changed
    }

    pub fn navigate(
        &self,
        id: u64,
        direction: Direction,
        columns: usize,
        lines: usize,
    ) -> Option<u64> {
        let layout = self.layout(columns, lines);
        let (_, current) = layout.panes.iter().find(|(pane_id, _)| *pane_id == id)?;
        if current.width == 0 || current.height == 0 {
            return None;
        }

        layout
            .panes
            .iter()
            .filter(|(pane_id, rect)| *pane_id != id && rect.width != 0 && rect.height != 0)
            .filter_map(|(pane_id, rect)| {
                let (near, far, start, length, target_start, target_length) = match direction {
                    Direction::Left if rect.x + rect.width <= current.x => (
                        rect.x + rect.width,
                        current.x,
                        current.y,
                        current.height,
                        rect.y,
                        rect.height,
                    ),
                    Direction::Right if rect.x >= current.x + current.width => (
                        current.x + current.width,
                        rect.x,
                        current.y,
                        current.height,
                        rect.y,
                        rect.height,
                    ),
                    Direction::Up if rect.y + rect.height <= current.y => (
                        rect.y + rect.height,
                        current.y,
                        current.x,
                        current.width,
                        rect.x,
                        rect.width,
                    ),
                    Direction::Down if rect.y >= current.y + current.height => (
                        current.y + current.height,
                        rect.y,
                        current.x,
                        current.width,
                        rect.x,
                        rect.width,
                    ),
                    _ => return None,
                };
                let overlap = start < target_start + target_length && target_start < start + length;
                let center = start as u128 * 2 + length as u128;
                let target_center = target_start as u128 * 2 + target_length as u128;
                Some((*pane_id, (!overlap, far - near, center.abs_diff(target_center))))
            })
            .min_by_key(|(_, distance)| *distance)
            .map(|(pane_id, _)| pane_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> PaneNode {
        let mut root = PaneNode::Leaf(10);
        assert!(root.split(10, 20, Axis::Vertical));
        assert!(root.split(10, 30, Axis::Horizontal));
        assert!(root.split(20, 40, Axis::Horizontal));
        root
    }

    #[test]
    fn split_and_remove_preserve_identity_and_collapse_only_the_target_branch() {
        let mut root = grid();
        assert_eq!(root.leaf_ids(), vec![10, 30, 20, 40]);
        assert!(!root.split(99, 50, Axis::Horizontal));
        assert!(!root.split(10, 20, Axis::Horizontal));
        assert!(!root.remove(99));
        assert!(root.remove(10));
        assert_eq!(root.leaf_ids(), vec![30, 20, 40]);
        assert!(root.remove(40));
        assert_eq!(root.leaf_ids(), vec![30, 20]);
        assert!(root.remove(30));
        assert_eq!(root, PaneNode::Leaf(20));
        assert!(!root.remove(20));
        assert!(root.validate());
    }

    #[test]
    fn validation_rejects_duplicates_bad_ratios_and_excessive_panes() {
        let mut root = PaneNode::Leaf(0);
        for id in 1..MAX_PANES as u64 {
            assert!(root.split(id - 1, id, Axis::Vertical));
        }
        assert!(root.validate());
        let original = root.clone();
        assert!(!root.split(15, 16, Axis::Vertical));
        assert_eq!(root, original);
        let too_many = PaneNode::Split {
            axis: Axis::Horizontal,
            ratio: 500,
            first: Box::new(root),
            second: Box::new(PaneNode::Leaf(16)),
        };
        assert!(!too_many.validate());
        assert!(!grid().remap_ids(|_| 1).validate());
        for ratio in [0, 99, 901, u16::MAX] {
            let mut invalid = PaneNode::Split {
                axis: Axis::Vertical,
                ratio,
                first: Box::new(PaneNode::Leaf(0)),
                second: Box::new(PaneNode::Leaf(1)),
            };
            assert!(!invalid.validate());
            assert!(!invalid.split(0, 2, Axis::Horizontal));
        }
    }

    #[test]
    fn saved_tree_round_trip_and_remapping_preserve_structure_and_ratios() {
        let mut root = grid();
        assert!(root.resize_divider(&[], 70, 0, 101, 41));
        assert!(root.resize_divider(&[false], 0, 10, 101, 41));
        let encoded = serde_json::to_string(&root).unwrap();
        let decoded: PaneNode = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, root);
        let mapped = root.remap_ids(|id| id + 1000);
        assert_eq!(mapped.leaf_ids(), vec![1010, 1030, 1020, 1040]);
        assert_eq!(mapped.remap_ids(|id| id - 1000), root);
        assert!(mapped.validate());
    }

    #[test]
    fn nested_dividers_use_their_containing_area_for_resize_and_hit_testing() {
        let mut root = grid();
        let layout = root.layout(101, 41);
        assert_eq!(layout.panes[0], (10, PaneRect { x: 0, y: 0, width: 50, height: 20 }));
        assert_eq!(layout.panes[1], (30, PaneRect { x: 0, y: 21, width: 50, height: 20 }));
        assert_eq!(layout.panes[2], (20, PaneRect { x: 51, y: 0, width: 50, height: 20 }));
        assert_eq!(layout.panes[3], (40, PaneRect { x: 51, y: 21, width: 50, height: 20 }));
        assert_eq!(layout.hit_divider(50, 20).unwrap().path, Vec::<bool>::new());
        assert_eq!(layout.hit_divider(49, 20).unwrap().path, vec![false]);
        assert_eq!(layout.hit_divider(51, 20).unwrap().path, vec![true]);
        assert!(layout.hit_divider(0, 0).is_none());
        assert!(layout.hit_divider(101, 41).is_none());
        assert!(root.resize_divider(&[true], 99, 30, 101, 41));
        let layout = root.layout(101, 41);
        assert_eq!(layout.panes[0].1.height, 20);
        assert_eq!(layout.panes[2].1.height, 30);
        assert_eq!(layout.panes[3].1.y, 31);
        assert!(!root.resize_divider(&[true, false], 0, 0, 101, 41));
        assert!(root.resize_divider(&[], usize::MAX, 0, 101, 41));
        assert_eq!(root.layout(101, 41).panes[0].1.width, 90);
        assert!(!root.resize_divider(&[], usize::MAX, 0, 101, 41));
        assert!(root.resize_divider(&[], 0, 0, 101, 41));
        assert_eq!(root.layout(101, 41).panes[0].1.width, 10);
    }

    #[test]
    fn all_small_sizes_partition_without_overlap_or_out_of_bounds_cells() {
        let mut root = grid();
        assert!(root.split(40, 50, Axis::Vertical));
        assert!(root.split(50, 60, Axis::Horizontal));
        for columns in 0..80 {
            for lines in 0..40 {
                let layout = root.layout(columns, lines);
                assert_eq!(layout.panes.len(), 6);
                let mut occupied = vec![false; columns * lines];
                let rects = layout
                    .panes
                    .iter()
                    .map(|(_, rect)| rect)
                    .chain(layout.dividers.iter().map(|divider| &divider.rect));
                for rect in rects {
                    assert!(rect.x + rect.width <= columns);
                    assert!(rect.y + rect.height <= lines);
                    for y in rect.y..rect.y + rect.height {
                        for x in rect.x..rect.x + rect.width {
                            assert!(!occupied[y * columns + x]);
                            occupied[y * columns + x] = true;
                        }
                    }
                }
                assert!(occupied.into_iter().all(|cell| cell));
                if columns >= 8 && lines >= 7 {
                    assert!(
                        layout.panes.iter().all(|(_, rect)| rect.width >= 2 && rect.height >= 1)
                    );
                }
            }
        }
        assert!(!root.resize_divider(&[], 0, 0, 1, 1));
    }

    #[test]
    fn directional_navigation_follows_visible_geometry_without_wrapping() {
        let root = grid();
        assert_eq!(root.navigate(10, Direction::Right, 101, 41), Some(20));
        assert_eq!(root.navigate(10, Direction::Down, 101, 41), Some(30));
        assert_eq!(root.navigate(40, Direction::Left, 101, 41), Some(30));
        assert_eq!(root.navigate(40, Direction::Up, 101, 41), Some(20));
        assert_eq!(root.navigate(10, Direction::Left, 101, 41), None);
        assert_eq!(root.navigate(10, Direction::Up, 101, 41), None);
        assert_eq!(root.navigate(99, Direction::Down, 101, 41), None);
        assert_eq!(root.navigate(10, Direction::Down, 0, 0), None);
        assert_eq!(root.navigate(40, Direction::Left, 1, 1), None);
    }

    #[test]
    fn navigation_prefers_overlap_over_a_nearer_diagonal_pane() {
        let mut root = PaneNode::Leaf(0);
        assert!(root.split(0, 1, Axis::Vertical));
        assert!(root.split(0, 2, Axis::Horizontal));
        assert!(root.split(1, 3, Axis::Horizontal));
        assert!(root.resize_divider(&[false], 0, 8, 81, 41));
        assert!(root.resize_divider(&[true], 0, 32, 81, 41));
        assert_eq!(root.navigate(0, Direction::Right, 81, 41), Some(1));
        assert_eq!(root.navigate(3, Direction::Left, 81, 41), Some(2));
    }

    #[test]
    fn maximum_dimensions_do_not_overflow_geometry_or_direction_ranking() {
        let root = grid();
        let layout = root.layout(usize::MAX, usize::MAX);
        assert_eq!(layout.panes.len(), 4);
        for (_, rect) in layout.panes {
            assert!(rect.x.checked_add(rect.width).is_some());
            assert!(rect.y.checked_add(rect.height).is_some());
        }
        assert_eq!(root.navigate(10, Direction::Right, usize::MAX, usize::MAX), Some(20));
        assert!(
            !PaneRect { x: usize::MAX, y: usize::MAX, width: 0, height: 0 }
                .contains(usize::MAX, usize::MAX)
        );
    }
}
