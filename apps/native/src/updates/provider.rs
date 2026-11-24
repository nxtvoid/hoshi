use crate::m;
use crate::state::Workspace;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{
    ProviderUpdate, ProviderUpdateFailure, ProviderVersion, ThreadStatus, UpdateStep,
};
use hoshi_engine::ProvidersStatus;
use hoshi_ui::prelude::{Colors, IconName};

pub fn version_summary(version: &ProviderVersion) -> String {
    let Some(installed) = &version.installed else {
        return m::provider_version_not_found();
    };
    match (&version.latest, version.update_available) {
        (Some(latest), true) => m::provider_version_update_available(installed, latest),
        (Some(_), false) => m::provider_version_up_to_date(installed),
        (None, _) => installed.clone(),
    }
}

fn failure_message(reason: &ProviderUpdateFailure) -> String {
    match reason {
        ProviderUpdateFailure::TurnsRunning => m::provider_update_turns_running(),
        ProviderUpdateFailure::Unsupported => m::provider_update_unsupported(),
        ProviderUpdateFailure::SpawnFailed => m::provider_update_spawn_failed(),
        ProviderUpdateFailure::TimedOut => m::provider_update_timed_out(),
        ProviderUpdateFailure::Exited { code: None } => m::provider_update_stopped(),
        ProviderUpdateFailure::Exited { code: Some(code) } => m::provider_update_exited(code),
        ProviderUpdateFailure::DownloadFailed => m::provider_update_download_failed(),
        ProviderUpdateFailure::VerifyFailed => m::provider_update_verify_failed(),
        ProviderUpdateFailure::WriteFailed => m::provider_update_write_failed(),
    }
}

/// What a running update shows: the step of an install hoshi runs, else the updater's line.
pub fn running_text(line: Option<&str>, step: Option<UpdateStep>) -> String {
    match step {
        Some(UpdateStep::Downloading {
            percent: Some(percent),
        }) => m::provider_install_downloading_percent(percent),
        Some(UpdateStep::Downloading { percent: None }) => m::provider_install_downloading(),
        Some(UpdateStep::Extracting) => m::provider_install_extracting(),
        Some(UpdateStep::Verifying) => m::provider_install_verifying(),
        None => line.map_or_else(m::provider_update_running, str::to_owned),
    }
}

/// The CLI an update replaces is what turns run.
pub fn turns_running(workspace: &Workspace) -> bool {
    workspace
        .model
        .threads()
        .any(|thread| thread.status == ThreadStatus::Working)
}

/// Reads the providers' CLIs and accounts as the engine knows them, unless something already
/// did.
pub fn load_version(workspace: &Entity<Workspace>, cx: &mut App) {
    if !unknown(&workspace.read(cx).providers) {
        return;
    }
    let engine = workspace.read(cx).engine.clone();
    let workspace = workspace.downgrade();
    cx.spawn(async move |cx| {
        let status = cx
            .background_spawn(async move { engine.providers_status() })
            .await;
        workspace
            .update(cx, |workspace, cx| {
                if unknown(&workspace.providers) {
                    workspace.set_providers(status);
                    cx.notify();
                }
            })
            .ok();
    })
    .detach();
}

fn unknown(status: &ProvidersStatus) -> bool {
    status.versions.is_empty() && status.checked_at.is_none()
}

/// Starts the update of the default provider's CLI; its progress arrives as its version.
pub fn start_update(workspace: &Entity<Workspace>, cx: &mut App) {
    start_update_of(workspace, hoshi_core::DEFAULT_PROVIDER, cx);
}

/// Starts the update of `provider`'s CLI; its progress arrives as its version.
pub fn start_update_of(workspace: &Entity<Workspace>, provider: &str, cx: &mut App) {
    let engine = workspace.read(cx).engine.clone();
    let provider = provider.to_owned();
    cx.background_spawn(async move { engine.update_provider_of(&provider) })
        .detach();
}

/// How the last update goes or went; nothing while none ran.
pub fn update_status(
    version: &ProviderVersion,
    colors: &Colors,
    mono: SharedString,
) -> Option<AnyElement> {
    let line = |icon: IconName, tone: Hsla, text: String, wrap: bool| {
        div()
            .flex()
            .min_w_0()
            .items_start()
            .gap_1p5()
            .text_xs()
            .text_color(colors.muted_foreground)
            .child(
                div()
                    .flex_none()
                    .mt(px(1.))
                    .child(if icon == IconName::LoaderCircle {
                        hoshi_ui::spinner(px(14.), tone)
                    } else {
                        Icon::new(icon)
                            .size_3p5()
                            .text_color(tone)
                            .into_any_element()
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .when(!wrap, |this| this.truncate())
                    .child(text),
            )
    };
    match &version.update {
        ProviderUpdate::Idle => (version.update_available && version.update_command.is_none())
            .then(|| {
                div()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(m::provider_update_unsupported())
                    .into_any_element()
            }),
        ProviderUpdate::Running {
            line: progress,
            step,
        } => Some(
            line(
                IconName::LoaderCircle,
                colors.muted_foreground,
                running_text(progress.as_deref(), *step),
                false,
            )
            .into_any_element(),
        ),
        ProviderUpdate::Succeeded => Some(
            line(
                IconName::CircleCheck,
                colors.status_done,
                m::provider_update_succeeded(version.installed.clone().unwrap_or_default()),
                false,
            )
            .into_any_element(),
        ),
        ProviderUpdate::Unchanged => Some(
            line(
                IconName::TriangleAlert,
                colors.status_error,
                m::provider_update_unchanged(),
                true,
            )
            .into_any_element(),
        ),
        ProviderUpdate::Failed { reason, output } => Some(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap_1p5()
                .child(line(
                    IconName::TriangleAlert,
                    colors.status_error,
                    failure_message(reason),
                    true,
                ))
                .when_some(output.clone(), |this, output| {
                    this.child(
                        div()
                            .id("provider-update-output")
                            .max_h_32()
                            .overflow_y_scroll()
                            .rounded_md()
                            .px_2()
                            .py_1p5()
                            .bg(colors.muted)
                            .text_color(colors.muted_foreground)
                            .text_size(rems(0.6875))
                            .line_height(rems(1.))
                            .font_family(mono)
                            .child(output),
                    )
                })
                .into_any_element(),
        ),
    }
}
