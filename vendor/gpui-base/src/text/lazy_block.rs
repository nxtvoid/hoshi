//! The top-level blocks of a document that is not its own scroller, built only while they show.
//!
//! A long document inside a scrolling parent, such as a long message in a chat transcript, would
//! otherwise lay out and paint every block on every frame. Each block is drawn whole the first
//! time and the room it takes is kept; from then on a block outside the content mask only takes
//! up that room, and is built in prepaint, as `gpui::list` builds its items, once it comes into
//! view.

use std::{cell::RefCell, rc::Rc};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Style, Window, px, relative, size,
};

/// What the heights were measured for: a change to any of them invalidates every height.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct HeightsKey {
    /// The document's blocks, by the address of their shared list.
    pub(crate) blocks: usize,
    /// The length of the document's source, in case a new list of blocks took an old address.
    pub(crate) source: usize,
    /// The text style, by the address of its shared value.
    pub(crate) style: usize,
    pub(crate) rem_size: Pixels,
    pub(crate) font_size: Pixels,
}

/// The room each top-level block of a document takes: from the bottom of the block before it,
/// or the top of the document, to its own bottom. That takes in a margin that collapses above a
/// block, which its own bounds leave out.
#[derive(Default, Debug)]
pub(crate) struct BlockHeights {
    key: Option<HeightsKey>,
    width: Option<Pixels>,
    heights: Vec<Option<Pixels>>,
    /// This frame: where the document starts and where each block drawn so far ends.
    top: Pixels,
    bottoms: Vec<Pixels>,
}

impl BlockHeights {
    /// Readies the heights for a document of `count` blocks under `key`, forgetting them when
    /// anything they depend on changed.
    pub(crate) fn sync(&mut self, key: HeightsKey, count: usize) {
        if self.key != Some(key) || self.heights.len() != count {
            self.key = Some(key);
            self.heights = vec![None; count];
        }
        self.bottoms.resize(count, px(0.));
    }

    /// Starts a frame of the document at `bounds`; `false` when the heights were measured at
    /// another width, which forgets them.
    pub(crate) fn start(&mut self, bounds: Bounds<Pixels>) -> bool {
        self.top = bounds.top();
        match self.width.replace(bounds.size.width) {
            Some(before) if before != bounds.size.width => {
                self.heights.iter_mut().for_each(|height| *height = None);
                false
            }
            _ => true,
        }
    }

    /// The block at `y`, from the top of the document, when every block above it was measured.
    pub(crate) fn block_at(&self, y: Pixels) -> Option<usize> {
        let mut bottom = px(0.);
        for (ix, height) in self.heights.iter().enumerate() {
            bottom += (*height)?;
            if y < bottom {
                return Some(ix);
            }
        }
        self.heights.len().checked_sub(1)
    }

    /// Whether every block was measured, so blocks out of view can stand in at their height.
    pub(crate) fn measured(&self) -> bool {
        !self.heights.is_empty() && self.heights.iter().all(Option::is_some)
    }

    /// The height the block at `ix` stands in at out of view. The first and the last block are
    /// always built: a margin of theirs can collapse into the document's own edge.
    fn stand_in(&self, ix: usize) -> Option<Pixels> {
        if ix == 0 || ix + 1 >= self.heights.len() || !self.measured() {
            return None;
        }
        self.heights[ix]
    }

    /// Where the block before `ix` ended this frame.
    fn above(&self, ix: usize) -> Pixels {
        match ix {
            0 => self.top,
            ix => self.bottoms[ix - 1],
        }
    }

    /// Records the room the block at `ix` took and where it ended; `false` when that room differs
    /// from what it was laid out with.
    fn record(&mut self, ix: usize, height: Pixels, bottom: Pixels) -> bool {
        if let Some(end) = self.bottoms.get_mut(ix) {
            *end = bottom;
        }
        match self.heights.get_mut(ix) {
            Some(slot) => slot.replace(height).is_none_or(|before| before == height),
            None => true,
        }
    }
}

type Build = Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement>;

