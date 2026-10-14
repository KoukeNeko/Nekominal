//! Pane layout tree for one tab. Every update returns a new tree.

pub type PaneId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// `Row` places panes side by side, `Column` stacks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split(Box<Split>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    pub axis: Axis,
    /// Share of the space given to `first`.
    pub ratio: f64,
    pub first: Node,
    pub second: Node,
}

/// Fractions of the tab area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub const UNIT: Rect = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };

#[derive(Clone, Debug, PartialEq)]
pub struct Splitter {
    /// Child indices (0 = first, 1 = second) from the root to the split.
    pub path: Vec<u8>,
    pub axis: Axis,
    /// Area occupied by the split's two children.
    pub rect: Rect,
    pub ratio: f64,
}

pub const MIN_RATIO: f64 = 0.1;
pub const MAX_RATIO: f64 = 1.0 - MIN_RATIO;
const EPSILON: f64 = 1e-6;

fn clamp_ratio(ratio: f64) -> f64 {
    ratio.clamp(MIN_RATIO, MAX_RATIO)
}

fn child_rects(split: &Split, rect: Rect) -> (Rect, Rect) {
    match split.axis {
        Axis::Row => {
            let w = rect.w * split.ratio;
            (Rect { w, ..rect }, Rect { x: rect.x + w, w: rect.w - w, ..rect })
        }
        Axis::Column => {
            let h = rect.h * split.ratio;
            (Rect { h, ..rect }, Rect { y: rect.y + h, h: rect.h - h, ..rect })
        }
    }
}

