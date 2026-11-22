use super::*;

impl SettingsView {
    pub(super) fn color_schemes(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let appearance = look::appearance(cx).clone();
        let owners = half_owners(&appearance.selection(), &appearance.custom_themes);
        let scheme_colors =
            |half: ThemeAppearance| owner_colors(owners.get(half), half, &appearance.custom_themes);
        let light = scheme_colors(ThemeAppearance::Light);
        let dark = scheme_colors(ThemeAppearance::Dark);
        let tiles = [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ]
        .map(|preference| {
            let selected = preference == appearance.preference;
            let hover_bg = colors.muted.opacity(0.5);
            let hover_fg = colors.foreground;
            let frame = div()
                .relative()
                .h_20()
                .overflow_hidden()
                .rounded_md()
                .border_1()
                .border_color(colors.border);
            let preview = match preference {
                ThemePreference::Light => frame.child(wireframe(&light)),
                ThemePreference::Dark => frame.child(wireframe(&dark)),
                ThemePreference::System => frame
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .h_full()
                            .w_1_2()
                            .overflow_hidden()
                            .child(div().h_full().w(relative(2.)).child(wireframe(&light))),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .h_full()
                            .w_1_2()
                            .overflow_hidden()
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .h_full()
                                    .w(relative(2.))
                                    .child(wireframe(&dark)),
                            ),
                    ),
            };
            div()
                .id(SharedString::from(format!("color-scheme-{preference:?}")))
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_2()
                .p_2()
                .rounded_xl()
                .border_1()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .map(|this| {
                    if selected {
                        this.border_color(colors.primary)
                            .text_color(colors.foreground)
                    } else {
                        this.border_color(colors.border)
                            .text_color(colors.muted_foreground)
                            .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.change_look(cx, |look| look.preference = preference)
                }))
                .child(preview)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .px_1()
                        .child(Icon::new(scheme_icon(preference)).size_4())
                        .child(scheme_label(preference)),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(section_title(m::color_scheme()))
            .child(div().flex().gap_3().children(tiles))
    }

    pub(super) fn themes_section(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let appearance = look::appearance(cx).clone();
        let custom = appearance.custom_themes.clone();
        let owners = half_owners(&appearance.selection(), &custom);
        let picked = move |card: Option<&str>| -> Vec<ThemeAppearance> {
            ThemeAppearance::ALL
                .into_iter()
                .filter(|half| owners.get(*half) == card)
                .collect()
        };
        let initial = look::resolved(window, cx);
        let mut cards: Vec<AnyElement> = built_in_themes()
            .iter()
            .cloned()
            .map(|theme| {
                let card = (theme.id != STANDARD_THEME_ID).then(|| theme.id.clone());
                let modes = picked(card.as_deref());
                self.theme_card(theme, card, modes, false, initial, cx)
                    .into_any_element()
            })
            .collect();
        for group in collections_of(&custom) {
            if group.len() > 1 {
                cards.push(
                    self.collection_card(group, &picked, initial, cx)
                        .into_any_element(),
                );
            } else {
                let theme = group[0].clone();
                let modes = picked(Some(&theme.id));
                let card = Some(theme.id.clone());
                cards.push(
                    self.theme_card(theme, card, modes, true, initial, cx)
                        .into_any_element(),
                );
            }
        }
        let colors = cx.hoshi().clone();
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .min_h_7()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_4()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(m::themes()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("create-theme")
                                    .icon(IconName::Brush)
                                    .label(m::create_theme())
                                    .variant(ButtonVariant::Outline)
                                    .size(ButtonSize::Xs)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let seed = this.current_owner(initial, cx);
                                        this.open_theme_editor(
                                            None, seed, None, initial, window, cx,
                                        )
                                    })),
                            )
                            .child(
                                Button::new("add-theme")
                                    .icon(IconName::Plus)
                                    .label(m::add_theme())
                                    .variant(ButtonVariant::Outline)
                                    .size(ButtonSize::Xs)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_theme_import(window, cx)
                                    })),
                            ),
                    ),
            )
            .when_some(self.looks.notice.clone(), |this, notice| {
                this.child(
                    div()
                        .px_4()
                        .text_xs()
                        .text_color(colors.destructive)
                        .child(notice),
                )
            })
            .child(div().grid().grid_cols(3).gap_2().children(cards))
    }

    /// The theme that paints `half` now, to start a new theme from.
    pub(super) fn current_owner(&self, half: ThemeAppearance, cx: &App) -> Option<String> {
        let appearance = look::appearance(cx);
        half_owners(&appearance.selection(), &appearance.custom_themes)
            .get(half)
            .map(str::to_owned)
    }

    pub(super) fn export_theme(&mut self, theme: &ThemeDefinition, cx: &mut Context<Self>) {
        let contents = serialize_theme_file(theme);
        let name = format!("{}.json", theme.id);
        let picked = cx.prompt_for_new_path(&home_dir(), Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = picked.await else {
                return;
            };
            let written = cx
                .background_spawn(async move { std::fs::write(path, contents) })
                .await;
            this.update(cx, |this, cx| {
                this.looks.notice = written.is_err().then(m::export_theme_failed);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
