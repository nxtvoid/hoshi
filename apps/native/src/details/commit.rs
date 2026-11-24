use super::ThreadDetails;
use super::git::on_default_branch;
use crate::m;
use gpui_kit::component::input::TextareaState;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;
use hoshi_core::{DiffFile, DiffScope, GitAction, GitActionRequest, GitStatus};
use hoshi_engine::Engine;
use hoshi_ui::prelude::*;
use std::collections::HashSet;
use std::sync::Arc;

/// "Commit…": the files that go in, which can be left out, and the user's own message.
struct CommitForm {
    files: Option<Vec<DiffFile>>,
    excluded: HashSet<String>,
    message: Entity<TextareaState>,
}

impl CommitForm {
    fn selected(&self) -> Vec<&DiffFile> {
        self.files
            .iter()
            .flatten()
            .filter(|file| !self.excluded.contains(&file.path))
            .collect()
    }

    fn request(&self, new_branch: bool, cx: &App) -> GitActionRequest {
        let message = self.message.read(cx).value().to_string();
        let paths = (!self.excluded.is_empty()).then(|| {
            self.selected()
                .into_iter()
                .flat_map(|file| file.old_path.iter().chain([&file.path]).cloned())
                .collect()
        });
        GitActionRequest {
            action: GitAction::Commit,
            message: (!message.trim().is_empty()).then_some(message),
            paths,
            new_branch,
            draft: false,
        }
    }
}

pub fn open(
    details: WeakEntity<ThreadDetails>,
    thread_id: String,
    status: GitStatus,
    engine: Arc<Engine>,
    window: &mut Window,
    cx: &mut Context<ThreadDetails>,
) {
    let message = cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(3, 6)
            .placeholder(m::git_commit_message_placeholder())
    });
    let form = cx.new(|_| CommitForm {
        files: None,
        excluded: HashSet::new(),
        message,
    });
    let reader = form.downgrade();
    let id = thread_id.clone();
    cx.spawn(async move |_, cx| {
        let files = cx
            .background_spawn(async move { engine.thread_diff_files(&id, DiffScope::Uncommitted) })
            .await
            .unwrap_or_default();
        reader
            .update(cx, |form, cx| {
                form.files = Some(files);
                cx.notify();
            })
            .ok();
    })
    .detach();
    DialogLayer::open(window, cx, move |dialog, _, cx| {
        let colors = cx.hoshi().clone();
        let mono = cx.theme().mono_font_family.clone();
        let state = form.read(cx);
        let selected = state.selected().len();
        let total = state.files.as_ref().map(Vec::len);
        let rows: Vec<_> = state
            .files
            .iter()
            .flatten()
            .enumerate()
            .map(|(index, file)| {
                let excluded = state.excluded.contains(&file.path);
                let path = file.path.clone();
                let form = form.clone();
                let hover = colors.accent.opacity(0.6);
                div()
                    .id(("commit-file", index))
                    .flex()
                    .h_7()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .px(px(6.))
                    .text_xs()
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .child(checkbox(!excluded, &colors))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono.clone())
                            .when(excluded, |this| {
                                this.text_color(colors.muted_foreground).line_through()
                            })
                            .child(file.path.clone()),
                    )
                    .when_some(file.additions, |this, additions| {
                        this.child(
                            div()
                                .flex()
                                .flex_none()
                                .gap_1()
                                .font_family(mono.clone())
                                .child(
                                    div()
                                        .text_color(colors.pr_open)
                                        .child(format!("+{additions}")),
                                )
                                .child(
                                    div()
                                        .text_color(colors.status_error)
                                        .child(format!("−{}", file.deletions.unwrap_or(0))),
                                ),
                        )
                    })
                    .on_click(move |_, _, cx| {
                        form.update(cx, |form, cx| {
                            if !form.excluded.remove(&path) {
                                form.excluded.insert(path.clone());
                            }
                            cx.notify();
                        })
                    })
            })
            .collect();
        let label = |text: String| {
            div()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(text)
        };
        let commit = |new_branch: bool| {
            let form = form.clone();
            let details = details.clone();
            let thread_id = thread_id.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let request = form.read(cx).request(new_branch, cx);
                let thread_id = thread_id.clone();
                details
                    .update(cx, |details, cx| {
                        details.run_git(thread_id, request, window, cx)
                    })
                    .ok();
            }
        };
        dialog
            .size(DialogSize::Lg)
            .title(m::git_commit_title())
            .description(m::git_commit_description())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label(m::git_commit_branch()))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .text_xs()
                            .font_family(mono.clone())
                            .child(status.branch.clone().unwrap_or_else(|| "—".into()))
                            .when(on_default_branch(&status), |this| {
                                this.child(
                                    div()
                                        .text_color(colors.status_waiting)
                                        .child(status.default_branch.clone().unwrap_or_default()),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(label(m::git_commit_files()))
                            .when_some(total, |this, total| {
                                this.child(label(m::git_commit_files_count(selected, total)))
                            }),
                    )
                    .child(
                        div()
                            .id("commit-files")
                            .flex()
                            .flex_col()
                            .max_h(px(224.))
                            .overflow_y_scroll()
                            .rounded_lg()
                            .border_1()
                            .border_color(colors.border)
                            .p_1()
                            .children(rows),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label(m::git_commit_message()))
                    .child(
                        div()
                            .rounded_lg()
                            .border_1()
                            .border_color(colors.input)
                            .px_2p5()
                            .py_1p5()
                            .text_sm()
                            .child(text_area(&state.message).appearance(false)),
                    ),
            )
            .footer(
                DialogButton::new("commit-cancel")
                    .label(m::cancel())
                    .variant(DialogButtonVariant::Outline)
                    .close(),
            )
            .footer(
                DialogButton::new("commit-new-branch")
                    .label(m::git_commit_new_branch())
                    .variant(DialogButtonVariant::Outline)
                    .disabled(selected == 0)
                    .on_click(commit(true))
                    .close(),
            )
            .footer(
                DialogButton::new("commit-confirm")
                    .label(m::git_commit())
                    .disabled(selected == 0)
                    .on_click(commit(false))
                    .close(),
            )
    });
}

fn checkbox(checked: bool, colors: &Colors) -> Div {
    div()
        .flex()
        .flex_none()
        .size(px(14.))
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .border_1()
        .border_color(if checked {
            colors.primary
        } else {
            colors.input
        })
        .when(checked, |this| {
            this.bg(colors.primary).child(
                Icon::new(IconName::Check)
                    .size(px(10.))
                    .text_color(colors.primary_foreground),
            )
        })
}
