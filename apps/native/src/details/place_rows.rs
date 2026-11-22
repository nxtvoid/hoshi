use super::*;

impl ThreadDetails {
    pub(super) fn folder_row(
        &self,
        path: &str,
        badge: Option<String>,
        colors: &Colors,
    ) -> impl IntoElement {
        let folder = path
            .split(['\\', '/'])
            .rfind(|part| !part.is_empty())
            .unwrap_or(path)
            .to_owned();
        let hover = colors.foreground.opacity(0.05);
        let reveal = PathBuf::from(path);
        div()
            .id("details-folder")
            .flex()
            .flex_none()
            .h_8()
            .min_w_0()
            .items_center()
            .gap(px(10.))
            .rounded_lg()
            .pr_1()
            .pl(px(10.))
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(colors.foreground.opacity(0.8))
            .child(icon(IconName::Folder, colors))
            .child(
                div()
                    .id("details-folder-name")
                    .min_w_0()
                    .truncate()
                    .child(folder)
                    .with_tooltip(Tooltip::new(path.to_owned())),
            )
            .when_some(badge, |this, badge| {
                this.child(
                    div()
                        .flex_none()
                        .rounded_full()
                        .border_1()
                        .border_color(colors.border)
                        .px(px(6.))
                        .text_size(px(10.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(colors.muted_foreground)
                        .child(badge),
                )
            })
            .child(
                div()
                    .id("details-reveal")
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .size_6()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .child(icon(IconName::FolderOpen, colors))
                    .on_click(move |_, _, cx| cx.reveal_path(&reveal))
                    .with_tooltip(Tooltip::new(m::show_in_folder())),
            )
    }

    pub(super) fn editor_row(
        &self,
        editor: Editor,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let installed = self.editors.clone().unwrap_or_default();
        let this = cx.weak_entity();
        let arrow_colors = colors.clone();
        let menu_colors = colors.clone();
        let menu = Popover::new("details-editor-menu")
            .anchor(self.editor_anchor.clone())
            .align(Align::End)
            .match_trigger_width()
            .trigger(move |open| {
                split_arrow("details-editor-arrow", open, &arrow_colors)
                    .debug_selector(|| "details-editor-arrow".into())
            })
            .menu(move |menu, _, _| {
                menu.items(installed.into_iter().enumerate().map(|(index, choice)| {
                    let this = this.clone();
                    MenuOption::new(("details-editor", index), editor_label(choice))
                        .leading(div().mr_1().child(editor_icon(choice, &menu_colors)))
                        .when(choice == editor, |option| option.shortcut(open_shortcut()))
                        .on_select(move |_, cx| {
                            this.update(cx, |this, cx| this.open_in(choice, cx)).ok();
                        })
                }))
            });
        split_row(
            "details-editor",
            split_primary("details-open-editor", colors)
                .child(editor_icon(editor, colors))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(m::open_in_editor(editor_label(editor))),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.open_in(editor, cx))),
            Some(menu.into_any_element()),
            colors,
        )
        .debug_selector(|| "details-editor-row".into())
        .child(self.editor_anchor.area())
    }

    pub(super) fn branch_row(
        &self,
        branch: String,
        thread: &Thread,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mono = cx.theme().mono_font_family.clone();
        let worktree = thread.worktree_path.is_some();
        let build = {
            let colors = colors.clone();
            let branch = branch.clone();
            move |open: bool, enabled: bool| {
                let base = if enabled {
                    row("details-branch", &colors)
                } else {
                    row_static("details-branch", &colors).opacity(0.6)
                };
                base.when(open, |this| this.bg(colors.foreground.opacity(0.05)))
                    .child(icon(
                        if worktree {
                            IconName::GitFork
                        } else {
                            IconName::GitBranch
                        },
                        &colors,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono)
                            .text_xs()
                            .child(branch),
                    )
                    .child(icon(IconName::ChevronDown, &colors))
            }
        };
        if thread.status == ThreadStatus::Working {
            return build(false, false)
                .with_tooltip(Tooltip::new(m::error_thread_busy()))
                .into_any_element();
        }
        let others: Option<Vec<String>> = self.branches.as_ref().map(|branches| {
            branches
                .iter()
                .filter(|name| **name != branch)
                .cloned()
                .collect()
        });
        let this = cx.weak_entity();
        let muted = colors.muted_foreground;
        div()
            .flex()
            .flex_col()
            .child(
                Popover::new("details-branch-menu")
                    .align(Align::End)
                    .open(self.branch_menu_open)
                    .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                        this.set_branch_menu(change.open, cx)
                    }))
                    .trigger(move |open| build(open, true))
                    .menu(move |menu, _, _| {
                        let empty = others.as_ref().is_some_and(Vec::is_empty);
                        menu.max_list_height(px(288.))
                            .heading(m::switch_branch())
                            .when(empty, |menu| {
                                menu.child(
                                    div()
                                        .px_2()
                                        .py_1p5()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(m::no_other_branches()),
                                )
                            })
                            .items(
                                others
                                    .into_iter()
                                    .flatten()
                                    .enumerate()
                                    .map(|(index, name)| {
                                        let this = this.clone();
                                        MenuOption::new(
                                            ("details-branch-option", index),
                                            name.clone(),
                                        )
                                        .mono()
                                        .on_select(
                                            move |_, cx| {
                                                let name = name.clone();
                                                this.update(cx, |this, cx| {
                                                    this.switch_branch(name, cx)
                                                })
                                                .ok();
                                            },
                                        )
                                    }),
                            )
                    }),
            )
            .into_any_element()
    }
}
