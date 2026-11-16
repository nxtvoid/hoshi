use super::tooltip::{Tooltip, TooltipTrigger};
use crate::ClickHandler;
use crate::assets::IconName;
use crate::theme::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Outline,
    Ghost,
    Destructive,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    Xs,
    Sm,
    #[default]
    Default,
    IconXs,
    IconSm,
    Icon,
}

impl ButtonSize {
    fn is_icon(self) -> bool {
        matches!(self, Self::IconXs | Self::IconSm | Self::Icon)
    }
}

/// A button with the web's variants and sizes.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<IconName>,
    variant: ButtonVariant,
    size: ButtonSize,
    rounded_full: bool,
    disabled: bool,
    selected: bool,
    color: Option<Hsla>,
    loading: bool,
    suffix: Option<AnyElement>,
    tooltip: Option<Tooltip>,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: None,
            icon: None,
            variant: ButtonVariant::default(),
            size: ButtonSize::default(),
            rounded_full: false,
            disabled: false,
            selected: false,
            color: None,
            loading: false,
            suffix: None,
            tooltip: None,
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

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn rounded_full(mut self) -> Self {
        self.rounded_full = true;
        self
    }

    /// Shown on hover; an icon button's name. Text, or a [`Tooltip`] for the side, the shortcut
    /// and the rest. It stays closed while the button is selected, as when its popover is open.
    pub fn tooltip(mut self, tooltip: impl Into<Tooltip>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Shown after the label, such as the key that presses it.
    pub fn suffix(mut self, element: impl IntoElement) -> Self {
        self.suffix = Some(element.into_any_element());
        self
    }

    /// Shows the turning loading circle in place of the icon.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// The label and icon in `color` instead of the variant's.
    pub fn text_color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

/// Selected shows as pressed; a popover marks its trigger selected while it is open.
impl gpui_kit::component::Selectable for Button {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        let (bg, fg, hover, border) = match self.variant {
            ButtonVariant::Primary => (
                colors.primary,
                colors.primary_foreground,
                colors.primary.opacity(0.8),
                None,
            ),
            ButtonVariant::Secondary => (
                colors.secondary,
                colors.secondary_foreground,
                colors.secondary.blend(colors.foreground.opacity(0.05)),
                None,
            ),
            ButtonVariant::Outline => (
                colors.background,
                colors.foreground,
                colors.muted,
                Some(colors.border),
            ),
            ButtonVariant::Ghost => (
                gpui_kit::transparent_black(),
                colors.foreground,
                colors.muted,
                None,
            ),
            ButtonVariant::Destructive => (
                colors.destructive.opacity(0.1),
                colors.destructive,
                colors.destructive.opacity(0.2),
                None,
            ),
        };
        let (height, pad, text, icon_size, radius) = match self.size {
            ButtonSize::Xs => (px(24.), px(8.), rems(0.75), px(12.), px(8.)),
            ButtonSize::Sm => (px(28.), px(10.), rems(0.8), px(14.), px(10.)),
            ButtonSize::Default => (px(32.), px(10.), rems(0.875), px(16.), px(10.)),
            ButtonSize::IconXs => (px(24.), px(0.), rems(0.75), px(12.), px(8.)),
            ButtonSize::IconSm => (px(28.), px(0.), rems(0.8), px(16.), px(10.)),
            ButtonSize::Icon => (px(32.), px(0.), rems(0.875), px(16.), px(10.)),
        };
        let hover = match (self.variant, self.color) {
            (ButtonVariant::Ghost, Some(color)) => color.opacity(0.1),
            _ => hover,
        };
        let fg = self.color.unwrap_or(fg);
        let disabled = self.disabled;
        let bg = if self.selected { hover } else { bg };
        let selected = self.selected;
        let tooltip = self.tooltip.map(|tooltip| {
            let tooltip = if selected {
                tooltip.disabled(true)
            } else {
                tooltip
            };
            (self.id.clone(), tooltip)
        });

        let button = div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap_1()
            .h(height)
            .when(self.size.is_icon(), |this| this.w(height))
            .px(pad)
            .when(self.rounded_full, |this| this.rounded_full())
            .when(!self.rounded_full, |this| this.rounded(radius))
            .border_1()
            .border_color(border.unwrap_or(gpui_kit::transparent_black()))
            .bg(bg)
            .text_color(fg)
            .text_size(text)
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .when(disabled, |this| this.opacity(0.5))
            .when(!disabled, |this| {
                this.cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .when_some(self.on_click, |this, handler| {
                        this.on_mouse_down(MouseButton::Left, |_, _, cx| {
                            gpui_kit::base::GlobalState::suppress_text_selection(cx)
                        })
                        .on_click(move |event, window, cx| {
                            cx.stop_propagation();
                            handler(event, window, cx)
                        })
                    })
            })
            .when(self.loading, |this| {
                this.child(crate::components::spinner(icon_size, fg))
            })
            .when_some(self.icon.filter(|_| !self.loading), |this, icon| {
                this.child(Icon::new(icon).size(icon_size).text_color(fg))
            })
            .when_some(self.label, |this, label| this.child(label))
            .children(self.suffix);
        match tooltip {
            Some((id, tooltip)) => TooltipTrigger::new(id, button, tooltip).into_any_element(),
            None => button.into_any_element(),
        }
    }
}

/// A key, as shown next to what it does.
#[derive(IntoElement)]
pub struct Kbd {
    key: SharedString,
}

impl Kbd {
    pub fn new(key: impl Into<SharedString>) -> Self {
        Self { key: key.into() }
    }
}

impl RenderOnce for Kbd {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h_5()
            .min_w_5()
            .px_1()
            .rounded_sm()
            .bg(colors.muted)
            .text_color(colors.muted_foreground)
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .child(self.key)
    }
}

/// A card above the composer: what the agent waits on.
#[derive(IntoElement)]
pub struct Panel {
    children: Vec<AnyElement>,
}

impl Panel {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }
}

impl Default for Panel {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for Panel {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Panel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        div()
            .mb_2()
            .flex()
            .flex_col()
            .gap_2p5()
            .p_3()
            .rounded_2xl()
            .border_1()
            .border_color(colors.border)
            .bg(colors.card)
            .shadow_xs()
            .children(self.children)
    }
}
