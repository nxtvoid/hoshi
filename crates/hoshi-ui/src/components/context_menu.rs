use crate::components::{Align, Popover, PopoverMenu, Side};
use gpui_kit::*;

/// A menu opened by a right click, with its corner at `position` in the window. Render it while
/// it is open; `on_close` runs when it asks to close.
pub fn context_menu(
    id: impl Into<ElementId>,
    position: Point<Pixels>,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    menu: impl FnOnce(PopoverMenu, &mut Window, &mut App) -> PopoverMenu + 'static,
) -> impl IntoElement {
    anchored().position(position).child(
        Popover::new(id)
            .open(true)
            .side(Side::Bottom)
            .align(Align::Start)
            .side_offset(px(0.))
            .on_open_change(move |change, window, cx| {
                if !change.open {
                    on_close(window, cx);
                }
            })
            .trigger(|_| div().size(px(1.)))
            .menu(menu),
    )
}
