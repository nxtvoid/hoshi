use super::{
    Dialog, DialogButton, DialogButtonSize, DialogButtonVariant, DialogSize, InitialFocus, Surface,
};
use crate::assets::IconName;
use crate::theme::{ActiveTheme, Colors};
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cell::Cell;
use std::rc::Rc;

/// `destructive` for what can't be undone, `warning` to tell about a refusal, `success` for
/// something that went through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConfirmTone {
    #[default]
    Default,
    Destructive,
    Warning,
    Success,
}

impl ConfirmTone {
    fn color(self, colors: &Colors) -> Hsla {
        match self {
            Self::Default => colors.status_working,
            Self::Destructive => colors.destructive,
            Self::Warning => colors.status_waiting,
            Self::Success => colors.status_done,
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Default => IconName::CircleQuestionMark,
            Self::Destructive => IconName::Trash,
            Self::Warning => IconName::TriangleAlert,
            Self::Success => IconName::CircleCheck,
        }
    }

    fn action(self) -> DialogButtonVariant {
        match self {
            Self::Destructive => DialogButtonVariant::Destructive,
            _ => DialogButtonVariant::Primary,
        }
    }
}

/// Which way ahead the user took in [`DialogLayer::choose`](super::DialogLayer::choose).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmChoice {
    Confirm,
    Alternative,
}

/// What [`DialogLayer::confirm`](super::DialogLayer::confirm) asks. Without a cancel label it is
/// a notice with one button.
#[derive(Clone, Debug)]
pub struct ConfirmRequest {
    pub(crate) title: SharedString,
    pub(crate) description: Option<SharedString>,
    pub(crate) confirm_label: SharedString,
    pub(crate) cancel_label: Option<SharedString>,
    pub(crate) alternative: Option<(SharedString, bool)>,
    pub(crate) tone: ConfirmTone,
    pub(crate) icon: Option<IconName>,
}

impl ConfirmRequest {
    pub fn new(title: impl Into<SharedString>, confirm_label: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            description: None,
            confirm_label: confirm_label.into(),
            cancel_label: None,
            alternative: None,
            tone: ConfirmTone::default(),
            icon: None,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn cancel_label(mut self, label: impl Into<SharedString>) -> Self {
        self.cancel_label = Some(label.into());
        self
    }

    /// A second way ahead, between cancel and confirm; `destructive` shows it in that color.
    pub fn alternative(mut self, label: impl Into<SharedString>, destructive: bool) -> Self {
        self.alternative = Some((label.into(), destructive));
        self
    }

    pub fn tone(mut self, tone: ConfirmTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn destructive(self) -> Self {
        self.tone(ConfirmTone::Destructive)
    }

    /// Says what going ahead does, in place of the tone's own icon.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }
}

pub(crate) fn build(
    dialog: Dialog,
    request: &ConfirmRequest,
    answer: &Rc<Cell<Option<ConfirmChoice>>>,
    confirm_focus: &FocusHandle,
    cx: &App,
) -> Dialog {
    let colors = cx.hoshi();
    let tone = request.tone.color(colors);
    let halo = BoxShadow {
        color: tone.opacity(0.06),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(6.),
        inset: false,
    };
    let answer = answer.clone();
    let alternative_answer = answer.clone();
    let width = if request.alternative.is_some() {
        px(496.)
    } else {
        px(400.)
    };

    dialog
        .alert()
        .surface(Surface::Confirm)
        .size(DialogSize::Custom(width))
        .initial_focus(InitialFocus::Handle(confirm_focus.clone()))
        .title(request.title.clone())
        .when_some(request.description.clone(), |this, description| {
            this.description(description)
        })
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(44.))
                .rounded_full()
                .bg(tone.opacity(0.12))
                .shadow(vec![halo])
                .child(
                    Icon::new(request.icon.unwrap_or(request.tone.icon()))
                        .size(px(20.))
                        .text_color(tone),
                ),
        )
        .child(
            div()
                .mt_4()
                .text_base()
                .line_height(relative(1.375))
                .font_weight(FontWeight::SEMIBOLD)
                .child(request.title.clone()),
        )
        .when_some(request.description.clone(), |this, description| {
            this.child(
                div()
                    .mt_1p5()
                    .text_sm()
                    .text_color(colors.muted_foreground)
                    .child(description),
            )
        })
        .child(
            div()
                .mt_6()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap_2()
                .when_some(request.cancel_label.clone(), |this, label| {
                    this.child(
                        DialogButton::new("confirm-cancel")
                            .label(label)
                            .variant(DialogButtonVariant::Outline)
                            .size(DialogButtonSize::Lg)
                            .close(),
                    )
                })
                .when_some(request.alternative.clone(), |this, (label, destructive)| {
                    this.child(
                        DialogButton::new("confirm-alternative")
                            .label(label)
                            .variant(if destructive {
                                DialogButtonVariant::OutlineDestructive
                            } else {
                                DialogButtonVariant::Outline
                            })
                            .size(DialogButtonSize::Lg)
                            .on_click(move |_, _, _| {
                                alternative_answer.set(Some(ConfirmChoice::Alternative))
                            })
                            .close(),
                    )
                })
                .child(
                    DialogButton::new("confirm-action")
                        .label(request.confirm_label.clone())
                        .variant(request.tone.action())
                        .size(DialogButtonSize::Lg)
                        .track_focus(confirm_focus)
                        .on_click(move |_, _, _| answer.set(Some(ConfirmChoice::Confirm)))
                        .close(),
                ),
        )
}
