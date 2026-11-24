use super::*;
use rows::icon_button;

impl ThreadDetails {
    /// The folder the thread works in, with the editor that opens it at the end.
    pub(super) fn folder_row(
        &self,
        path: &str,
        badge: Option<String>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder = path
            .split(['\\', '/'])
            .rfind(|part| !part.is_empty())
            .unwrap_or(path)
            .to_owned();
        let reveal = PathBuf::from(path);
        let base = div()
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
            });
        let Some(editor) = self.open_target() else {
            return base
                .child(
                    icon_button(
                        "details-reveal",
                        icon(IconName::FolderOpen, colors),
                        false,
                        colors,
                    )
                    .ml_auto()
                    .on_click(move |_, _, cx| cx.reveal_path(&reveal))
                    .with_tooltip(Tooltip::new(m::show_in_folder())),
                )
                .into_any_element();
        };
        let installed = self.editors.clone().unwrap_or_default();
        let this = cx.weak_entity();
        let arrow_colors = colors.clone();
        let menu_colors = colors.clone();
        let menu = Popover::new("details-editor-menu")
            .anchor(self.editor_anchor.clone())
            .align(Align::End)
            .match_trigger_width()
            .trigger(move |open| {
                icon_button(
                    "details-editor-arrow",
                    icon(IconName::ChevronDown, &arrow_colors),
                    open,
                    &arrow_colors,
                )
                .debug_selector(|| "details-editor-arrow".into())
            })
            .menu(move |menu, _, _| {
                let reveal = reveal.clone();
                menu.items(installed.into_iter().enumerate().map(|(index, choice)| {
                    let this = this.clone();
                    MenuOption::new(("details-editor", index), editor_label(choice))
                        .leading(div().mr_1().child(editor_icon(choice, &menu_colors)))
                        .when(choice == editor, |option| option.shortcut(open_shortcut()))
                        .on_select(move |_, cx| {
                            this.update(cx, |this, cx| this.open_in(choice, cx)).ok();
                        })
                }))
                .separator()
                .item(
                    MenuOption::new("details-reveal", m::show_in_folder())
                        .icon(IconName::FolderOpen)
                        .on_select(move |_, cx| cx.reveal_path(&reveal)),
                )
            });
        base.debug_selector(|| "details-editor-row".into())
            .child(
                icon_button(
                    "details-open-editor",
                    editor_icon(editor, colors),
                    false,
                    colors,
                )
                .ml_auto()
                .on_click(cx.listener(move |this, _, _, cx| this.open_in(editor, cx)))
                .with_tooltip(Tooltip::new(m::open_in_editor(editor_label(editor)))),
            )
            .child(menu)
            .child(self.editor_anchor.area())
            .into_any_element()
    }

    /// The branch the thread works on, whether its checkout is clean, and `actions` at the end.
    pub(super) fn branch_row(
        &self,
        branch: String,
        thread: &Thread,
        clean: bool,
        actions: Option<AnyElement>,
        colors: &Colors,
    ) -> AnyElement {
        let anchored = actions.is_some();
        row_static("details-branch", colors)
            .debug_selector(|| "details-branch-row".into())
            .relative()
            .when(anchored, |this| this.pr_1())
            .child(icon(
                if thread.worktree_path.is_some() {
                    IconName::GitFork
                } else {
                    IconName::GitBranch
                },
                colors,
            ))
            .child(div().flex_1().min_w_0().truncate().child(branch))
            .when(clean, |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(colors.muted_foreground)
                        .child(m::checkout_clean()),
                )
            })
            .children(actions)
            .when(anchored, |this| this.child(self.branch_anchor.area()))
            .into_any_element()
    }
}
