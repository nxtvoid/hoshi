use super::SettingsView;
use crate::look;
use crate::m;
use crate::themes::appearance::{
    Appearance, CODE_SIZE, COMPOSER_SIZE, DEFAULT_APPEARANCE, FontSlot, INTERFACE_SIZE, SizeRange,
    TERMINAL_SIZE, ThemePreference, is_font_preset, size_options,
};
use crate::themes::model::{
    STANDARD_THEME_ID, ThemeAppearance, ThemeColors, ThemeDefinition, built_in_themes, choose_card,
    choose_half, choose_theme, find_theme, half_owners, remove_themes, selection_without,
    serialize_theme_file, theme_colors_for, theme_modes,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::SystemFont;
use hoshi_ui::prelude::*;
use hoshi_ui::{DiffView, theme};
use std::collections::BTreeSet;
use std::time::Duration;

const PREVIEW_FILE: &str = "src/formatUser.ts";
const PREVIEW_PATCH: &str = "@@ -1,3 +1,3 @@\n export function formatUser(user: User) {\n-  return user.name.toUpperCase();\n+  return user.name.trim() || user.email; // 0O 1lI\n }";

/// What the Appearance page keeps between frames.
pub(super) struct AppearanceState {
    font_search: Entity<InputState>,
    /// `None` until read in the background.
    system_fonts: Option<Vec<SystemFont>>,
    removal: Option<Removal>,
    notice: Option<String>,
    pub(super) editor: Option<(Entity<super::theme_editor::ThemeEditor>, Subscription)>,
    pub(super) import: Option<(Entity<super::theme_import::ThemeImport>, Subscription)>,
    /// The collection whose variants fan out from one orb, and which half's.
    fan: Option<(String, ThemeAppearance)>,
    _subscriptions: Vec<Subscription>,
}

struct Removal {
    /// Its themes, more than one when it is a collection.
    themes: Vec<ThemeDefinition>,
    name: String,
    chosen: BTreeSet<String>,
}

impl AppearanceState {
    pub(super) fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        let font_search = cx.new(|cx| InputState::new(window, cx).placeholder(m::search_fonts()));
        let subscriptions = vec![cx.subscribe(&font_search, |_, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        })];
        cx.spawn(async move |this, cx| {
            let fonts = cx
                .background_spawn(async { hoshi_app::fonts::system_fonts() })
                .await;
            this.update(cx, |this, cx| {
                this.looks.system_fonts = Some(fonts);
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            font_search,
            system_fonts: None,
            removal: None,
            notice: None,
            editor: None,
            import: None,
            fan: None,
            _subscriptions: subscriptions,
        }
    }
}

pub(crate) fn scheme_label(preference: ThemePreference) -> String {
    match preference {
        ThemePreference::System => m::theme_system(),
        ThemePreference::Light => m::theme_light(),
        ThemePreference::Dark => m::theme_dark(),
    }
}

pub(crate) fn scheme_icon(preference: ThemePreference) -> IconName {
    match preference {
        ThemePreference::System => IconName::Monitor,
        ThemePreference::Light => IconName::Sun,
        ThemePreference::Dark => IconName::Moon,
    }
}

fn appearance_icon(appearance: ThemeAppearance) -> IconName {
    match appearance {
        ThemeAppearance::Light => IconName::Sun,
        ThemeAppearance::Dark => IconName::Moon,
    }
}

/// The colors of whichever theme owns a half: `None` is hoshi's own.
fn owner_colors(
    owner: Option<&str>,
    appearance: ThemeAppearance,
    custom: &[ThemeDefinition],
) -> ThemeColors {
    let theme =
        find_theme(owner.unwrap_or(STANDARD_THEME_ID), custom).unwrap_or(&built_in_themes()[0]);
    theme_colors_for(theme, appearance)
        .cloned()
        .unwrap_or_else(|| theme.colors.clone())
}

fn paint(colors: &ThemeColors, role: &str) -> Hsla {
    look::color(colors.get(role)).unwrap_or_else(gpui_kit::transparent_black)
}

/// Custom themes grouped by the extension that installed them; a theme of its own stands alone.
fn collections_of(themes: &[ThemeDefinition]) -> Vec<Vec<ThemeDefinition>> {
    let mut groups: Vec<(String, Vec<ThemeDefinition>)> = Vec::new();
    for theme in themes {
        let key = match &theme.collection {
            Some(collection) => format!("collection:{}", collection.id),
            None => format!("theme:{}", theme.id),
        };
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, group)) => group.push(theme.clone()),
            None => groups.push((key, vec![theme.clone()])),
        }
    }
    groups.into_iter().map(|(_, group)| group).collect()
}

