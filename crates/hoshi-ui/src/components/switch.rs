use crate::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

type ChangeHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// An on/off toggle, like the web's `Switch`.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    checked: bool,
    disabled: bool,
    on_change: Option<ChangeHandler>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            checked: false,
            disabled: false,
            on_change: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called with the new state when it is pressed.
    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| {
                cx.focus_handle().tab_stop(true)
            })
            .read(cx)
            .clone();
        let colors = cx.hoshi();
        let ring = crate::theme::focus_ring(colors);
        let checked = self.checked;
        let track = if checked {
            colors.primary
        } else {
            colors.input
        };
        let thumb = if checked {
            colors.primary_foreground
        } else {
            colors.background
        };
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .w(px(32.))
            .h(px(18.))
            .p(px(1.))
            .rounded_full()
            .bg(track)
            .when(checked, |this| this.justify_end())
            .when(self.disabled, |this| this.opacity(0.5))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .track_focus(&focus)
                    .focus_visible(move |style| style.shadow(ring))
                    .when_some(self.on_change, |this, handler| {
                        let pressed = handler.clone();
                        this.on_mouse_down(MouseButton::Left, |_, _, cx| {
                            gpui_kit::base::GlobalState::suppress_text_selection(cx)
                        })
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            handler(!checked, window, cx)
                        })
                        .on_key_down(move |event, window, cx| {
                            if matches!(event.keystroke.key.as_str(), "space" | "enter") {
                                cx.stop_propagation();
                                pressed(!checked, window, cx)
                            }
                        })
                    })
            })
            .child(div().size(px(16.)).rounded_full().bg(thumb).shadow_sm())
    }
}
