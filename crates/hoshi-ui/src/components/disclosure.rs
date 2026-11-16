use crate::ClickHandler;
use crate::assets::IconName;
use crate::components::PressSelectsNothing as _;
use crate::theme::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

const GROUP: &str = "disclosure";

/// A one-line row that opens to show more below it. The caller keeps whether it is open
/// and flips it in `on_toggle`; the body is only built while open.
#[derive(IntoElement)]
pub struct Disclosure {
    id: ElementId,
    open: bool,
    expandable: bool,
    nested: bool,
    header: Vec<AnyElement>,
    body: Option<AnyElement>,
    on_toggle: Option<ClickHandler>,
}

impl Disclosure {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            open: false,
            expandable: true,
            nested: false,
            header: Vec::new(),
            body: None,
            on_toggle: None,
        }
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// A row with nothing to show stays one line, without a chevron.
    pub fn expandable(mut self, expandable: bool) -> Self {
        self.expandable = expandable;
        self
    }

    /// The body holds more rows, set off by a rule at their left instead of a panel.
    pub fn nested(mut self, nested: bool) -> Self {
        self.nested = nested;
        self
    }

    pub fn body(mut self, body: impl IntoElement) -> Self {
        self.body = Some(body.into_any_element());
        self
    }

    pub fn on_toggle(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
}

impl ParentElement for Disclosure {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.header.extend(elements);
    }
}

impl RenderOnce for Disclosure {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        let open = self.open && self.expandable;
        let chevron = Icon::new(if open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .size_3p5()
        .flex_none();
        let selector = format!("disclosure-{}", self.id);
        let header = div()
            .id(self.id)
            .debug_selector(|| selector)
            .group(GROUP)
            .flex()
            .items_center()
            .gap_2()
            .min_h_6()
            .min_w_0()
            .text_sm()
            .text_color(colors.muted_foreground)
            .hover(|row| row.text_color(colors.foreground))
            .children(self.header)
            .when(self.expandable, |row| {
                row.cursor_pointer()
                    .when_some(self.on_toggle, |row, handler| {
                        row.press_selects_nothing().on_click(handler)
                    })
                    .child(
                        div()
                            .flex_none()
                            .when(!open, |mark| {
                                mark.opacity(0.).group_hover(GROUP, |mark| mark.opacity(1.))
                            })
                            .child(chevron),
                    )
            });
        let nested = self.nested;
        let body = self.body.filter(|_| open).map(|body| {
            div()
                .mt_1()
                .when(nested, |body| {
                    body.ml_2()
                        .pl_2p5()
                        .border_l_1()
                        .border_color(colors.border.opacity(0.6))
                })
                .when(!nested, |body| {
                    body.ml_6()
                        .mb_2()
                        .rounded_md()
                        .overflow_hidden()
                        .bg(colors.muted.opacity(0.4))
                })
                .child(body)
        });
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(header)
            .children(body)
    }
}
