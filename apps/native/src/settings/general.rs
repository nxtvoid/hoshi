use super::SettingsView;
use crate::m;
use gpui_kit::*;
use hoshi_core::StartupView;
use hoshi_ui::prelude::*;

fn startup_label(view: StartupView) -> String {
    match view {
        StartupView::Inbox => m::inbox(),
        StartupView::LastProject => m::startup_last_project(),
    }
}

impl SettingsView {
    pub(super) fn general(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let general = self.settings(cx).general.clone();
        let startup = Select::new("settings-startup-view", startup_label(general.startup_view))
            .options([StartupView::Inbox, StartupView::LastProject].map(|view| {
                SelectOption::new(
                    SharedString::from(format!("startup-{view:?}")),
                    startup_label(view),
                    view == general.startup_view,
                )
                .on_select(self.setter(cx, move |settings| settings.general.startup_view = view))
            }));

        let mut page = vec![
            SettingsGroup::new()
                .child(
                    SettingsRow::new(m::startup_view())
                        .description(m::startup_view_description())
                        .control(startup),
                )
                .child(
                    SettingsRow::new(m::notifications_setting())
                        .description(m::notifications_setting_description())
                        .control(
                            Switch::new("settings-notifications")
                                .checked(general.notifications)
                                .on_change(self.toggler(cx, |settings, on| {
                                    settings.general.notifications = on
                                })),
                        ),
                )
                .into_any_element(),
        ];
        page.extend(self.save_error(cx));
        page
    }
}
