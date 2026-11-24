use super::*;

impl SettingsView {
    pub(super) fn typography(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let look = look::appearance(cx).clone();
        let separate = look.separate_fonts;
        let mut group = SettingsGroup::new()
            .title(m::typography())
            .child(self.font_row(
                FontSlot::Interface,
                m::interface_font(),
                m::interface_font_description(),
                (!separate).then(|| prompt_preview(cx).into_any_element()),
                cx,
            ));
        if separate {
            group = group.child(self.font_row(
                FontSlot::Composer,
                m::prompt_font(),
                m::prompt_font_description(),
                Some(prompt_preview(cx).into_any_element()),
                cx,
            ));
        }
        let code_preview = div()
            .flex()
            .flex_col()
            .child(code_preview(cx))
            .when(!separate, |this| this.child(terminal_preview(cx)))
            .into_any_element();
        group = group.child(self.font_row(
            FontSlot::Code,
            if separate {
                m::code_font()
            } else {
                m::monospace_font()
            },
            if separate {
                m::code_font_description()
            } else {
                m::monospace_font_description()
            },
            Some(code_preview),
            cx,
        ));
        if separate {
            group = group.child(self.font_row(
                FontSlot::Terminal,
                m::terminal_font(),
                m::terminal_font_description(),
                Some(terminal_preview(cx).into_any_element()),
                cx,
            ));
        }
        let wrap_reset = (look.word_wrap != DEFAULT_APPEARANCE.word_wrap).then(|| {
            reset_button(
                "reset-word-wrap",
                m::reset_setting(m::word_wrap()),
                self.look_setter(cx, |look| look.word_wrap = DEFAULT_APPEARANCE.word_wrap),
            )
        });
        let this = cx.weak_entity();
        group = group.child(
            SettingsRow::new(m::word_wrap())
                .description(m::word_wrap_description())
                .when_some(wrap_reset, |row, reset| row.action(reset))
                .control(
                    Switch::new("settings-word-wrap")
                        .checked(look.word_wrap)
                        .on_change(move |on, _, cx| {
                            this.update(cx, |this, cx| {
                                this.change_look(cx, |look| look.word_wrap = on)
                            })
                            .ok();
                        }),
                ),
        );
        let smoothing_reset =
            (look.font_smoothing != DEFAULT_APPEARANCE.font_smoothing).then(|| {
                reset_button(
                    "reset-font-smoothing",
                    m::reset_setting(m::font_smoothing()),
                    self.look_setter(cx, |look| {
                        look.font_smoothing = DEFAULT_APPEARANCE.font_smoothing
                    }),
                )
            });
        let this = cx.weak_entity();
        group = group.child(
            SettingsRow::new(m::font_smoothing())
                .description(m::font_smoothing_description())
                .when_some(smoothing_reset, |row, reset| row.action(reset))
                .control(
                    Switch::new("settings-font-smoothing")
                        .checked(look.font_smoothing)
                        .on_change(move |on, _, cx| {
                            this.update(cx, |this, cx| {
                                this.change_look(cx, |look| look.font_smoothing = on)
                            })
                            .ok();
                        }),
                ),
        );
        let this = cx.weak_entity();
        let advanced = self.advanced_group(
            "typography",
            [SettingsRow::new(m::separate_fonts())
                .description(m::separate_fonts_description())
                .control(
                    Switch::new("settings-separate-fonts")
                        .checked(separate)
                        .on_change(move |on, _, cx| {
                            this.update(cx, |this, cx| {
                                this.change_look(cx, |look| look.separate_fonts = on)
                            })
                            .ok();
                        }),
                )
                .into_any_element()],
            cx,
        );
        group.child(advanced)
    }