/// The part of each variant's name that tells it apart from the others in its collection.
fn variant_labels(themes: &[ThemeDefinition]) -> Vec<String> {
    let words: Vec<Vec<&str>> = themes
        .iter()
        .map(|theme| theme.label.split_whitespace().collect())
        .collect();
    let first = words.first().cloned().unwrap_or_default();
    let shared = first.iter().enumerate().position(|(index, word)| {
        words.iter().any(|label| {
            label
                .get(index)
                .is_none_or(|other| other.to_lowercase() != word.to_lowercase())
        })
    });
    let prefix = shared.unwrap_or(first.len().saturating_sub(1));
    themes
        .iter()
        .zip(&words)
        .map(|(theme, words)| {
            let rest = words
                .iter()
                .skip(prefix)
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            if rest.trim().is_empty() {
                theme.label.clone()
            } else {
                rest
            }
        })
        .collect()
}

/// A round swatch of a theme: its background lit by its accent.
fn orb(colors: &ThemeColors, appearance: ThemeAppearance, size: Pixels) -> Div {
    let background = paint(colors, "background");
    let primary = paint(colors, "primary");
    let foreground = paint(colors, "foreground");
    let angle = match appearance {
        ThemeAppearance::Light => 225.,
        ThemeAppearance::Dark => 45.,
    };
    div()
        .size(size)
        .flex_none()
        .rounded_full()
        .border_2()
        .border_color(background)
        .bg(background)
        .shadow_sm()
        .child(
            div()
                .size_full()
                .rounded_full()
                .border_1()
                .border_color(foreground.opacity(0.1))
                .bg(linear_gradient(
                    angle,
                    linear_color_stop(primary, 0.),
                    linear_color_stop(primary.opacity(0.), 0.75),
                )),
        )
}

/// A small window drawn in one theme's colors.
fn wireframe(colors: &ThemeColors) -> impl IntoElement {
    let sidebar_foreground = paint(colors, "sidebarForeground");
    div()
        .size_full()
        .flex()
        .bg(paint(colors, "background"))
        .child(
            div()
                .w_1_4()
                .h_full()
                .flex()
                .flex_col()
                .gap_1()
                .p_1p5()
                .bg(paint(colors, "sidebar"))
                .border_r_1()
                .border_color(paint(colors, "sidebarBorder"))
                .child(
                    div()
                        .h_1()
                        .rounded_full()
                        .bg(sidebar_foreground.opacity(0.25)),
                )
                .child(
                    div()
                        .h_1p5()
                        .rounded_sm()
                        .bg(paint(colors, "sidebarAccent")),
                )
                .child(
                    div()
                        .h_1()
                        .w_2_3()
                        .rounded_full()
                        .bg(sidebar_foreground.opacity(0.15)),
                ),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .justify_end()
                .gap_1p5()
                .p_2()
                .child(
                    div()
                        .ml_auto()
                        .h_2()
                        .w_1_2()
                        .rounded_md()
                        .bg(paint(colors, "secondary")),
                )
                .child(
                    div()
                        .h_1p5()
                        .w_3_4()
                        .rounded_full()
                        .bg(paint(colors, "foreground").opacity(0.15)),
                )
                .child(
                    div()
                        .h_1p5()
                        .w_1_2()
                        .rounded_full()
                        .bg(paint(colors, "primary")),
                ),
        )
}

fn set_half(look: &mut Appearance, half: ThemeAppearance, card: Option<&str>) {
    let next = choose_half(&look.selection(), half, card);
    look.set_selection(next);
}

impl SettingsView {
    pub(super) fn appearance(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut page = vec![
            self.color_schemes(cx).into_any_element(),
            self.themes_section(window, cx).into_any_element(),
            self.typography(cx).into_any_element(),
        ];
        page.extend(self.removal_dialog(cx));
        page.extend(self.theme_import_dialog(cx));
        page
    }

