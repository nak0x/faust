//! Splits: a binary tree of panes, each owning its own tabs.
//!
//! Faust's window is one pane until you split it. A split is a node with two
//! children and a ratio; the leaves are panes, and a pane is a tab strip plus
//! whatever its active tab shows. Splitting *moves* the current tab into the
//! new pane rather than duplicating it, so a document is never open twice and
//! the pane you came from falls back to the tab on its left.
//!
//! Geometry lives here too: [`Panes::layout`] turns the tree into rectangles,
//! and remembers them so that [`Panes::focus_dir`] can pick the neighbour a
//! vim motion means, the way vim picks it — by where the panes actually are.

use egui::{pos2, vec2, Rect};

use crate::document::Document;

/// Thickness of the draggable gap between two panes, in points.
pub const SPLIT_GAP: f32 = 6.0;
/// A split never collapses either of its sides completely.
const MIN_RATIO: f32 = 0.08;
/// Neither side of a split shrinks below this, in points, however deeply the
/// window is divided — a pane narrower than its own tab strip is not a pane.
const MIN_SIDE: f32 = 80.0;

pub type PaneId = u64;

/// Which way a split's two children sit next to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Side by side, divided by a vertical line — vim's `:vsplit`.
    Row,
    /// Stacked, divided by a horizontal line — vim's `:split`.
    Column,
}

/// A direction the focus can travel in, from `ctrl+h/j/k/l`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    /// The axis a split has to be on for this direction to cross it.
    const fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Row,
            Self::Up | Self::Down => Axis::Column,
        }
    }
}

/// One open file in a pane.
pub struct Pane {
    pub id: PaneId,
    pub tabs: Vec<Document>,
    /// Index into `tabs`; meaningless while `tabs` is empty.
    pub active: usize,
}

impl Pane {
    const fn new(id: PaneId) -> Self {
        Self {
            id,
            tabs: Vec::new(),
            active: 0,
        }
    }

    pub fn doc(&self) -> Option<&Document> {
        self.tabs.get(self.active)
    }

    pub fn doc_mut(&mut self) -> Option<&mut Document> {
        self.tabs.get_mut(self.active)
    }

    /// Removes a tab, leaving the one to its left active.
    ///
    /// That is what makes `ctrl+w v` read the way it does: the current tab
    /// moves into the new pane, and the pane it left shows its neighbour.
    pub fn take(&mut self, index: usize) -> Option<Document> {
        if index >= self.tabs.len() {
            return None;
        }
        let doc = self.tabs.remove(index);
        self.active = match self.active {
            active if active > index => active - 1,
            active if active == index => index.saturating_sub(1),
            active => active,
        };
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        Some(doc)
    }

    fn put(&mut self, doc: Document, at: usize) -> usize {
        let at = at.min(self.tabs.len());
        self.tabs.insert(at, doc);
        self.active = at;
        at
    }
}

/// The layout tree. Splits are interior nodes; panes are the leaves.
enum Node {
    Leaf(PaneId),
    Split {
        axis: Axis,
        /// Share of the parent given to `first`, `0.0..=1.0`.
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

impl Node {
    fn holds(&self, id: PaneId) -> bool {
        match self {
            Self::Leaf(leaf) => *leaf == id,
            Self::Split { first, second, .. } => first.holds(id) || second.holds(id),
        }
    }

    fn first_leaf(&self) -> PaneId {
        match self {
            Self::Leaf(id) => *id,
            Self::Split { first, .. } => first.first_leaf(),
        }
    }

    fn leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Self::Leaf(id) => out.push(*id),
            Self::Split { first, second, .. } => {
                first.leaves(out);
                second.leaves(out);
            }
        }
    }

