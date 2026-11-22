use super::*;

impl SettingsView {
    pub(super) fn ask_removal(
        &mut self,
        theme: &ThemeDefinition,
        group: Vec<ThemeDefinition>,
        cx: &mut Context<Self>,
    ) {
        let name = if group.len() > 1 {
            theme
                .collection
                .as_ref()
                .map(|collection| collection.label.clone())
                .unwrap_or_else(|| theme.label.clone())
        } else {
            theme.label.clone()
        };
        let chosen = if group.len() > 1 {
            BTreeSet::new()
        } else {
            BTreeSet::from([theme.id.clone()])
        };
        self.looks.removal = Some(Removal {
            themes: group,
            name,
            chosen,
        });
        cx.notify();
    }

    pub(super) fn confirm_removal(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(removal) = self.looks.removal.take() else {
            return;
        };
        let ids: Vec<String> = removal.chosen.into_iter().collect();
        if ids.is_empty() {
            return;
        }
        self.change_look(cx, |look| {
            let selection = selection_without(&look.selection(), &ids);
            look.custom_themes = remove_themes(&look.custom_themes, &ids);
            look.set_selection(selection);
        });
    }

    pub(super) fn removal_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let removal = self.looks.removal.as_ref()?;
        let colors = cx.hoshi().clone();
        let collection = removal.themes.len() > 1;
        let count = removal.chosen.len();
        let (title, description) = if collection {
            (
                m::remove_themes_from_confirm(&removal.name),
                m::remove_themes_description(),
            )
        } else {
            (
                m::remove_theme_confirm(&removal.name),
                m::remove_theme_description(),
            )
        };
        let confirm_label = if !collection {
            m::remove_theme()
        } else if count > 0 {
            m::remove_selected_count(count)
        } else {
            m::remove_selected()
        };
        let options: Vec<AnyElement> = removal
            .themes
            .iter()
            .map(|theme| {
                let checked = removal.chosen.contains(&theme.id);
                let id = theme.id.clone();
                div()
                    .id(SharedString::from(format!("remove-{}", theme.id)))
                    .relative()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .min_h(px(112.))
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .cursor_pointer()
                    .map(|this| {
                        if checked {
                            this.border_color(colors.ring)
                                .bg(colors.accent.opacity(0.4))
                        } else {
                            this.border_color(colors.border.opacity(0.7))
                                .bg(colors.muted.opacity(0.2))
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(removal) = &mut this.looks.removal
                            && !removal.chosen.remove(&id)
                        {
                            removal.chosen.insert(id.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .children(theme_modes(theme).into_iter().map(|half| {
                                let swatch = theme_colors_for(theme, half)
                                    .cloned()
                                    .unwrap_or_else(|| theme.colors.clone());
                                orb(&swatch, half, px(40.))
                            })),
                    )
                    .child(
                        div()
                            .max_w_full()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(theme.label.clone()),
                    )
                    .when(checked, |this| {
                        this.child(
                            div()
                                .absolute()
                                .top_2()
                                .right_2()
                                .child(Icon::new(IconName::Check).size_4()),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        Some(
            Dialog::new("remove-theme")
                .alert()
                .open(true)
                .size(DialogSize::Md)
                .title(title)
                .description(description)
                .on_open_change(cx.listener(|this, change: &OpenChange, _, cx| {
                    if !change.open {
                        this.looks.removal = None;
                        cx.notify();
                    }
                }))
                .when(collection, |dialog| {
                    dialog.child(
                        div()
                            .id("remove-themes-list")
                            .max_h_72()
                            .overflow_y_scroll()
                            .grid()
                            .grid_cols(2)
                            .gap_2()
                            .children(options),
                    )
                })
                .footer(
                    DialogButton::new("remove-cancel")
                        .label(m::cancel())
                        .variant(DialogButtonVariant::Outline)
                        .close(),
                )
                .footer(
                    DialogButton::new("remove-confirm")
                        .label(confirm_label)
                        .variant(DialogButtonVariant::Destructive)
                        .disabled(count == 0)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.confirm_removal(window, cx)),
                        ),
                )
                .into_any_element(),
        )
    }
}