impl Node {
    pub fn pane_ids(&self) -> Vec<PaneId> {
        match self {
            Node::Leaf(id) => vec![*id],
            Node::Split(split) => {
                let mut ids = split.first.pane_ids();
                ids.extend(split.second.pane_ids());
                ids
            }
        }
    }

    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Leaf(leaf) => *leaf == id,
            Node::Split(split) => split.first.contains(id) || split.second.contains(id),
        }
    }

    /// Replaces `target` with a split holding `target` and the new pane (placed second).
    pub fn split_pane(&self, target: PaneId, axis: Axis, new_id: PaneId) -> Node {
        match self {
            Node::Leaf(id) if *id == target => Node::Split(Box::new(Split {
                axis,
                ratio: 0.5,
                first: Node::Leaf(*id),
                second: Node::Leaf(new_id),
            })),
            Node::Leaf(_) => self.clone(),
            Node::Split(split) => Node::Split(Box::new(Split {
                first: split.first.split_pane(target, axis, new_id),
                second: split.second.split_pane(target, axis, new_id),
                ..**split
            })),
        }
    }

    /// Removes a pane; its sibling takes the freed space. Returns `None` when the tree empties.
    pub fn remove_pane(&self, id: PaneId) -> Option<Node> {
        match self {
            Node::Leaf(leaf) => (*leaf != id).then(|| self.clone()),
            Node::Split(split) => match (split.first.remove_pane(id), split.second.remove_pane(id)) {
                (None, second) => second,
                (first, None) => first,
                (Some(first), Some(second)) => Some(Node::Split(Box::new(Split { first, second, ..**split }))),
            },
        }
    }

    /// Pane rectangles in tree order.
    pub fn layout_rects(&self, rect: Rect) -> Vec<(PaneId, Rect)> {
        match self {
            Node::Leaf(id) => vec![(*id, rect)],
            Node::Split(split) => {
                let (first, second) = child_rects(split, rect);
                let mut rects = split.first.layout_rects(first);
                rects.extend(split.second.layout_rects(second));
                rects
            }
        }
    }

    pub fn rect_of(&self, id: PaneId, rect: Rect) -> Option<Rect> {
        self.layout_rects(rect).into_iter().find(|(pane, _)| *pane == id).map(|(_, r)| r)
    }

    pub fn splitters(&self, rect: Rect) -> Vec<Splitter> {
        let mut found = Vec::new();
        self.collect_splitters(rect, &mut Vec::new(), &mut found);
        found
    }

    fn collect_splitters(&self, rect: Rect, path: &mut Vec<u8>, found: &mut Vec<Splitter>) {
        let Node::Split(split) = self else { return };
        found.push(Splitter { path: path.clone(), axis: split.axis, rect, ratio: split.ratio });
        let (first, second) = child_rects(split, rect);
        path.push(0);
        split.first.collect_splitters(first, path, found);
        path.pop();
        path.push(1);
        split.second.collect_splitters(second, path, found);
        path.pop();
    }

    /// Sets the ratio of the split at `path` (clamped so no pane disappears).
    pub fn set_ratio_at(&self, path: &[u8], ratio: f64) -> Node {
        let Node::Split(split) = self else { return self.clone() };
        match path.split_first() {
            None => Node::Split(Box::new(Split { ratio: clamp_ratio(ratio), ..(**split).clone() })),
            Some((0, rest)) => Node::Split(Box::new(Split { first: split.first.set_ratio_at(rest, ratio), ..(**split).clone() })),
            Some((_, rest)) => Node::Split(Box::new(Split { second: split.second.set_ratio_at(rest, ratio), ..(**split).clone() })),
        }
    }

    /// Nearest pane in a direction, preferring the one that overlaps most along the other axis.
    pub fn neighbor(&self, from: PaneId, direction: Direction) -> Option<PaneId> {
        let rects = self.layout_rects(UNIT);
        let origin = rects.iter().find(|(id, _)| *id == from)?.1;
        let horizontal = matches!(direction, Direction::Left | Direction::Right);

        let mut best: Option<(PaneId, f64, f64)> = None;
        for (id, rect) in rects {
            if id == from {
                continue;
            }
            let gap = match direction {
                Direction::Right => rect.x - (origin.x + origin.w),
                Direction::Left => origin.x - (rect.x + rect.w),
                Direction::Down => rect.y - (origin.y + origin.h),
                Direction::Up => origin.y - (rect.y + rect.h),
            };
            if gap < -EPSILON {
                continue;
            }
            let overlap = if horizontal {
                (origin.y + origin.h).min(rect.y + rect.h) - origin.y.max(rect.y)
            } else {
                (origin.x + origin.w).min(rect.x + rect.w) - origin.x.max(rect.x)
            };
            if overlap <= EPSILON {
                continue;
            }
            let better = match best {
                None => true,
                Some((_, best_gap, best_overlap)) => {
                    gap < best_gap - EPSILON || ((gap - best_gap).abs() <= EPSILON && overlap > best_overlap)
                }
            };
            if better {
                best = Some((id, gap, overlap));
            }
        }
        best.map(|(id, _, _)| id)
    }

    /// Moves the border of `id` that faces `direction` by `delta` (a fraction of the tab area),
    /// using the nearest ancestor split along that axis that has such a border.
    pub fn resize_pane(&self, id: PaneId, direction: Direction, delta: f64) -> Node {
        let axis = if matches!(direction, Direction::Left | Direction::Right) { Axis::Row } else { Axis::Column };
        let grows = matches!(direction, Direction::Right | Direction::Down);

        // Ancestors from the root down: (path, split, side the pane is on, area of the split).
        let mut chain: Vec<(Vec<u8>, &Split, u8, Rect)> = Vec::new();
        fn find<'a>(node: &'a Node, id: PaneId, area: Rect, path: &mut Vec<u8>, chain: &mut Vec<(Vec<u8>, &'a Split, u8, Rect)>) -> bool {
            match node {
                Node::Leaf(leaf) => *leaf == id,
                Node::Split(split) => {
                    let (first, second) = child_rects(split, area);
                    for (side, child, child_area) in [(0u8, &split.first, first), (1u8, &split.second, second)] {
                        path.push(side);
                        let found = find(child, id, child_area, path, chain);
                        path.pop();
                        if found {
                            chain.push((path.clone(), split, side, area));
                            return true;
                        }
                    }
                    false
                }
            }
        }
        if !find(self, id, UNIT, &mut Vec::new(), &mut chain) {
            return self.clone();
        }

        // `chain` is innermost-first. The border to move must be shared with a sibling in `direction`.
        for (path, split, side, area) in chain {
            if split.axis != axis {
                continue;
            }
            let facing = if grows { side == 0 } else { side == 1 };
            if !facing {
                continue;
            }
            let span = if axis == Axis::Row { area.w } else { area.h };
            let change = (if grows { delta } else { -delta }) / span;
            return self.set_ratio_at(&path, split.ratio + change);
        }
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: PaneId) -> Node {
        Node::Leaf(id)
    }

    /// ```text
    /// +-----+-----+
    /// |     |  2  |
    /// |  1  +-----+
    /// |     |  3  |
    /// +-----+-----+
    /// ```
    fn three_panes() -> Node {
        leaf(1).split_pane(1, Axis::Row, 2).split_pane(2, Axis::Column, 3)
    }

    fn rect(root: &Node, id: PaneId) -> Rect {
        root.rect_of(id, UNIT).unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn split_places_the_new_pane_after_the_target() {
        assert_eq!(three_panes().pane_ids(), vec![1, 2, 3]);
    }

    #[test]
    fn removing_a_pane_gives_its_space_to_the_sibling() {
        let tree = three_panes().remove_pane(3).unwrap();
        assert_eq!(rect(&tree, 1), Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 });
        assert_eq!(rect(&tree, 2), Rect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 });
    }

    #[test]
    fn removing_the_last_pane_empties_the_tree() {
        assert_eq!(leaf(1).remove_pane(1), None);
    }

    #[test]
    fn unknown_ids_leave_the_tree_unchanged() {
        let tree = three_panes();
        assert_eq!(tree.remove_pane(99), Some(tree.clone()));
        assert_eq!(tree.split_pane(99, Axis::Row, 4), tree);
    }

    #[test]
    fn ratios_divide_the_area() {
        let tree = leaf(1).split_pane(1, Axis::Column, 2).set_ratio_at(&[], 0.25);
        assert_eq!(rect(&tree, 1), Rect { x: 0.0, y: 0.0, w: 1.0, h: 0.25 });
        assert_eq!(rect(&tree, 2), Rect { x: 0.0, y: 0.25, w: 1.0, h: 0.75 });
    }

    #[test]
    fn ratios_are_clamped_so_no_pane_disappears() {
        let tree = leaf(1).split_pane(1, Axis::Row, 2).set_ratio_at(&[], 0.99);
        assert!(close(rect(&tree, 2).w, 0.1));
        let tree = tree.set_ratio_at(&[], -3.0);
        assert!(close(rect(&tree, 1).w, 0.1));
    }

    #[test]
    fn splitters_report_one_entry_per_split_with_its_path() {
        let found = three_panes().splitters(UNIT);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].path.clone(), found[0].axis), (vec![], Axis::Row));
        assert_eq!((found[1].path.clone(), found[1].axis), (vec![1], Axis::Column));
        assert_eq!(found[1].rect, Rect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 });
    }

    #[test]
    fn neighbors_move_across_and_within_splits() {
        let tree = three_panes();
        assert_eq!(tree.neighbor(1, Direction::Right), Some(2));
        assert_eq!(tree.neighbor(3, Direction::Left), Some(1));
        assert_eq!(tree.neighbor(2, Direction::Down), Some(3));
        assert_eq!(tree.neighbor(3, Direction::Up), Some(2));
    }

    #[test]
    fn neighbors_stop_at_the_edge() {
        let tree = three_panes();
        assert_eq!(tree.neighbor(1, Direction::Left), None);
        assert_eq!(tree.neighbor(2, Direction::Up), None);
        assert_eq!(tree.neighbor(3, Direction::Right), None);
    }

    #[test]
    fn neighbor_prefers_the_pane_with_the_larger_overlap() {
        let lopsided = three_panes().set_ratio_at(&[1], 0.8); // pane 2 tall, pane 3 short
        assert_eq!(lopsided.neighbor(1, Direction::Right), Some(2));
    }

    #[test]
    fn resize_moves_the_border_that_faces_the_direction() {
        let tree = leaf(1).split_pane(1, Axis::Row, 2);
        assert!(close(rect(&tree.resize_pane(1, Direction::Right, 0.1), 1).w, 0.6));
        assert!(close(rect(&tree.resize_pane(2, Direction::Left, 0.1), 1).w, 0.4));
    }

    #[test]
    fn resize_ignores_a_direction_with_no_border() {
        let tree = leaf(1).split_pane(1, Axis::Row, 2);
        assert_eq!(tree.resize_pane(1, Direction::Left, 0.1), tree);
        assert_eq!(tree.resize_pane(1, Direction::Down, 0.1), tree);
    }

    #[test]
    fn resize_falls_back_to_an_outer_split() {
        // Pane 3's left border is the root split, not the inner column split.
        let moved = three_panes().resize_pane(3, Direction::Left, 0.1);
        assert!(close(rect(&moved, 1).w, 0.4));
    }

    #[test]
    fn contains_finds_nested_panes() {
        let tree = three_panes();
        assert!(tree.contains(3));
        assert!(!tree.contains(4));
    }
}