    /// Replaces `target`'s leaf with a split of `target` and `new`.
    fn split(&mut self, target: PaneId, axis: Axis, new: PaneId) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                *self = Self::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Leaf(target)),
                    second: Box::new(Self::Leaf(new)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split(target, axis, new) || second.split(target, axis, new)
            }
        }
    }

    /// The subtree that would take over if `id` went away.
    fn sibling(&self, id: PaneId) -> Option<&Self> {
        let Self::Split { first, second, .. } = self else {
            return None;
        };
        if first.holds(id) {
            return first.sibling(id).or(Some(second));
        }
        if second.holds(id) {
            return second.sibling(id).or(Some(first));
        }
        None
    }

    /// Drops `id`'s leaf, collapsing the split it was half of.
    fn without(self, id: PaneId) -> Option<Self> {
        match self {
            Self::Leaf(leaf) => (leaf != id).then_some(Self::Leaf(leaf)),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => match (first.without(id), second.without(id)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }

    fn nth_split(&mut self, index: usize, next: &mut usize) -> Option<&mut f32> {
        let Self::Split {
            ratio,
            first,
            second,
            ..
        } = self
        else {
            return None;
        };
        let this = *next;
        *next += 1;
        if this == index {
            return Some(ratio);
        }
        first
            .nth_split(index, next)
            .or_else(|| second.nth_split(index, next))
    }

    fn equalize(&mut self) {
        if let Self::Split {
            ratio,
            first,
            second,
            ..
        } = self
        {
            *ratio = 0.5;
            first.equalize();
            second.equalize();
        }
    }
}

/// Where one pane was painted this frame.
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub id: PaneId,
    pub rect: Rect,
}

/// The draggable gap between two panes.
#[derive(Debug, Clone, Copy)]
pub struct Bar {
    /// Position of the split in pre-order, the handle [`Panes::set_ratio`] takes.
    pub index: usize,
    pub axis: Axis,
    /// The gap itself.
    pub rect: Rect,
    /// The area both children share, which the ratio is measured against.
    pub parent: Rect,
}

pub struct Panes {
    root: Node,
    /// Pane storage, in no particular order; the tree gives the order.
    panes: Vec<Pane>,
    focus: PaneId,
    next_id: PaneId,
    /// Where each pane was last painted, for directional focus.
    slots: Vec<Slot>,
}

impl Default for Panes {
    fn default() -> Self {
        Self::new()
    }
}

impl Panes {
    pub fn new() -> Self {
        Self {
            root: Node::Leaf(0),
            panes: vec![Pane::new(0)],
            focus: 0,
            next_id: 1,
            slots: Vec::new(),
        }
    }

    // ----------------------------------------------------------------- panes

    pub const fn focus(&self) -> PaneId {
        self.focus
    }

    pub fn set_focus(&mut self, id: PaneId) {
        if self.panes.iter().any(|pane| pane.id == id) {
            self.focus = id;
        }
    }

