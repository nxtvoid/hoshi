//! The pieces of a wizard shown in a [`Dialog::wizard`](super::Dialog::wizard): the header with
//! the steps, the inset panel that holds the current step, and the footer.

use super::dialog::{DialogButton, DialogButtonSize, DialogButtonVariant};
use crate::assets::IconName;
use crate::theme::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

type SelectHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// The brand on the left, the close button on the right, and the steps below.
#[derive(IntoElement)]
pub struct WizardHeader {
    brand: AnyElement,
    close_label: SharedString,
    children: Vec<AnyElement>,
}

impl WizardHeader {
    pub fn new(brand: impl IntoElement, close_label: impl Into<SharedString>) -> Self {
        Self {
            brand: brand.into_any_element(),
            close_label: close_label.into(),
            children: Vec::new(),
        }
    }
}

impl ParentElement for WizardHeader {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for WizardHeader {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .flex_col()
            .gap_4()
            .px_6()
            .py_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(self.brand)
                    .child(
                        div().mr(px(-8.)).child(
                            DialogButton::new("wizard-close")
                                .icon(IconName::X)
                                .variant(DialogButtonVariant::Ghost)
                                .size(DialogButtonSize::IconSm)
                                .accessible_label(self.close_label)
                                .close(),
                        ),
                    ),
            )
            .children(self.children)
    }
}

/// A segmented track of steps; only the steps before `current` can be picked.
#[derive(IntoElement)]
pub struct WizardSteps {
    steps: Vec<SharedString>,
    current: usize,
    on_select: Option<SelectHandler>,
}

impl WizardSteps {
    pub fn new(steps: impl IntoIterator<Item = impl Into<SharedString>>, current: usize) -> Self {
        Self {
            steps: steps.into_iter().map(Into::into).collect(),
            current,
            on_select: None,
        }
    }

    pub fn on_select(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for WizardSteps {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let current = self.current;
        div()
            .flex()
            .gap_1()
            .p_1()
            .rounded_xl()
            .bg(colors.muted.opacity(0.6))
            .children(self.steps.into_iter().enumerate().map(|(index, step)| {
                let done = index < current;
                let active = index == current;
                let on_select = self.on_select.clone();
                let hover = colors.background.opacity(0.6);
                let marker = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size_5()
                    .rounded_full()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .map(|this| {
                        if done {
                            this.bg(colors.primary)
                                .text_color(colors.primary_foreground)
                                .child(Icon::new(IconName::Check).size_3())
                        } else if active {
                            this.bg(colors.primary.opacity(0.1))
                                .border_1()
                                .border_color(colors.primary.opacity(0.25))
                                .text_color(colors.foreground)
                                .child((index + 1).to_string())
                        } else {
                            this.bg(colors.background)
                                .border_1()
                                .border_color(colors.border)
                                .text_color(colors.muted_foreground)
                                .child((index + 1).to_string())
                        }
                    });
                div()
                    .id(("wizard-step", index))
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .px_2p5()
                    .py_1p5()
                    .rounded_lg()
                    .when(active, |this| {
                        this.bg(colors.background)
                            .border_1()
                            .border_color(colors.border)
                            .shadow_xs()
                    })
                    .when(done, |this| {
                        this.cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .when_some(on_select, |this, on_select| {
                                this.on_click(move |_, window, cx| on_select(index, window, cx))
                            })
                    })
                    .child(marker)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if active {
                                colors.foreground
                            } else {
                                colors.muted_foreground
                            })
                            .child(step),
                    )
            }))
    }
}

/// The inset band that holds the current step.
pub fn wizard_panel(cx: &App) -> Stateful<Div> {
    let colors = cx.hoshi();
    div()
        .id("wizard-panel")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px_8()
        .py_7()
        .bg(colors.muted.opacity(0.4))
        .border_y_1()
        .border_color(colors.border)
}

pub fn wizard_title(title: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_2xl()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.hoshi().foreground)
        .child(title.into())
}

pub fn wizard_description(description: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .mt_2()
        .text_sm()
        .line_height(relative(1.625))
        .text_color(cx.hoshi().muted_foreground)
        .child(description.into())
}

/// `leading` sits on the left, such as a skip button; the main actions on the right.
pub fn wizard_footer(leading: Option<AnyElement>, actions: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .px_6()
        .py_4()
        .children(leading)
        .child(div().ml_auto().flex().items_center().gap_2().child(actions))
}
