use super::*;

impl Sidebar {
    pub(super) fn header(&mut self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        let has_projects = self.workspace.read(cx).model.projects().next().is_some();
        let searching = !self.search.read(cx).value().is_empty();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .relative()
            .px_2()
            .pb(px(10.))
            .border_b_1()
            .border_color(colors.sidebar_border)
            .bg(colors.header)
            .child(starfield(colors))
            .child(
                div()
                    .h_10()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("toggle-sidebar")
                            .icon(IconName::PanelLeft)
                            .size(ButtonSize::IconSm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(Tooltip::new(m::toggle_sidebar()).side(Side::Bottom))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::Toggle))),
                    )
                    .child(
                        div()
                            .id("sidebar-drag")
                            .drag_region()
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .px_1()
                            .child(
                                svg()
                                    .path(brand::WORDMARK)
                                    .h(px(12.))
                                    .w(px(39.))
                                    .text_color(colors.sidebar_foreground),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .id("search-threads")
                            .flex_1()
                            .min_w_0()
                            .h_8()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .rounded_lg()
                            .text_sm()
                            .text_color(colors.sidebar_foreground.opacity(0.7))
                            .hover(|this| this.bg(colors.sidebar_accent.opacity(0.6)))
                            .child(Icon::new(IconName::Search).size_4().flex_none())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(text_input(&self.search).appearance(false)),
                            )
                            .when(!searching, |this| {
                                this.child(
                                    div()
                                        .id("palette-shortcut")
                                        .flex_none()
                                        .cursor_pointer()
                                        .on_click(cx.listener(|_, _, _, cx| {
                                            cx.emit(SidebarEvent::OpenPalette(
                                                palette::Mode::Commands,
                                            ))
                                        }))
                                        .child(Kbd::new(palette_shortcut()))
                                        .with_tooltip(
                                            Tooltip::new(m::palette_label()).side(Side::Bottom),
                                        ),
                                )
                            }),
                    )
                    .child(self.sort_menu(cx))
                    .child(
                        Button::new("add-project")
                            .icon(IconName::FolderPlus)
                            .size(ButtonSize::IconSm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(Tooltip::new(m::add_project()).side(Side::Bottom))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(SidebarEvent::OpenPalette(palette::Mode::AddProject))
                            })),
                    )
                    .child(
                        Button::new("new-thread")
                            .icon(IconName::SquarePen)
                            .size(ButtonSize::IconSm)
                            .variant(ButtonVariant::Ghost)
                            .tooltip(Tooltip::new(m::new_thread()).side(Side::Bottom))
                            .disabled(!has_projects)
                            .on_click(cx.listener(|this, _, _, cx| this.new_thread(cx))),
                    ),
            )
    }

    pub(super) fn sort_menu(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let (sort, filter) = (self.prefs.sort, self.prefs.filter);
        let narrowing = (filter != Filter::All).then_some(cx.hoshi().sidebar_primary);
        Popover::new("sort-menu")
            .align(Align::End)
            .w(px(176.))
            .open(self.sort_open)
            .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                this.sort_open = change.open;
                cx.notify();
            }))
            .trigger(move |open| {
                Button::new("sort-trigger")
                    .icon(IconName::SlidersHorizontal)
                    .size(ButtonSize::IconSm)
                    .variant(ButtonVariant::Ghost)
                    .selected(open)
                    .when_some(narrowing, |button, color| button.text_color(color))
                    .tooltip(Tooltip::new(m::filter_threads()).side(Side::Bottom))
            })
            .menu(move |menu, _, _| {
                let sorts = [
                    (Sort::Activity, m::sort_by_activity()),
                    (Sort::Created, m::sort_by_created()),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (option, label))| {
                    let this = this.clone();
                    MenuOption::radio(("sort-option", index), label, option == sort).on_select(
                        move |_, cx| {
                            this.update(cx, |this, cx| {
                                this.prefs.sort = option;
                                this.save_prefs(cx);
                                cx.notify();
                            })
                            .ok();
                        },
                    )
                });
                let filters = [
                    (Filter::All, m::filter_all_threads()),
                    (Filter::Worktrees, m::filter_worktrees()),
                    (Filter::PullRequests, m::filter_with_pull_request()),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (option, label))| {
                    let this = this.clone();
                    MenuOption::radio(("filter-option", index), label, option == filter).on_select(
                        move |_, cx| {
                            this.update(cx, |this, cx| {
                                this.prefs.filter = option;
                                this.save_prefs(cx);
                                cx.notify();
                            })
                            .ok();
                        },
                    )
                });
                menu.heading(m::sort_threads())
                    .items(sorts)
                    .separator()
                    .heading(m::show_threads())
                    .items(filters)
            })
    }

    pub(super) fn view_heading(
        &mut self,
        icon: IconName,
        title: String,
        lone_project: Option<(String, String)>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let has_projects = self.workspace.read(cx).model.projects().next().is_some();
        div()
            .h_8()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl_2()
            .pr_1()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .when(has_projects, |this| {
                let back = lone_project
                    .as_ref()
                    .map(|(_, title)| title.clone())
                    .unwrap_or_else(m::projects);
                let lone = lone_project.as_ref().map(|(id, _)| id.clone());
                this.child(
                    div()
                        .id("back-to-project")
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .min_w_0()
                        .max_w(relative(0.5))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(colors.muted_foreground)
                        .cursor_pointer()
                        .hover(|this| this.text_color(colors.foreground))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(lone) = &lone {
                                this.project_id = Some(lone.clone());
                            }
                            this.set_view(View::Project, cx)
                        }))
                        .child(Icon::new(IconName::ArrowLeft).size(px(14.)).flex_none())
                        .child(div().truncate().child(back)),
                )
                .child(
                    div()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(colors.muted_foreground.opacity(0.6))
                        .child("/"),
                )
            })
            .when(!has_projects, |this| {
                this.child(
                    Icon::new(icon)
                        .size(px(14.))
                        .text_color(colors.muted_foreground),
                )
            })
            .child(div().truncate().child(title))
            .into_any_element()
    }

    pub(super) fn project_heading(
        &mut self,
        project: Option<&Project>,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(project) = project else {
            return div().into_any_element();
        };
        let icon = self.icon(&project.id, cx);
        let this = cx.weak_entity();
        let id = project.id.clone();
        div()
            .h_8()
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .pl_2()
            .pr_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(match icon {
                        Some(path) => img(path)
                            .size(px(14.))
                            .rounded(px(3.))
                            .object_fit(ObjectFit::Contain)
                            .into_any_element(),
                        None => Icon::new(IconName::Folder)
                            .size(px(14.))
                            .text_color(colors.muted_foreground)
                            .into_any_element(),
                    })
                    .child(div().truncate().child(project.title.clone())),
            )
            .child(
                Popover::new("project-actions")
                    .align(Align::End)
                    .w(px(192.))
                    .trigger(|open| {
                        Button::new("project-actions-trigger")
                            .icon(IconName::Ellipsis)
                            .size(ButtonSize::IconXs)
                            .variant(ButtonVariant::Ghost)
                            .selected(open)
                            .tooltip(m::project_actions())
                    })
                    .menu(move |menu, _, _| project_menu(menu, this, id)),
            )
            .into_any_element()
    }

    pub(super) fn footer(&self, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .px_2()
            .py(px(6.))
            .border_t_1()
            .border_color(colors.sidebar_border)
            .child(
                Button::new("settings")
                    .icon(IconName::Settings)
                    .size(ButtonSize::IconSm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(m::settings())
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::OpenSettings))),
            )
            .child(
                Button::new("usage")
                    .icon(IconName::ChartColumn)
                    .size(ButtonSize::IconSm)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(m::usage())
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::OpenUsage))),
            )
            .child(div().ml_auto().child(self.updates.clone()))
    }
}