    pub(super) fn font_row(
        &mut self,
        slot: FontSlot,
        title: String,
        description: String,
        preview: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> SettingsRow {
        let look = look::appearance(cx).clone();
        let defaults = &*DEFAULT_APPEARANCE;
        let (font, size, range) = slot_values(&look, slot);
        let (default_font, default_size, _) = slot_values(defaults, slot);
        let changed = font != default_font || size != default_size;
        let reset = changed.then(|| {
            reset_button(
                SharedString::from(format!("reset-font-{slot:?}")),
                m::reset_setting(&title),
                self.look_setter(cx, move |look| reset_font(look, slot)),
            )
        });
        let picker = self.font_picker(slot, &font, &look.interface_font, cx);
        let sizes = size_select(slot, size, range, self, cx);
        SettingsRow::new(title)
            .description(description)
            .when_some(reset, |row, reset| row.action(reset))
            .control(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(picker)
                    .child(sizes),
            )
            .children(preview)
    }

    pub(super) fn font_picker(
        &mut self,
        slot: FontSlot,
        value: &str,
        interface: &str,
        cx: &mut Context<Self>,
    ) -> Popover {
        let colors = cx.hoshi().clone();
        let monospace = matches!(slot, FontSlot::Code | FontSlot::Terminal);
        let label = font_label(slot, value, interface, cx);
        let families: Vec<String> = self
            .looks
            .system_fonts
            .as_ref()
            .map(|fonts| {
                fonts
                    .iter()
                    .filter(|font| (!monospace || font.monospace) && !bundled(&font.family))
                    .map(|font| font.family.clone())
                    .collect()
            })
            .unwrap_or_default();
        let loading = self.looks.system_fonts.is_none();
        let search = self.looks.font_search.clone();
        let current = value.to_owned();
        let this = cx.weak_entity();
        let presets: Vec<(String, String)> = presets(slot)
            .iter()
            .map(|preset| (preset.to_string(), font_label(slot, preset, interface, cx)))
            .collect();
        let trigger_colors = colors.clone();
        Popover::new(SharedString::from(format!("font-picker-{slot:?}")))
            .side(Side::Bottom)
            .align(Align::End)
            .w(px(256.))
            .p_1()
            .on_open_change({
                let search = search.clone();
                move |change, window, cx| {
                    if change.open {
                        search.update(cx, |search, cx| {
                            search.set_value("", window, cx);
                            search.focus(window, cx);
                        });
                    }
                }
            })
            .trigger(move |open| {
                div()
                    .id(SharedString::from(format!("font-trigger-{slot:?}")))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_1p5()
                    .w(px(176.))
                    .h_8()
                    .pl_2p5()
                    .pr_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(trigger_colors.input)
                    .when(open, |this| this.bg(trigger_colors.muted))
                    .cursor_pointer()
                    .text_sm()
                    .child(div().min_w_0().truncate().child(label))
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size_4()
                            .flex_none()
                            .text_color(trigger_colors.muted_foreground),
                    )
            })
            .content(move |popover, _, cx| {
                let items = font_options(&presets, &families, &search.read(cx).value());
                let default = presets.first().map(|(value, _)| value.clone());
                let count = items.len();
                let popover = popover.clone();
                let this = this.clone();
                let current = current.clone();
                let list_colors = colors.clone();
                let list = uniform_list("font-list", count, move |range, _, cx| {
                    range
                        .map(|index| {
                            let (value, label) = items[index].clone();
                            let selected = value == current;
                            let family = item_family(slot, &value, cx);
                            let is_default = Some(&value) == default.as_ref();
                            let popover = popover.clone();
                            let this = this.clone();
                            let hover = list_colors.accent;
                            div()
                                .id(("font-option", index))
                                .flex()
                                .items_center()
                                .gap_2()
                                .h_7()
                                .px_2()
                                .rounded_md()
                                .text_sm()
                                .cursor_pointer()
                                .hover(move |style| style.bg(hover))
                                .on_click(move |_, window, cx| {
                                    let value = value.clone();
                                    this.update(cx, |this, cx| {
                                        this.change_look(cx, |look| {
                                            set_font(look, slot, Some(value), None)
                                        })
                                    })
                                    .ok();
                                    popover.close(window, cx);
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .when_some(family, |this, family| this.font_family(family))
                                        .child(label),
                                )
                                .when(is_default, |this| {
                                    this.child(
                                        div()
                                            .flex_none()
                                            .text_xs()
                                            .text_color(list_colors.muted_foreground)
                                            .child(m::font_default()),
                                    )
                                })
                                .when(selected, |this| {
                                    this.child(Icon::new(IconName::Check).size_3p5().flex_none())
                                })
                        })
                        .collect()
                })
                .h(px((count.clamp(1, 10) * 28) as f32));
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .h_8()
                            .px_2()
                            .border_b_1()
                            .border_color(colors.border)
                            .text_color(colors.muted_foreground)
                            .child(Icon::new(IconName::Search).size_4().flex_none())
                            .child(div().flex_1().child(text_input(&search).appearance(false))),
                    )
                    .map(|this| {
                        if count == 0 && !loading {
                            this.child(
                                div()
                                    .py_4()
                                    .text_center()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(m::no_fonts_found()),
                            )
                        } else {
                            this.child(list)
                        }
                    })
                    .when(loading, |this| {
                        this.child(
                            div()
                                .py_2()
                                .text_center()
                                .text_xs()
                                .text_color(colors.muted_foreground)
                                .child(m::loading_fonts()),
                        )
                    })
            })
    }
}
