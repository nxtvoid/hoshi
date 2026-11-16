//! Inside the composer, above its bar: the provider the thread moves to with its next
//! message, which takes the conversation with it.

use super::Composer;
use super::model_picker::provider_label;
use crate::m;
use crate::state::Workspace;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::Command;
use hoshi_ui::prelude::*;

/// A model as the composer names it: its catalog name, else its slug, else its provider.
pub fn model_name(workspace: &Workspace, provider: &str, slug: Option<&str>) -> String {
    workspace
        .chosen_model(provider, slug)
        .map(|model| model.name.clone())
        .or_else(|| slug.map(str::to_owned))
        .unwrap_or_else(|| provider_label(provider))
}

impl Composer {
    pub(super) fn render_handoff(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (from, to, back) = {
            let workspace = self.workspace.read(cx);
            let thread = workspace.model.thread(&workspace.open_thread_id(cx)?)?;
            let pending = thread.pending_model.as_deref()?;
            (
                model_name(workspace, &thread.provider, thread.model.as_deref()),
                model_name(workspace, &pending.provider, pending.model.as_deref()),
                Command::ThreadModelSet {
                    thread_id: thread.id.clone(),
                    provider: thread.provider.clone(),
                    model: thread.model.clone(),
                    effort: thread.effort,
                    fast_mode: thread.fast_mode,
                    ultracode: thread.ultracode,
                },
            )
        };
        let colors = cx.hoshi().clone();
        Some(
            div()
                .debug_selector(|| "handoff-pending".into())
                .flex()
                .items_center()
                .gap_1p5()
                .pl_2()
                .pt_1()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(Icon::new(IconName::ArrowRightLeft).size_3p5().flex_none())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(m::handoff_pending(&from, &to)),
                )
                .child(
                    div().debug_selector(|| "handoff-cancel".into()).child(
                        Button::new("handoff-cancel")
                            .label(m::cancel())
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Ghost)
                            .text_color(colors.muted_foreground)
                            .tooltip(
                                Tooltip::new(m::handoff_pending_cancel_hint(&from)).side(Side::Top),
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.dispatch(back.clone(), cx)),
                            ),
                    ),
                )
                .into_any_element(),
        )
    }
}
