use crate::ClickHandler;
use crate::assets::IconName;
use crate::components::PressSelectsNothing as _;
use crate::theme::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// A muted line, with a chevron after it, that opens to the rows it folds, set off by a rule
/// at their left. A rule below closes it. The caller keeps whether it is open and flips it in
/// `on_toggle`; its rows are only built while open.
#[derive(IntoElement)]
pub struct FoldRow {
    id: ElementId,
    label: SharedString,
    open: bool,
    rows: Vec<AnyElement>,
    on_toggle: Option<ClickHandler>,
}

impl FoldRow {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            open: false,
            rows: Vec::new(),
            on_toggle: None,
        }
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
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

impl ParentElement for FoldRow {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.rows.extend(elements);
    }
}

impl RenderOnce for FoldRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi();
        let rule = colors.border.opacity(0.6);
        let selector = format!("fold-row-{}", self.id);
        let chevron = Icon::new(if self.open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .size_3p5()
        .flex_none();
        let toggle = div()
            .id(self.id)
            .debug_selector(|| selector)
            .flex()
            .items_center()
            .gap_1()
            .min_h_6()
            .text_sm()
            .text_color(colors.muted_foreground)
            .hover(|line| line.text_color(colors.foreground))
            .cursor_pointer()
            .when_some(self.on_toggle, |line, handler| {
                line.press_selects_nothing().on_click(handler)
            })
            .child(self.label)
            .child(chevron);
        let open = self.open;
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .pb_2()
            .border_b_1()
            .border_color(rule)
            .child(div().flex().child(toggle))
            .when(open, |fold| {
                fold.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .min_w_0()
                        .ml_2()
                        .my_2()
                        .pl_4()
                        .border_l_1()
                        .border_color(rule)
                        .children(self.rows),
                )
            })
    }
}
