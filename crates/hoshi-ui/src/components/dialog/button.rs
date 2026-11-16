use super::Close;
use crate::assets::IconName;
use crate::theme::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogButtonVariant {
    #[default]
    Primary,
    Outline,
    Ghost,
    /// Solid, for what can't be undone.
    Destructive,
    /// Outlined with destructive text, for the riskier of two ways ahead.
    OutlineDestructive,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogButtonSize {
    #[default]
    Default,
    /// `h-9 px-4`, the confirm dialog's buttons.
    Lg,
    IconSm,
}

type Handler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A dialog action: a tab stop with a focus ring, pressed with the pointer, Enter or Space.
#[derive(IntoElement)]
pub struct DialogButton {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<IconName>,
    icon_end: Option<IconName>,
    variant: DialogButtonVariant,
    size: DialogButtonSize,
    disabled: bool,
    closes: bool,
    focus: Option<FocusHandle>,
    accessible_label: Option<SharedString>,
    on_click: Option<Handler>,
}

impl DialogButton {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: None,
            icon: None,
            icon_end: None,
            variant: DialogButtonVariant::default(),
            size: DialogButtonSize::default(),
            disabled: false,
            closes: false,
            focus: None,
            accessible_label: None,
            on_click: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// An icon after the label, such as the arrow of a step forward.
    pub fn icon_end(mut self, icon: IconName) -> Self {
        self.icon_end = Some(icon);
        self
    }

    pub fn variant(mut self, variant: DialogButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: DialogButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Closes the dialog it sits in after `on_click`, reported as
    /// [`ClosePress`](super::OpenChangeReason::ClosePress), like Base UI's `Dialog.Close`.
    pub fn close(mut self) -> Self {
        self.closes = true;
        self
    }

    /// Uses a caller-owned focus handle, to aim [`InitialFocus::Handle`](super::InitialFocus)
    /// at this button.
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }

    pub fn accessible_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessible_label = Some(label.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for DialogButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let (bg, fg, hover, border) = match self.variant {
            DialogButtonVariant::Primary => (
                colors.primary,
                colors.primary_foreground,
                colors.primary.opacity(0.8),
                None,
            ),
            DialogButtonVariant::Outline => (
                colors.background,
                colors.foreground,
                colors.muted,
                Some(colors.border),
            ),
            DialogButtonVariant::OutlineDestructive => (
                colors.background,
                colors.destructive,
                colors.muted,
                Some(colors.border),
            ),
            DialogButtonVariant::Ghost => {
                (transparent_black(), colors.foreground, colors.muted, None)
            }
            DialogButtonVariant::Destructive => (
                colors.destructive,
                colors.destructive_foreground,
                colors.destructive.opacity(0.9),
                None,
            ),
        };
        let (height, pad, text, icon_size) = match self.size {
            DialogButtonSize::Default => (px(32.), px(10.), rems(0.875), px(16.)),
            DialogButtonSize::Lg => (px(36.), px(16.), rems(0.875), px(16.)),
            DialogButtonSize::IconSm => (px(28.), px(0.), rems(0.8), px(16.)),
        };
        let focus = self.focus.unwrap_or_else(|| {
            window
                .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
                .read(cx)
                .clone()
        });
        let disabled = self.disabled;
        let ring = !disabled && focus.is_focused(window) && window.last_input_was_keyboard();
        let on_click = self.on_click;
        let closes = self.closes;
        let anchor = focus.clone();

        gpui_kit::base::Button::new(self.id)
            .track_focus(&focus)
            .disabled(disabled)
            .relative()
            .flex_none()
            .gap_1()
            .h(height)
            .when(self.size == DialogButtonSize::IconSm, |this| this.w(height))
            .px(pad)
            .rounded(px(10.))
            .border_1()
            .border_color(if ring {
                colors.ring
            } else {
                border.unwrap_or(transparent_black())
            })
            .bg(bg)
            .text_color(fg)
            .text_size(text)
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .when(disabled, |this| this.opacity(0.5))
            .when(!disabled, |this| {
                this.cursor_pointer().hover(move |style| style.bg(hover))
            })
            .when_some(self.accessible_label, |this, label| {
                this.accessibility_label(label)
            })
            .on_click(move |event, window, cx| {
                if let Some(on_click) = &on_click {
                    on_click(event, window, cx);
                }
                if closes {
                    anchor.dispatch_action(&Close, window, cx);
                }
            })
            .when_some(self.icon, |this, icon| {
                this.child(Icon::new(icon).size(icon_size).text_color(fg))
            })
            .when_some(self.label, |this, label| this.child(label))
            .when_some(self.icon_end, |this, icon| {
                this.child(Icon::new(icon).size(icon_size).text_color(fg))
            })
            .when(ring, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(-4.))
                        .left(px(-4.))
                        .right(px(-4.))
                        .bottom(px(-4.))
                        .rounded(px(13.))
                        .border(px(3.))
                        .border_color(colors.ring.opacity(0.5)),
                )
            })
    }
}
