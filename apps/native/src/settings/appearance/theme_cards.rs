use super::*;

impl SettingsView {
    pub(super) fn theme_card(
        &self,
        theme: ThemeDefinition,
        card: Option<String>,
        picked: Vec<ThemeAppearance>,
        custom: bool,
        initial: ThemeAppearance,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let modes = theme_modes(&theme);
        let orbs = div()
            .flex()
            .min_h_16()
            .items_center()
            .justify_center()
            .gap_2p5()
            .px_3()
            .pt_3()
            .children(modes.iter().map(|&half| {
                let card = card.clone();
                self.orb_button(&theme, half, picked.contains(&half), cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let card = card.clone();
                        this.change_look(cx, |look| set_half(look, half, card.as_deref()))
                    }))
            }));
        let tooltip = if modes.len() > 1 {
            m::use_theme_both()
        } else if modes.first() == Some(&ThemeAppearance::Light) {
            m::use_theme_light_only()
        } else {
            m::use_theme_dark_only()
        };
        let chosen = theme.clone();
        let use_theme = move |look: &mut Appearance| {
            let next = if custom {
                choose_card(&look.selection(), &chosen)
            } else {
                choose_theme(&chosen.id)
            };
            look.set_selection(next);
        };
        let actions = self.card_actions(&theme, None, custom, initial, cx);
        self.card_frame(
            SharedString::from(format!("theme-card-{}", theme.id)),
            theme.label.clone(),
            orbs.into_any_element(),
            actions,
            tooltip,
            cx.listener(move |this, _, _, cx| this.change_look(cx, &use_theme)),
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn card_frame(
        &self,
        id: SharedString,
        name: String,
        orbs: AnyElement,
        actions: Vec<AnyElement>,
        tooltip: String,
        on_use: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.hoshi();
        let hover = colors.muted.opacity(0.4);
        div()
            .id(id)
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_xl()
            .border_1()
            .border_color(colors.border.opacity(0.7))
            .bg(colors.card.opacity(0.6))
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .on_click(on_use)
            .child(orbs)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .pt_2()
                    .pb(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(name),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(2.))
                            .children(actions),
                    ),
            )
            .with_tooltip(Tooltip::new(tooltip))
    }

    pub(super) fn orb_button(
        &self,
        theme: &ThemeDefinition,
        half: ThemeAppearance,
        picked: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let colors = cx.hoshi();
        let swatch = theme_colors_for(theme, half)
            .cloned()
            .unwrap_or_else(|| theme.colors.clone());
        div()
            .id(SharedString::from(format!(
                "orb-{}-{}",
                theme.id,
                half.as_str()
            )))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(68.))
            .p_1p5()
            .rounded_full()
            .cursor_pointer()
            .child(orb(&swatch, half, px(56.)))
            .when(picked, |this| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .rounded_full()
                        .border_2()
                        .border_color(colors.ring),
                )
                .child(
                    div()
                        .absolute()
                        .right(px(2.))
                        .bottom(px(2.))
                        .size_5()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .border_1()
                        .border_color(colors.border.opacity(0.7))
                        .bg(colors.background)
                        .text_color(colors.foreground)
                        .shadow_sm()
                        .child(Icon::new(appearance_icon(half)).size_3()),
                )
            })
    }

    pub(super) fn card_actions(
        &self,
        theme: &ThemeDefinition,
        collection: Option<Vec<ThemeDefinition>>,
        custom: bool,
        initial: ThemeAppearance,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let action = |id: &str, icon: IconName, label: String| {
            Button::new(SharedString::from(format!("{id}-{}", theme.id)))
                .icon(icon)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::IconXs)
                .tooltip(label)
        };
        let mut actions = Vec::new();
        let source = theme.clone();
        actions.push(
            action("duplicate", IconName::Copy, m::duplicate_theme())
                .on_click(cx.listener(move |this, _, window, cx| {
                    let seed = (source.id != STANDARD_THEME_ID).then(|| source.id.clone());
                    let name = m::theme_copy_name(&source.label);
                    this.open_theme_editor(None, seed, Some(name), initial, window, cx);
                }))
                .into_any_element(),
        );
        if custom {
            let editing = theme.id.clone();
            actions.push(
                action("edit", IconName::Pencil, m::edit_theme())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_theme_editor(
                            Some(editing.clone()),
                            None,
                            None,
                            initial,
                            window,
                            cx,
                        )
                    }))
                    .into_any_element(),
            );
            let exported = theme.clone();
            actions.push(
                action("export", IconName::Upload, m::export_theme())
                    .on_click(cx.listener(move |this, _, _, cx| this.export_theme(&exported, cx)))
                    .into_any_element(),
            );
            let removed = theme.clone();
            let group = collection.unwrap_or_else(|| vec![theme.clone()]);
            let label = if group.len() > 1 {
                m::remove_themes()
            } else {
                m::remove_theme()
            };
            actions.push(
                action("remove", IconName::Trash, label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ask_removal(&removed, group.clone(), cx)
                    }))
                    .into_any_element(),
            );
        }
        actions
    }

    pub(super) fn collection_card(
        &self,
        themes: Vec<ThemeDefinition>,
        picked: &dyn Fn(Option<&str>) -> Vec<ThemeAppearance>,
        initial: ThemeAppearance,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let labels = variant_labels(&themes);
        let shown = themes
            .iter()
            .position(|theme| !picked(Some(&theme.id)).is_empty())
            .unwrap_or(0);
        let theme = themes[shown].clone();
        let name = theme
            .collection
            .as_ref()
            .map(|collection| collection.label.clone())
            .unwrap_or_else(|| theme.label.clone());
        let colors = cx.hoshi().clone();
        let groups = ThemeAppearance::ALL.map(|half| {
            let options: Vec<(usize, ThemeDefinition)> = themes
                .iter()
                .enumerate()
                .filter(|(_, candidate)| theme_modes(candidate).contains(&half))
                .map(|(index, candidate)| (index, candidate.clone()))
                .collect();
            let selected = options
                .iter()
                .find(|(_, option)| picked(Some(&option.id)).contains(&half))
                .or(options.first())
                .cloned();
            let variants: Vec<(String, String, bool)> = options
                .iter()
                .map(|(index, option)| {
                    (
                        option.id.clone(),
                        labels[*index].clone(),
                        picked(Some(&option.id)).contains(&half),
                    )
                })
                .collect();
            (half, variants, selected)
        });
        let card_key = theme
            .collection
            .as_ref()
            .map(|collection| collection.id.clone())
            .unwrap_or_else(|| theme.id.clone());
        let fanned = self
            .looks
            .fan
            .as_ref()
            .filter(|(key, _)| *key == card_key)
            .map(|(_, half)| *half);
        let mut layers: Vec<AnyElement> = Vec::new();
        let mut fans: Vec<AnyElement> = Vec::new();
        for (half, variants, selected) in groups {
            let Some((index, selected)) = selected else {
                continue;
            };
            let offset = match half {
                ThemeAppearance::Light => -52.,
                ThemeAppearance::Dark => 52.,
            };
            let is_picked = picked(Some(&selected.id)).contains(&half);
            let count = variants.len();
            let label = labels[index].clone();
            let chosen = selected.id.clone();
            let hovered = card_key.clone();
            layers.push(
                self.orb_button(&selected, half, is_picked, cx)
                    .absolute()
                    .top_2()
                    .left(relative(0.5))
                    .ml(px(offset - 34.))
                    .when(count > 1, |this| {
                        this.on_hover(cx.listener(move |this, over: &bool, _, cx| {
                            if *over {
                                this.looks.fan = Some((hovered.clone(), half));
                                cx.notify();
                            }
                        }))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let chosen = chosen.clone();
                        this.change_look(cx, |look| set_half(look, half, Some(&chosen)))
                    }))
                    .into_any_element(),
            );
            layers.push(
                div()
                    .absolute()
                    .bottom_0()
                    .left(relative(0.5))
                    .ml(px(offset - 48.))
                    .w_24()
                    .flex()
                    .justify_center()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(div().min_w_0().truncate().child(label))
                    .when(count > 1, |this| {
                        this.child(
                            div()
                                .flex_none()
                                .rounded_full()
                                .px_1()
                                .bg(colors.muted)
                                .text_color(colors.muted_foreground)
                                .text_size(rems(0.625))
                                .child(format!("+{}", count - 1)),
                        )
                    })
                    .into_any_element(),
            );
            if fanned != Some(half) || count < 2 {
                continue;
            }
            let swatches: Vec<(String, ThemeColors)> = themes
                .iter()
                .filter(|candidate| variants.iter().any(|(id, _, _)| *id == candidate.id))
                .map(|candidate| {
                    (
                        candidate.id.clone(),
                        theme_colors_for(candidate, half)
                            .cloned()
                            .unwrap_or_else(|| candidate.colors.clone()),
                    )
                })
                .collect();
            let still = cx.reduce_motion();
            for (position, (id, _, checked)) in variants.into_iter().enumerate() {
                let progress = position as f32 / (count - 1) as f32 - 0.5;
                let (x, y) = (offset + progress * 68., progress.abs() * 10.);
                let swatch = swatches
                    .iter()
                    .find(|(candidate, _)| *candidate == id)
                    .map(|(_, colors)| colors.clone());
                let chosen = id.clone();
                let option = div()
                    .id(SharedString::from(format!(
                        "variant-{id}-{}",
                        half.as_str()
                    )))
                    .absolute()
                    .top_1()
                    .left(relative(0.5))
                    .size_7()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(colors.background)
                    .shadow_sm()
                    .cursor_pointer()
                    .border(if checked { px(2.) } else { px(1.) })
                    .border_color(if checked {
                        colors.ring
                    } else {
                        colors.border.opacity(0.7)
                    })
                    .children(swatch.map(|swatch| orb(&swatch, half, px(20.))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let chosen = chosen.clone();
                        this.change_look(cx, |look| set_half(look, half, Some(&chosen)))
                    }));
                let placed = move |option: Stateful<Div>, delta: f32| {
                    let at_x = offset + (x - offset) * delta;
                    let at_y = 28. + (y - 28.) * delta;
                    option.opacity(delta).ml(px(at_x - 14.)).mt(px(at_y))
                };
                fans.push(if still {
                    placed(option, 1.).into_any_element()
                } else {
                    option
                        .with_animation(
                            SharedString::from(format!("fan-{id}-{}", half.as_str())),
                            Animation::new(Duration::from_millis(200))
                                .with_easing(ease_out_quint()),
                            placed,
                        )
                        .into_any_element()
                });
            }
        }
        let leaving = card_key.clone();
        let orbs = div()
            .id(SharedString::from(format!("orbs-{card_key}")))
            .relative()
            .h_24()
            .w_full()
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                let open_here = this
                    .looks
                    .fan
                    .as_ref()
                    .is_some_and(|(key, _)| *key == leaving);
                if !*over && open_here {
                    this.looks.fan = None;
                    cx.notify();
                }
            }))
            .children(layers)
            .children(fans);
        let defaults = themes.clone();
        let actions = self.card_actions(&theme, Some(themes.clone()), true, initial, cx);
        self.card_frame(
            SharedString::from(format!("collection-card-{}", theme.id)),
            name,
            orbs.into_any_element(),
            actions,
            m::use_first_variants(),
            cx.listener(move |this, _, _, cx| {
                let defaults = defaults.clone();
                this.change_look(cx, move |look| {
                    for half in ThemeAppearance::ALL {
                        if let Some(first) = defaults
                            .iter()
                            .find(|candidate| theme_modes(candidate).contains(&half))
                        {
                            set_half(look, half, Some(&first.id));
                        }
                    }
                })
            }),
            cx,
        )
    }
}
