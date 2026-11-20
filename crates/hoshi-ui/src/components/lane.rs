use gpui_kit::*;

/// What something floating at the right of the window's lanes, such as a card, takes from
/// them: the lanes move left only as far as `right` needs, keeping `gutter` from the left
/// edge, and narrow past that.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LaneRoom {
    pub right: Pixels,
    pub gutter: Pixels,
}

impl Global for LaneRoom {}

impl LaneRoom {
    /// Sets the room the lanes leave; tells nothing when it is the same. Whether it changed.
    pub fn set(room: LaneRoom, cx: &mut App) -> bool {
        let changed = cx.try_global::<LaneRoom>() != Some(&room);
        if changed {
            cx.set_global(room);
        }
        changed
    }
}

/// The centered column a conversation reads in, as wide as `--chat-lane` on the web.
#[derive(IntoElement)]
pub struct Lane {
    children: Vec<AnyElement>,
}

impl Lane {
    pub const WIDTH: f32 = 768.0;

    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }
}

impl Default for Lane {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for Lane {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Lane {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let room = cx.try_global::<LaneRoom>().copied().unwrap_or_default();
        let side = |min: Pixels| {
            div()
                .flex_grow_1()
                .flex_shrink_0()
                .flex_basis(px(0.))
                .min_w(min)
        };
        div()
            .w_full()
            .flex()
            .px_6()
            .child(side(room.gutter))
            .child(
                div()
                    .relative()
                    .flex_grow_0()
                    .flex_shrink(1.)
                    .flex_basis(px(Self::WIDTH))
                    .min_w_0()
                    .children(self.children),
            )
            .child(side(room.right))
    }
}