    pub(super) fn change_look(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Appearance),
    ) {
        look::update(cx, change);
        cx.notify();
    }

    /// A handler for a click that changes the look.
    fn look_setter(
        &self,
        cx: &Context<Self>,
        change: impl Fn(&mut Appearance) + 'static,
    ) -> impl Fn(&mut Window, &mut App) + 'static {
        let this = cx.weak_entity();
        move |_, cx| {
            this.update(cx, |this, cx| this.change_look(cx, &change))
                .ok();
        }
    }

    fn color_schemes(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
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

    fn themes_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
    fn current_owner(&self, half: ThemeAppearance, cx: &App) -> Option<String> {
        let appearance = look::appearance(cx);
        half_owners(&appearance.selection(), &appearance.custom_themes)
            .get(half)
            .map(str::to_owned)
    }

    fn theme_card(
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
    fn card_frame(
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

    fn orb_button(
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

    fn card_actions(
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

    fn collection_card(
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

    fn export_theme(&mut self, theme: &ThemeDefinition, cx: &mut Context<Self>) {
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

    fn ask_removal(
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

    fn confirm_removal(&mut self, _: &mut Window, cx: &mut Context<Self>) {
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

    fn removal_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
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

    fn typography(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
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

    fn font_row(
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

    fn font_picker(
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
                            .child(div().flex_1().child(Input::new(&search).appearance(false))),
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

/// The presets, then the installed families, keeping those whose name contains the query.
fn font_options(
    presets: &[(String, String)],
    families: &[String],
    query: &str,
) -> Vec<(String, String)> {
    let needle = query.trim().to_lowercase();
    let matches = |label: &str| label.to_lowercase().contains(&needle);
    let mut options: Vec<(String, String)> = presets
        .iter()
        .filter(|(_, label)| matches(label))
        .cloned()
        .collect();
    options.extend(
        families
            .iter()
            .filter(|family| !presets.iter().any(|(value, _)| value == *family) && matches(family))
            .map(|family| (family.clone(), family.clone())),
    );
    options
}

fn section_title(text: String) -> impl IntoElement {
    div()
        .px_4()
        .text_base()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

fn reset_button(
    id: impl Into<ElementId>,
    label: String,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    Button::new(id)
        .icon(IconName::RotateCcw)
        .variant(ButtonVariant::Ghost)
        .size(ButtonSize::IconXs)
        .tooltip(label)
        .on_click(move |_, window, cx| on_click(window, cx))
}

fn size_select(
    slot: FontSlot,
    size: u32,
    range: SizeRange,
    view: &SettingsView,
    cx: &Context<SettingsView>,
) -> impl IntoElement {
    Select::new(
        SharedString::from(format!("font-size-{slot:?}")),
        m::font_size_px(size),
    )
    .min_width(px(96.))
    .options(size_options(range).into_iter().map(|option| {
        SelectOption::new(
            SharedString::from(format!("font-size-{slot:?}-{option}")),
            m::font_size_px(option),
            option == size,
        )
        .on_select(view.look_setter(cx, move |look| set_font(look, slot, None, Some(option))))
    }))
}

/// A slot's font, its size and the sizes it takes.
fn slot_values(look: &Appearance, slot: FontSlot) -> (String, u32, SizeRange) {
    match slot {
        FontSlot::Interface => (
            look.interface_font.clone(),
            look.interface_size,
            INTERFACE_SIZE,
        ),
        FontSlot::Composer => (
            look.composer_font.clone(),
            look.composer_size,
            COMPOSER_SIZE,
        ),
        FontSlot::Code => (look.code_font.clone(), look.code_size, CODE_SIZE),
        FontSlot::Terminal => (
            look.terminal_font.clone(),
            look.terminal_size,
            TERMINAL_SIZE,
        ),
    }
}

fn slot_fields(look: &mut Appearance, slot: FontSlot) -> (&mut String, &mut u32) {
    match slot {
        FontSlot::Interface => (&mut look.interface_font, &mut look.interface_size),
        FontSlot::Composer => (&mut look.composer_font, &mut look.composer_size),
        FontSlot::Code => (&mut look.code_font, &mut look.code_size),
        FontSlot::Terminal => (&mut look.terminal_font, &mut look.terminal_size),
    }
}

fn set_font(look: &mut Appearance, slot: FontSlot, font: Option<String>, size: Option<u32>) {
    let (field, size_field) = slot_fields(look, slot);
    if let Some(font) = font {
        *field = font;
    }
    if let Some(size) = size {
        *size_field = size;
    }
}

fn reset_font(look: &mut Appearance, slot: FontSlot) {
    let (font, size, _) = slot_values(&DEFAULT_APPEARANCE, slot);
    set_font(look, slot, Some(font), Some(size));
}

fn presets(slot: FontSlot) -> &'static [&'static str] {
    match slot {
        FontSlot::Interface => &["geist", "inter"],
        FontSlot::Composer => &["interface", "geist", "inter"],
        FontSlot::Code | FontSlot::Terminal => &["jetbrains"],
    }
}

/// Shipped with the app and offered as presets, so the installed copies would repeat them.
fn bundled(family: &str) -> bool {
    matches!(
        family.to_lowercase().as_str(),
        "geist" | "geist variable" | "inter" | "inter variable"
    )
}

fn font_label(slot: FontSlot, value: &str, interface: &str, cx: &App) -> String {
    if !is_font_preset(slot, value) {
        return value.to_owned();
    }
    match value {
        "interface" => font_label(FontSlot::Interface, interface, interface, cx),
        "jetbrains" => theme::default_mono(cx)
            .map(|family| family.to_string())
            .unwrap_or_else(m::font_system_mono),
        "system" if matches!(slot, FontSlot::Code | FontSlot::Terminal) => m::font_system_mono(),
        "system" => m::font_system(),
        "geist" => m::font_geist(),
        "inter" => m::font_inter(),
        "cascadia" => m::font_cascadia_mono(),
        "system-mono" => m::font_system_mono(),
        other => other.to_owned(),
    }
}

/// The family an option draws its own name in.
fn item_family(slot: FontSlot, value: &str, cx: &App) -> Option<SharedString> {
    let monospace = matches!(slot, FontSlot::Code | FontSlot::Terminal);
    match value {
        "interface" => Some(cx.theme().font_family.clone()),
        "geist" => Some(theme::SANS.into()),
        "jetbrains" => theme::default_mono(cx),
        other => look::family(other, monospace, cx),
    }
}

fn chip(text: &str, color: Hsla) -> impl IntoElement {
    div()
        .px(px(2.))
        .rounded_sm()
        .bg(color.opacity(0.12))
        .text_color(color)
        .child(text.to_owned())
}

/// A sent prompt with mentions, in the prompt box's font and size.
fn prompt_preview(cx: &App) -> impl IntoElement {
    let colors = cx.hoshi();
    let (family, size) = theme::fonts(cx)
        .composer
        .unwrap_or_else(|| (cx.theme().font_family.clone(), px(14.)));
    div()
        .mt_3()
        .px_3()
        .py_2()
        .rounded_lg()
        .border_1()
        .border_color(colors.border)
        .bg(colors.background)
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x_1()
                .font_family(family)
                .text_size(size)
                .line_height(relative(1.625))
                .child(m::prompt_preview_use())
                .child(chip("$frontend-design", colors.provider_claude))
                .child(m::prompt_preview_fix())
                .child(chip("surface.test.ts", colors.link))
                .child(m::prompt_preview_align())
                .child(chip("SettingsPanels.tsx", colors.link))
                .child(m::prompt_preview_ship()),
        )
}

/// One file's diff, as the changes panel shows it.
fn code_preview(cx: &App) -> impl IntoElement {
    let colors = cx.hoshi();
    let mono = cx.theme().mono_font_family.clone();
    div()
        .mt_3()
        .overflow_hidden()
        .rounded_lg()
        .border_1()
        .border_color(colors.border)
        .bg(colors.background)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h_8()
                .px_3()
                .border_b_1()
                .border_color(colors.border)
                .text_xs()
                .child({
                    let (icon, tint) = file_path_icon(PREVIEW_FILE);
                    svg()
                        .path(hoshi_ui::assets::file_icon(icon))
                        .size_3p5()
                        .flex_none()
                        .text_color(tint(colors))
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(mono)
                        .child(PREVIEW_FILE),
                )
                .child(div().text_color(colors.pr_open).child("+1"))
                .child(div().text_color(colors.status_error).child("−1")),
        )
        .child(DiffView::patch(
            "code-font-preview",
            PREVIEW_FILE,
            PREVIEW_PATCH,
        ))
}

/// A few lines of a dev server, in the terminal's font and size.
fn terminal_preview(cx: &App) -> impl IntoElement {
    let colors = cx.hoshi().clone();
    let look = look::appearance(cx);
    let (family, size) = if look.separate_fonts {
        (
            look::family(&look.terminal_font, true, cx).or_else(|| theme::default_mono(cx)),
            px(look.terminal_size as f32),
        )
    } else {
        (
            Some(cx.theme().mono_font_family.clone()),
            cx.theme().mono_font_size,
        )
    };
    let span = |text: &str, color: Hsla| div().text_color(color).child(text.to_owned());
    let line = || div().flex().gap_x(px(6.)).whitespace_nowrap();
    div()
        .mt_3()
        .flex()
        .flex_col()
        .p_3()
        .overflow_hidden()
        .rounded_lg()
        .border_1()
        .border_color(colors.border)
        .bg(colors.background)
        .when_some(family, |this, family| this.font_family(family))
        .text_size(size)
        .line_height(relative(1.2))
        .child(
            line()
                .child(span("→", colors.status_done).font_weight(FontWeight::BOLD))
                .child(span("hoshi", colors.link).font_weight(FontWeight::BOLD))
                .child(
                    div()
                        .flex()
                        .child(span("git:(", colors.link))
                        .child(span("main", colors.status_error))
                        .child(span(")", colors.link)),
                )
                .child("bun dev"),
        )
        .child(div().h(size))
        .child(
            line()
                .pl_4()
                .child(span("VITE", colors.status_done).font_weight(FontWeight::BOLD))
                .child(span("v8.0.16", colors.status_done))
                .child(span("ready in", colors.muted_foreground))
                .child(div().font_weight(FontWeight::BOLD).child("1.24s")),
        )
        .child(div().h(size))
        .child(
            line()
                .pl_4()
                .child(span("→", colors.status_done))
                .child(span("Local:", colors.muted_foreground))
                .child(span("http://localhost:1420/", colors.link).underline()),
        )
        .child(
            line()
                .pl_4()
                .child(span("✓ 85 passed", colors.status_done))
                .child(span("△ 2 warnings", colors.status_waiting))
                .child(span("✗ 0 failed", colors.status_error)),
        )
}

fn home_dir() -> std::path::PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{collections_of, font_options, variant_labels};
    use crate::themes::model::{
        ThemeAppearance, ThemeCollection, ThemeDefinition, standard_colors,
    };

    fn theme(id: &str, label: &str, collection: Option<&str>) -> ThemeDefinition {
        let mut theme = ThemeDefinition::new(
            id,
            label,
            ThemeAppearance::Dark,
            standard_colors(ThemeAppearance::Dark).clone(),
        );
        theme.collection = collection.map(|id| ThemeCollection {
            id: id.into(),
            label: id.into(),
        });
        theme
    }

    #[test]
    fn variants_drop_the_words_their_collection_shares() {
        let themes = [
            theme("a", "Catppuccin Mocha", Some("c")),
            theme("b", "Catppuccin Latte", Some("c")),
        ];
        assert_eq!(variant_labels(&themes), ["Mocha", "Latte"]);
    }

    #[test]
    fn themes_group_by_collection_in_order() {
        let groups = collections_of(&[
            theme("a", "A", Some("c")),
            theme("solo", "Solo", None),
            theme("b", "B", Some("c")),
        ]);
        let ids: Vec<Vec<&str>> = groups
            .iter()
            .map(|group| group.iter().map(|theme| theme.id.as_str()).collect())
            .collect();
        assert_eq!(ids, [vec!["a", "b"], vec!["solo"]]);
    }

    #[test]
    fn presets_come_first_and_the_query_filters_both() {
        let presets = [
            ("geist".to_owned(), "Geist".to_owned()),
            ("inter".to_owned(), "Inter".to_owned()),
        ];
        let families = ["Arial".to_owned(), "Georgia".to_owned()];
        let all: Vec<String> = font_options(&presets, &families, "")
            .into_iter()
            .map(|(value, _)| value)
            .collect();
        assert_eq!(all, ["geist", "inter", "Arial", "Georgia"]);
        let found: Vec<String> = font_options(&presets, &families, "ge")
            .into_iter()
            .map(|(value, _)| value)
            .collect();
        assert_eq!(found, ["geist", "Georgia"]);
    }
}