    pub fn len(&self) -> usize {
        self.panes.len()
    }

    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.iter().find(|pane| pane.id == id)
    }

    pub fn pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.panes.iter_mut().find(|pane| pane.id == id)
    }

    /// The focused pane, which always exists.
    pub fn active(&self) -> &Pane {
        self.pane(self.focus).unwrap_or(&self.panes[0])
    }

    pub fn active_mut(&mut self) -> &mut Pane {
        let focus = self.focus;
        match self.panes.iter().position(|pane| pane.id == focus) {
            Some(index) => &mut self.panes[index],
            None => &mut self.panes[0],
        }
    }

    /// Every pane, in no particular order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Pane> {
        self.panes.iter_mut()
    }

    /// Every open document, in no particular order.
    pub fn docs_mut(&mut self) -> impl Iterator<Item = &mut Document> {
        self.panes.iter_mut().flat_map(|pane| pane.tabs.iter_mut())
    }

    /// Panes in layout order, left to right and top to bottom.
    pub fn order(&self) -> Vec<PaneId> {
        let mut out = Vec::with_capacity(self.panes.len());
        self.root.leaves(&mut out);
        out
    }

    // --------------------------------------------------------------- editing

    /// Splits the focused pane, moving its current tab into the new one.
    ///
    /// The new pane takes the right half (`Row`) or the bottom half (`Column`)
    /// and the focus; the old pane keeps the rest of its tabs.
    pub fn split(&mut self, axis: Axis) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;

        let mut pane = Pane::new(id);
        let source = self.active_mut();
        if let Some(doc) = source.take(source.active) {
            pane.tabs.push(doc);
        }
        self.panes.push(pane);
        self.root.split(self.focus, axis, id);
        self.focus = id;
        id
    }

    /// Closes the focused pane, handing its tabs to the pane that takes over.
    ///
    /// Like vim, closing a window never closes what was open in it.
    pub fn close(&mut self) {
        if self.panes.len() < 2 {
            return;
        }
        let heir = self
            .root
            .sibling(self.focus)
            .map_or(self.focus, Node::first_leaf);
        if heir == self.focus {
            return;
        }
        let going = self.focus;
        let tabs = self
            .pane_mut(going)
            .map(|pane| std::mem::take(&mut pane.tabs))
            .unwrap_or_default();

        self.panes.retain(|pane| pane.id != going);
        if let Some(root) = std::mem::replace(&mut self.root, Node::Leaf(heir)).without(going) {
            self.root = root;
        }
        self.focus = heir;
        self.adopt(heir, tabs);
    }

    /// Closes every other pane, gathering their tabs into the focused one.
    pub fn only(&mut self) {
        if self.panes.len() < 2 {
            return;
        }
        let keep = self.focus;
        let mut tabs = Vec::new();
        for id in self.order() {
            if id == keep {
                continue;
            }
            if let Some(pane) = self.pane_mut(id) {
                tabs.append(&mut pane.tabs);
            }
        }
        self.panes.retain(|pane| pane.id == keep);
        self.root = Node::Leaf(keep);
        self.adopt(keep, tabs);
    }

    /// Appends rescued tabs without disturbing the pane's own current tab.
    fn adopt(&mut self, id: PaneId, tabs: Vec<Document>) {
        let Some(pane) = self.pane_mut(id) else {
            return;
        };
        let was_empty = pane.tabs.is_empty();
        pane.tabs.extend(tabs);
        if was_empty {
            pane.active = 0;
        }
        pane.active = pane.active.min(pane.tabs.len().saturating_sub(1));
    }

    /// Moves a tab to `at` in another pane, or to a new position in its own.
    ///
    /// `at` is an index into the target pane as it looks *now*, which is what
    /// a drop between two tabs means; a move inside one pane accounts for the
    /// hole the tab leaves behind.
    pub fn move_tab(&mut self, from: PaneId, index: usize, to: PaneId, at: usize) {
        if from == to {
            let Some(pane) = self.pane_mut(from) else {
                return;
            };
            if index >= pane.tabs.len() {
                return;
            }
            let doc = pane.tabs.remove(index);
            let at = if at > index { at - 1 } else { at };
            pane.put(doc, at);
        } else {
            let Some(doc) = self.pane_mut(from).and_then(|pane| pane.take(index)) else {
                return;
            };
            let Some(target) = self.pane_mut(to) else {
                // The target vanished mid-drag; put it back rather than drop it.
                if let Some(pane) = self.pane_mut(from) {
                    pane.put(doc, index);
                }
                return;
            };
            target.put(doc, at);
        }
        self.set_focus(to);
    }

    // -------------------------------------------------------------- geometry

    /// Divides `area` between the panes, remembering the result.
    pub fn layout(&mut self, area: Rect) -> (Vec<Slot>, Vec<Bar>) {
        let mut slots = Vec::with_capacity(self.panes.len());
        let mut bars = Vec::new();
        let mut next = 0;
        walk(&self.root, area, &mut slots, &mut bars, &mut next);
        self.slots.clone_from(&slots);
        (slots, bars)
    }

    /// Where a pane was painted, from the last [`Self::layout`].
    pub fn rect(&self, id: PaneId) -> Option<Rect> {
        self.slots
            .iter()
            .find(|slot| slot.id == id)
            .map(|slot| slot.rect)
    }

    pub fn set_ratio(&mut self, index: usize, ratio: f32) {
        let mut next = 0;
        if let Some(slot) = self.root.nth_split(index, &mut next) {
            *slot = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
        }
    }

    pub fn equalize(&mut self) {
        self.root.equalize();
    }

    // ------------------------------------------------------------ navigation

    /// Focuses the nearest pane in `dir`, or stays put if there is none.
    ///
    /// Panes that share an edge with the current one win; among those, the
    /// closest. Falling back to any pane on that side keeps the motion useful
    /// in layouts where nothing lines up.
    pub fn focus_dir(&mut self, dir: Dir) {
        let Some(from) = self.rect(self.focus) else {
            return;
        };
        let ahead = |rect: &Rect| match dir {
            Dir::Left => from.left() - rect.right(),
            Dir::Right => rect.left() - from.right(),
            Dir::Up => from.top() - rect.bottom(),
            Dir::Down => rect.top() - from.bottom(),
        };
        let overlaps = |rect: &Rect| match dir.axis() {
            Axis::Row => rect.bottom() > from.top() && rect.top() < from.bottom(),
            Axis::Column => rect.right() > from.left() && rect.left() < from.right(),
        };

        let mut best: Option<(bool, f32, PaneId)> = None;
        for slot in &self.slots {
            if slot.id == self.focus {
                continue;
            }
            let gap = ahead(&slot.rect);
            if gap < -1.0 {
                continue;
            }
            let key = (!overlaps(&slot.rect), gap.max(0.0), slot.id);
            if best.is_none_or(|current| key < current) {
                best = Some(key);
            }
        }
        if let Some((_, _, id)) = best {
            self.focus = id;
        }
    }

    /// Focuses the next pane in layout order, wrapping around.
    pub fn cycle(&mut self, forward: bool) {
        let order = self.order();
        if order.len() < 2 {
            return;
        }
        let Some(at) = order.iter().position(|id| *id == self.focus) else {
            return;
        };
        let step = if forward { 1 } else { order.len() - 1 };
        self.focus = order[(at + step) % order.len()];
    }
}

