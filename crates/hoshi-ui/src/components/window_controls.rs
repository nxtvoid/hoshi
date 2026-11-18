use crate::assets::IconName;
use crate::theme::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Minimize,
    Maximize,
    Close,
}

/// Minimize, maximize or restore, and close, for a window that draws its own title bar.
/// On Windows the system handles them (snap layouts on maximize included); elsewhere a click
/// does it. macOS keeps its own traffic lights, so nothing is drawn there.
#[derive(IntoElement, Default)]
pub struct WindowControls;

impl WindowControls {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for WindowControls {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if cfg!(target_os = "macos") {
            return div().into_any_element();
        }
        let maximized = window.is_maximized();
        div()
            .flex()
            .flex_none()
            .h_full()
            .child(control(Control::Minimize, maximized, cx))
            .child(control(Control::Maximize, maximized, cx))
            .child(control(Control::Close, maximized, cx))
            .into_any_element()
    }
}

fn control(control: Control, maximized: bool, cx: &App) -> impl IntoElement {
    let colors = cx.hoshi();
    let (hover_bg, hover_fg) = if control == Control::Close {
        (colors.window_close, colors.window_close_foreground)
    } else {
        (colors.accent, colors.foreground)
    };
    let (id, icon, area) = match control {
        Control::Minimize => ("window-minimize", IconName::Minus, WindowControlArea::Min),
        Control::Maximize if maximized => {
            ("window-maximize", IconName::Copy, WindowControlArea::Max)
        }
        Control::Maximize => ("window-maximize", IconName::Square, WindowControlArea::Max),
        Control::Close => ("window-close", IconName::X, WindowControlArea::Close),
    };
    let size = if control == Control::Maximize && !maximized {
        px(14.)
    } else {
        px(16.)
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w_11()
        .h_full()
        .text_color(colors.foreground)
        .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
        .when(cfg!(target_os = "windows"), |this| {
            this.window_control_area(area)
        })
        .when(!cfg!(target_os = "windows"), |this| {
            this.on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                match control {
                    Control::Minimize => window.minimize_window(),
                    Control::Maximize => window.zoom_window(),
                    Control::Close => window.remove_window(),
                }
            })
        })
        .child(Icon::new(icon).size(size))
}

/// Makes an element a handle that moves the window, as a title bar does: drag to move,
/// double-click to maximize. Nothing clickable may sit inside it: the window asks for the
/// first control area under the mouse, and a parent's comes before its children's.
pub trait DragRegion: StatefulInteractiveElement + Sized {
    fn drag_region(self) -> Self {
        let this = self.window_control_area(WindowControlArea::Drag);
        if !cfg!(any(target_os = "linux", target_os = "freebsd")) {
            return this;
        }
        // Linux backends ignore control areas, so the move is asked of the compositor here.
        this.on_mouse_down(MouseButton::Left, |event, window, cx| {
            cx.stop_propagation();
            if event.click_count == 2 {
                window.zoom_window();
            }
        })
        .on_drag(WindowMove, |_, _, window, cx| {
            window.start_window_move();
            window.defer(cx, |window, cx| {
                cx.stop_active_drag(window);
            });
            cx.new(|_| WindowMove)
        })
        .on_mouse_down(MouseButton::Right, |event, window, cx| {
            cx.stop_propagation();
            window.show_window_menu(event.position)
        })
    }
}

impl<E: StatefulInteractiveElement> DragRegion for E {}

struct WindowMove;

impl Render for WindowMove {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}
