mod theme_cards;
mod theme_list;
mod theme_removal;
mod typography;

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
use gpui_kit::component::input::{InputEvent, InputState};
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
        let font_search = cx.new(|cx| crate::locale::input(m::search_fonts, window, cx));
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