/// One top-level block: drawn whole while its room is unknown, and otherwise only once it shows
/// inside the content mask.
pub(crate) struct LazyBlock {
    ix: usize,
    heights: Rc<RefCell<BlockHeights>>,
    build: Option<Build>,
    element: Option<AnyElement>,
}

impl LazyBlock {
    pub(crate) fn new(
        ix: usize,
        heights: Rc<RefCell<BlockHeights>>,
        build: impl FnOnce(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            ix,
            heights,
            build: Some(Box::new(build)),
            element: None,
        }
    }
}

impl IntoElement for LazyBlock {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for LazyBlock {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let stand_in = self.heights.borrow().stand_in(self.ix);
        if let Some(height) = stand_in {
            let mut style = Style::default();
            style.size.width = relative(1.).into();
            style.size.height = height.into();
            return (window.request_layout(style, None, cx), ());
        }
        let build = self.build.take().expect("built once");
        let mut element = build(window, cx);
        let layout_id = element.request_layout(window, cx);
        self.element = Some(element);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(element) = self.element.as_mut() {
            element.prepaint(window, cx);
            let mut heights = self.heights.borrow_mut();
            let room = bounds.bottom() - heights.above(self.ix);
            if !heights.record(self.ix, room, bounds.bottom()) {
                window.refresh();
            }
            return;
        }
        if !bounds.intersects(&window.content_mask().bounds) {
            self.heights
                .borrow_mut()
                .record(self.ix, bounds.size.height, bounds.bottom());
            return;
        }
        let Some(build) = self.build.take() else {
            return;
        };
        let mut element = build(window, cx);
        let drawn = element.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::MinContent,
            ),
            window,
            cx,
        );
        element.prepaint_at(bounds.origin, window, cx);
        let held = self
            .heights
            .borrow_mut()
            .record(self.ix, drawn.height, bounds.bottom());
        if !held {
            window.refresh();
        }
        self.element = Some(element);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(element) = self.element.as_mut() {
            element.paint(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockHeights, HeightsKey};
    use gpui::{Bounds, point, px, size};

    fn key(blocks: usize) -> HeightsKey {
        HeightsKey {
            blocks,
            source: 10,
            style: 0,
            rem_size: px(16.),
            font_size: px(15.),
        }
    }

    fn at(width: f32) -> Bounds<gpui::Pixels> {
        Bounds::new(point(px(0.), px(100.)), size(px(width), px(500.)))
    }

    /// Three blocks drawn at `width`, the middle one 4px below the first, as a collapsed margin
    /// puts it.
    fn draw(heights: &mut BlockHeights, width: f32) -> bool {
        let started = heights.start(at(width));
        let first = heights.record(0, px(20.), px(120.));
        let room = px(154.) - heights.above(1);
        let middle = heights.record(1, room, px(154.));
        let last = heights.record(2, px(10.), px(164.));
        started && first && middle && last
    }

    #[test]
    fn a_block_takes_the_room_from_the_one_above_it() {
        let mut heights = BlockHeights::default();
        heights.sync(key(1), 3);
        assert!(draw(&mut heights, 100.));
        assert!(heights.measured());
        assert_eq!(heights.stand_in(1), Some(px(34.)));
        assert_eq!(heights.stand_in(0), None, "the first is always built");
        assert_eq!(heights.stand_in(2), None, "the last is always built");
        assert_eq!(heights.block_at(px(25.)), Some(1));
    }

    #[test]
    fn heights_are_forgotten_when_the_document_or_the_width_changes() {
        let mut heights = BlockHeights::default();
        heights.sync(key(1), 3);
        draw(&mut heights, 100.);
        heights.sync(key(1), 3);
        assert!(heights.measured());
        assert!(!heights.start(at(90.)));
        assert!(!heights.measured());

        draw(&mut heights, 90.);
        heights.sync(key(2), 3);
        assert!(!heights.measured());
    }

    #[test]
    fn a_block_that_grew_reports_it() {
        let mut heights = BlockHeights::default();
        heights.sync(key(1), 3);
        draw(&mut heights, 100.);
        assert!(heights.record(1, px(34.), px(154.)));
        assert!(!heights.record(1, px(38.), px(158.)));
    }
}