/// Turns the tree into rectangles, numbering splits in pre-order.
fn walk(node: &Node, rect: Rect, slots: &mut Vec<Slot>, bars: &mut Vec<Bar>, next: &mut usize) {
    match node {
        Node::Leaf(id) => slots.push(Slot { id: *id, rect }),
        Node::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let index = *next;
            *next += 1;
            let (a, bar, b) = divide(rect, *axis, *ratio);
            bars.push(Bar {
                index,
                axis: *axis,
                rect: bar,
                parent: rect,
            });
            walk(first, a, slots, bars, next);
            walk(second, b, slots, bars, next);
        }
    }
}

/// Splits a rectangle into two halves and the gap between them.
fn divide(rect: Rect, axis: Axis, ratio: f32) -> (Rect, Rect, Rect) {
    match axis {
        Axis::Row => {
            let usable = (rect.width() - SPLIT_GAP).max(0.0);
            let cut = keep_both(rect.left() + usable * ratio, rect.left(), rect.right());
            (
                Rect::from_min_max(rect.min, pos2(cut, rect.bottom())),
                Rect::from_min_size(pos2(cut, rect.top()), vec2(SPLIT_GAP, rect.height())),
                Rect::from_min_max(pos2(cut + SPLIT_GAP, rect.top()), rect.max),
            )
        }
        Axis::Column => {
            let usable = (rect.height() - SPLIT_GAP).max(0.0);
            let cut = keep_both(rect.top() + usable * ratio, rect.top(), rect.bottom());
            (
                Rect::from_min_max(rect.min, pos2(rect.right(), cut)),
                Rect::from_min_size(pos2(rect.left(), cut), vec2(rect.width(), SPLIT_GAP)),
                Rect::from_min_max(pos2(rect.left(), cut + SPLIT_GAP), rect.max),
            )
        }
    }
}

