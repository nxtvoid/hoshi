use super::SettingsView;
use crate::m;
use gpui_kit::*;
use hoshi_ui::prelude::*;

impl SettingsView {
    pub(super) fn about(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let run_again = Button::new("run-onboarding-again")
            .label(m::onboarding_run_again_action())
            .variant(ButtonVariant::Outline)
            .size(ButtonSize::Sm)
            .on_click(cx.listener(|this, _, _, cx| {
                this.workspace
                    .update(cx, |_, cx| cx.emit(crate::onboarding::RunOnboarding));
            }));
        vec![
            SettingsGroup::new()
                .child(
                    SettingsRow::new(m::app_name())
                        .description(m::app_version(env!("CARGO_PKG_VERSION"))),
                )
                .child(
                    SettingsRow::new(m::onboarding_run_again())
                        .description(m::onboarding_run_again_description())
                        .control(run_again),
                )
                .into_any_element(),
        ]
    }
}
