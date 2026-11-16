//! `HOSHI_NATIVE_DIALOG=confirm|dialog|nested` opens a sample dialog on start, to screenshot it.

use crate::m;
use crate::state::Workspace;
use gpui_kit::component::Selectable as _;
use gpui_kit::*;
use hoshi_ui::prelude::{
    Button, ButtonVariant, ConfirmRequest, DialogButton, DialogButtonVariant, DialogLayer,
    DialogSize, MenuOption, Popover,
};
use hoshi_ui::theme::ActiveTheme;
use std::time::Duration;

pub fn open(kind: &str, workspace: &Entity<Workspace>, window: &mut Window, cx: &mut App) {
    let model = &workspace.read(cx).model;
    let project = model
        .projects()
        .next()
        .map(|project| project.title.clone())
        .unwrap_or_default();
    let thread = model
        .threads()
        .next()
        .map(|thread| thread.title.clone())
        .unwrap_or_else(m::new_chat);
    match kind {
        "confirm" => delete_thread(thread, window, cx),
        "dialog" => worktree_setup(project, window, cx),
        "nested" => {
            worktree_setup(project.clone(), window, cx);
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(400))
                        .await;
                    cx.update(|window, cx| remove_project(project, window, cx))
                        .ok();
                })
                .detach();
        }
        _ => {}
    }
}

fn delete_thread(title: String, window: &mut Window, cx: &mut App) {
    DialogLayer::confirm(
        ConfirmRequest::new(m::delete_thread(), m::delete_thread())
            .description(m::delete_thread_confirm(title))
            .cancel_label(m::cancel())
            .destructive(),
        window,
        cx,
    )
    .detach();
}

fn remove_project(title: String, window: &mut Window, cx: &mut App) {
    DialogLayer::confirm(
        ConfirmRequest::new(m::remove_project(), m::remove_project())
            .description(m::remove_project_confirm(title))
            .cancel_label(m::cancel())
            .destructive(),
        window,
        cx,
    )
    .detach();
}

fn worktree_setup(project: String, window: &mut Window, cx: &mut App) {
    DialogLayer::open(window, cx, move |dialog, _, cx| {
        let colors = cx.hoshi();
        let field = |label: String, value: String, hint: String| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(label),
                )
                .child(
                    div()
                        .px_2p5()
                        .py_1p5()
                        .rounded_lg()
                        .border_1()
                        .border_color(colors.input)
                        .text_xs()
                        .child(value),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(hint),
                )
        };
        let project = project.clone();
        dialog
            .size(DialogSize::Lg)
            .title(m::worktree_setup_title())
            .description(m::worktree_setup_description(&project))
            .child(field(
                m::worktree_setup_files(),
                ".env".into(),
                m::worktree_setup_files_hint(),
            ))
            .child(field(
                m::worktree_setup_command(),
                m::worktree_setup_placeholder(),
                m::worktree_setup_command_hint(),
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Popover::new("share")
                            .trigger(|open| {
                                Button::new("share-trigger")
                                    .label(m::worktree_setup_share())
                                    .variant(ButtonVariant::Ghost)
                                    .selected(open)
                            })
                            .menu(|menu, _, _| {
                                menu.item(MenuOption::new("copy", m::worktree_setup_copy()))
                                    .item(MenuOption::new(
                                        "import",
                                        m::worktree_setup_import_menu(),
                                    ))
                            }),
                    )
                    .child(
                        DialogButton::new("remove-project")
                            .label(m::remove_project())
                            .variant(DialogButtonVariant::Ghost)
                            .on_click(move |_, window, cx| {
                                remove_project(project.clone(), window, cx)
                            }),
                    ),
            )
            .footer(
                DialogButton::new("cancel")
                    .label(m::cancel())
                    .variant(DialogButtonVariant::Outline)
                    .close(),
            )
            .footer(DialogButton::new("save").label(m::save()).close())
    });
}