/// Holds a split's cut far enough from both ends to leave two usable panes,
/// falling back to the middle when there is not even room for that.
fn keep_both(cut: f32, start: f32, end: f32) -> f32 {
    let (low, high) = (start + MIN_SIDE, end - SPLIT_GAP - MIN_SIDE);
    if low <= high {
        cut.clamp(low, high)
    } else {
        (start + end - SPLIT_GAP) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A document for a file that does not exist: no I/O, no parsing.
    fn doc(name: &str) -> Document {
        Document::open(PathBuf::from(format!("/nonexistent/{name}.md")))
    }

    fn with_tabs(names: &[&str]) -> Panes {
        let mut panes = Panes::new();
        for name in names {
            panes.active_mut().tabs.push(doc(name));
        }
        panes.active_mut().active = names.len().saturating_sub(1);
        panes
    }

    fn titles(panes: &Panes, id: PaneId) -> Vec<String> {
        panes
            .pane(id)
            .map(|pane| pane.tabs.iter().map(|tab| tab.title.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn a_split_moves_the_current_tab_and_leaves_the_one_on_its_left() {
        let mut panes = with_tabs(&["a", "b", "c"]);
        let old = panes.focus();
        let new = panes.split(Axis::Row);

        assert_eq!(panes.focus(), new);
        assert_eq!(titles(&panes, new), ["c"]);
        assert_eq!(titles(&panes, old), ["a", "b"]);
        assert_eq!(panes.pane(old).unwrap().active, 1);
    }

    #[test]
    fn splitting_a_lone_tab_leaves_an_empty_pane_behind() {
        let mut panes = with_tabs(&["a"]);
        let old = panes.focus();
        let new = panes.split(Axis::Column);

        assert_eq!(titles(&panes, new), ["a"]);
        assert!(titles(&panes, old).is_empty());
        assert_eq!(panes.len(), 2);
    }

    #[test]
    fn closing_a_pane_hands_its_tabs_to_the_survivor() {
        let mut panes = with_tabs(&["a", "b"]);
        let old = panes.focus();
        panes.split(Axis::Row);
        panes.close();

        assert_eq!(panes.focus(), old);
        assert_eq!(panes.len(), 1);
        assert_eq!(titles(&panes, old), ["a", "b"]);
    }

    #[test]
    fn only_gathers_every_tab_into_the_focused_pane() {
        let mut panes = with_tabs(&["a", "b", "c"]);
        panes.split(Axis::Row);
        panes.split(Axis::Column);
        assert_eq!(panes.len(), 3);

        let keep = panes.focus();
        panes.only();
        assert_eq!(panes.len(), 1);
        assert_eq!(titles(&panes, keep).len(), 3);
    }

    #[test]
    fn the_last_pane_cannot_be_closed() {
        let mut panes = with_tabs(&["a"]);
        panes.close();
        assert_eq!(panes.len(), 1);
    }

    #[test]
    fn dragging_a_tab_within_a_pane_reorders_it() {
        let mut panes = with_tabs(&["a", "b", "c"]);
        let id = panes.focus();
        panes.move_tab(id, 0, id, 3);
        assert_eq!(titles(&panes, id), ["b", "c", "a"]);
        assert_eq!(panes.pane(id).unwrap().active, 2);

        // A drop either side of a tab's own place changes nothing.
        panes.move_tab(id, 1, id, 2);
        assert_eq!(titles(&panes, id), ["b", "c", "a"]);
    }

    #[test]
    fn dragging_a_tab_to_another_pane_moves_the_focus_with_it() {
        let mut panes = with_tabs(&["a", "b", "c"]);
        let old = panes.focus();
        let new = panes.split(Axis::Row);
        panes.set_focus(old);

        panes.move_tab(old, 0, new, 0);
        assert_eq!(titles(&panes, new), ["a", "c"]);
        assert_eq!(titles(&panes, old), ["b"]);
        assert_eq!(panes.focus(), new);
        assert_eq!(panes.pane(new).unwrap().active, 0);
    }

    #[test]
    fn motions_cross_to_the_pane_that_shares_an_edge() {
        // ┌───┬───┐
        // │ a │ b │
        // │   ├───┤
        // │   │ c │
        // └───┴───┘
        let mut panes = with_tabs(&["one"]);
        let a = panes.focus();
        let b = panes.split(Axis::Row);
        let c = panes.split(Axis::Column);
        panes.layout(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 800.0)));

        panes.set_focus(c);
        panes.focus_dir(Dir::Left);
        assert_eq!(panes.focus(), a);
        panes.focus_dir(Dir::Right);
        assert_eq!(panes.focus(), b);
        panes.focus_dir(Dir::Down);
        assert_eq!(panes.focus(), c);
        panes.focus_dir(Dir::Right);
        assert_eq!(panes.focus(), c, "nothing to the right of the last column");
    }

    #[test]
    fn a_ratio_stays_off_the_edges() {
        let mut panes = with_tabs(&["a"]);
        panes.split(Axis::Row);
        panes.set_ratio(0, 5.0);
        let (slots, _) = panes.layout(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 800.0)));
        assert!(slots.iter().all(|slot| slot.rect.width() >= MIN_SIDE));
    }

    #[test]
    fn nested_splits_stay_usable() {
        let mut panes = with_tabs(&["a"]);
        for _ in 0..6 {
            panes.split(Axis::Row);
            panes.split(Axis::Column);
        }
        let (slots, _) = panes.layout(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0)));
        assert_eq!(slots.len(), 13);
        assert!(slots
            .iter()
            .all(|slot| slot.rect.width() > 0.0 && slot.rect.height() > 0.0));
    }
}
