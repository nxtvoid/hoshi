//! The floating theme editor, like the web's `ThemeEditorPanel`: a name, light or dark, and
//! either the two simple colors or every color family. The whole app previews it while open.

use super::SettingsView;
use crate::look;
use crate::m;
use crate::themes::color::{BLACK, Rgb, color_to_hex, color_to_rgb, is_color, rgb_to_hex};
use crate::themes::editor::{
    AppearanceLock, AppearanceLockReason, EditorColors, EditorSave, PerAppearance, SIMPLE_ROLES,
    appearance_lock, find_merge_target, managed_colors, save_editor, start_editor,
};
use crate::themes::model::{
    THEME_COLOR_ROLES, ThemeAppearance, ThemeColors, ThemeDefinition, ThemeError, ThemeErrorCode,
    find_theme, standard_colors, theme_modes, update_color_family,
};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_ui::prelude::*;
use hoshi_ui::{ColorPicker, ColorPickerEvent, Hsv};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

/// Above the settings dialog, under the popovers it opens.
const EDITOR_PRIORITY: usize = 60;
const WIDTH: f32 = 416.;
const MARGIN: f32 = 16.;

type Family = (&'static str, fn() -> String);

fn groups() -> [(String, Vec<Family>); 4] {
    [
        (
            m::theme_group_foundation(),
            vec![
                ("background", m::theme_color_background as fn() -> String),
                ("card", m::theme_color_surface),
                ("foreground", m::theme_color_text),
                ("mutedForeground", m::theme_color_muted_text),
                ("border", m::theme_color_border),
                ("input", m::theme_color_input),
            ],
        ),
        (
            m::theme_group_brand(),
            vec![
                ("primary", m::theme_color_accent as fn() -> String),
                ("secondary", m::theme_color_subtle_surface),
                ("accent", m::theme_color_highlight_surface),
                ("ring", m::theme_color_focus),
                ("link", m::theme_color_link),
                ("header", m::theme_color_header),
            ],
        ),
        (
            m::theme_group_context(),
            vec![
                ("sidebar", m::theme_color_sidebar as fn() -> String),
                ("sidebarAccent", m::theme_color_sidebar_selection),
                ("terminalSelection", m::theme_color_terminal_selection),
                ("scrollbar", m::theme_color_scrollbar),
            ],
        ),
        (
            m::theme_group_status(),
            vec![
                ("destructive", m::theme_color_error as fn() -> String),
                ("statusWaiting", m::theme_color_warning),
            ],
        ),
    ]
}

fn simple_label(role: &str) -> String {
    match role {
        "background" => m::theme_color_background(),
        _ => m::theme_color_accent(),
    }
}

pub(super) fn theme_error_message(error: &ThemeError) -> String {
    let subject = error.subject.clone().unwrap_or_default();
    match error.code {
        ThemeErrorCode::NotAnObject => m::theme_error_not_an_object(),
        ThemeErrorCode::UnsupportedVersion => m::theme_error_unsupported_version(),
        ThemeErrorCode::MissingName => m::theme_error_missing_name(),
        ThemeErrorCode::MissingAppearance => m::theme_error_missing_appearance(),
        ThemeErrorCode::MissingColors => m::theme_error_missing_colors(),
        ThemeErrorCode::InvalidId => m::theme_error_invalid_id(),
        ThemeErrorCode::ReservedId => m::theme_error_reserved_id(subject),
        ThemeErrorCode::UnknownRole => m::theme_error_unknown_role(subject),
        ThemeErrorCode::InvalidColor => m::theme_error_invalid_color(subject),
        ThemeErrorCode::NoColors => m::theme_error_no_colors(),
        ThemeErrorCode::InvalidCollection
        | ThemeErrorCode::EmptyCollection
        | ThemeErrorCode::InvalidCollectionThemes => m::theme_error_invalid_collection(),
        ThemeErrorCode::InvalidVariants => m::theme_error_invalid_variants(),
        ThemeErrorCode::VariantRepeatsAppearance => {
            m::theme_error_variant_repeats_appearance(subject)
        }
        ThemeErrorCode::AlreadyInstalled => m::theme_error_already_installed(subject),
        ThemeErrorCode::NotInstalled => m::theme_error_not_installed(subject),
        ThemeErrorCode::PaletteTaken => match error.appearance {
            Some(ThemeAppearance::Dark) => m::theme_error_dark_palette_taken(subject),
            _ => m::theme_error_light_palette_taken(subject),
        },
        ThemeErrorCode::PalettesTaken => m::theme_error_palettes_taken(subject),
        ThemeErrorCode::CollectionChanged => m::theme_error_collection_changed(),
        ThemeErrorCode::VscodeNoBackground => m::theme_error_vscode_no_background(),
        ThemeErrorCode::InvalidJson => m::theme_error_invalid_json(),
        ThemeErrorCode::NoCompatibleThemes => m::open_vsx_no_themes(),
    }
}

fn lock_message(lock: &AppearanceLock) -> String {
    match (lock.reason, lock.appearance) {
        (AppearanceLockReason::EditingLacks, ThemeAppearance::Light) => {
            m::theme_lacks_light(&lock.name)
        }
        (AppearanceLockReason::EditingLacks, ThemeAppearance::Dark) => {
            m::theme_lacks_dark(&lock.name)
        }
        (AppearanceLockReason::MergeHas, ThemeAppearance::Light) => m::theme_has_light(&lock.name),
        (AppearanceLockReason::MergeHas, ThemeAppearance::Dark) => m::theme_has_dark(&lock.name),
    }
}

/// What a color field shows: `oklch()` reads better as hex.
fn shown_value(value: &str) -> String {
    if value.trim().to_lowercase().starts_with("oklch(") {
        color_to_hex(value).unwrap_or_else(|| value.to_owned())
    } else {
        value.to_owned()
    }
}

fn alpha_suffix(value: &str) -> String {
    match color_to_hex(value) {
        Some(hex) if hex.len() == 9 => hex[7..].to_owned(),
        _ => String::new(),
    }
}

fn opaque_hex(value: &str) -> String {
    color_to_hex(value)
        .unwrap_or_else(|| "#000000".into())
        .chars()
        .take(7)
        .collect()
}

fn rgb_text(hex: &str) -> String {
    let Rgb { r, g, b } = color_to_rgb(hex, BLACK);
    format!("{}, {}, {}", r.round(), g.round(), b.round())
}

fn rgb_text_to_hex(text: &str) -> Option<String> {
    let parts: Vec<&str> = text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let channels: Vec<u8> = parts
        .iter()
        .map(|part| part.parse::<u8>().ok())
        .collect::<Option<_>>()?;
    Some(rgb_to_hex(Rgb::new(
        channels[0] as f64,
        channels[1] as f64,
        channels[2] as f64,
    )))
}

fn is_hex(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn to_gpui(rgb: Rgb) -> Rgba {
    Rgba {
        r: (rgb.r / 255.) as f32,
        g: (rgb.g / 255.) as f32,
        b: (rgb.b / 255.) as f32,
        a: 1.,
    }
}

/// The colors the app previews: an unreadable one falls back to hoshi's own.
fn preview_colors(colors: &ThemeColors, appearance: ThemeAppearance) -> ThemeColors {
    let fallback = standard_colors(appearance);
    let mut preview = colors.clone();
    for role in THEME_COLOR_ROLES {
        if !is_color(colors.get(role)) {
            preview.set(role, fallback.get(role));
        }
    }
    preview
}

#[derive(Clone)]
struct Snapshot {
    colors: EditorColors,
    simple_dirty: PerAppearance<bool>,
    regenerate: bool,
}

pub(super) enum ThemeEditorEvent {
    Close,
    Saved(
        Vec<ThemeDefinition>,
        Option<crate::themes::model::ThemeSelection>,
    ),
}

pub(super) struct ThemeEditor {
    editing: Option<ThemeDefinition>,
    custom_themes: Vec<ThemeDefinition>,
    name: Entity<InputState>,
    query: Entity<InputState>,
    appearance: ThemeAppearance,
    advanced: bool,
    colors: EditorColors,
    simple_dirty: PerAppearance<bool>,
    regenerate: bool,
    error: Option<String>,
    minimized: bool,
    /// Where the panel's top left sits once dragged; `None` keeps it at the bottom right.
    position: Rc<Cell<Option<Point<Pixels>>>>,
    /// The pointer's offset in the panel while it is dragged.
    grab: Rc<Cell<Option<Point<Pixels>>>>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    fields: HashMap<&'static str, Entity<InputState>>,
    picker: Entity<ColorPicker>,
    picker_role: Option<&'static str>,
    hex: Entity<InputState>,
    rgb: Entity<InputState>,
    snapshot: Option<Snapshot>,
    applied_merge: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ThemeEditorEvent> for ThemeEditor {}

impl ThemeEditor {
    pub(super) fn new(
        editing_id: Option<String>,
        seed_id: Option<String>,
        seed_name: Option<String>,
        initial: ThemeAppearance,
        custom_themes: Vec<ThemeDefinition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editing = editing_id
            .as_deref()
            .and_then(|id| custom_themes.iter().find(|theme| theme.id == id))
            .cloned();
        let source = editing.clone().or_else(|| {
            seed_id
                .as_deref()
                .and_then(|id| find_theme(id, &custom_themes))
                .cloned()
        });
        let start = start_editor(source.as_ref(), initial);
        let initial_name = editing
            .as_ref()
            .map(|theme| theme.label.clone())
            .or(seed_name)
            .unwrap_or_default();
        let placeholder = if editing.is_some() {
            m::theme_name()
        } else {
            m::theme_name_placeholder()
        };
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(initial_name)
        });
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(m::filter_colors()));
        let picker = cx.new(|_| ColorPicker::new(to_gpui(BLACK)));
        let hex = cx.new(|cx| InputState::new(window, cx));
        let rgb = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&name, window, |this, _, event, window, cx| {
                if let InputEvent::Change = event {
                    this.error = None;
                    this.follow_merge(window, cx);
                    cx.notify();
                }
            }),
            cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&picker, window, |this, _, event, window, cx| {
                let ColorPickerEvent::Change(color) = event;
                let Some(role) = this.picker_role else { return };
                let hex = hoshi_ui::rgb_to_hex(*color);
                let suffix = alpha_suffix(this.colors.get(this.appearance).get(role));
                this.hex
                    .update(cx, |field, cx| field.set_value(hex.clone(), window, cx));
                let rgb = rgb_text(&hex);
                this.rgb
                    .update(cx, |field, cx| field.set_value(rgb, window, cx));
                this.update_color(role, format!("{hex}{suffix}"), window, cx);
            }),
            cx.subscribe_in(&hex, window, |this, field, event, window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let Some(role) = this.picker_role else { return };
                let text = field.read(cx).value().trim().to_lowercase();
                if !is_hex(&text) {
                    return;
                }
                let suffix = alpha_suffix(this.colors.get(this.appearance).get(role));
                this.picker.update(cx, |picker, cx| {
                    picker.set_color(to_gpui(color_to_rgb(&text, BLACK)), cx)
                });
                let rgb = rgb_text(&text);
                this.rgb
                    .update(cx, |field, cx| field.set_value(rgb, window, cx));
                this.update_color(role, format!("{text}{suffix}"), window, cx);
            }),
            cx.subscribe_in(&rgb, window, |this, field, event, window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let Some(role) = this.picker_role else { return };
                let Some(hex) = rgb_text_to_hex(&field.read(cx).value()) else {
                    return;
                };
                let suffix = alpha_suffix(this.colors.get(this.appearance).get(role));
                this.picker.update(cx, |picker, cx| {
                    picker.set_color(to_gpui(color_to_rgb(&hex, BLACK)), cx)
                });
                this.hex
                    .update(cx, |field, cx| field.set_value(hex.clone(), window, cx));
                this.update_color(role, format!("{hex}{suffix}"), window, cx);
            }),
        ];
        name.update(cx, |name, cx| name.focus(window, cx));
        let mut editor = Self {
            editing,
            custom_themes,
            name,
            query,
            appearance: start.appearance,
            advanced: start.advanced,
            colors: start.colors,
            simple_dirty: PerAppearance::default(),
            regenerate: start.regenerate,
            error: None,
            minimized: false,
            position: Rc::default(),
            grab: Rc::default(),
            bounds: Rc::default(),
            fields: HashMap::new(),
            picker,
            picker_role: None,
            hex,
            rgb,
            snapshot: None,
            applied_merge: None,
            _subscriptions: subscriptions,
        };
        editor.follow_merge(window, cx);
        editor.preview(window, cx);
        editor
    }

    fn name_text(&self, cx: &App) -> String {
        self.name.read(cx).value().to_string()
    }

    fn merge_target(&self, cx: &App) -> Option<ThemeDefinition> {
        find_merge_target(
            &self.name_text(cx),
            &self.custom_themes,
            self.editing.as_ref().map(|theme| theme.id.as_str()),
        )
        .cloned()
    }

    /// A new theme named like one with a single palette starts on the other appearance.
    fn follow_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.merge_target(cx);
        let taken = target.as_ref().map(theme_modes).unwrap_or_default();
        let key = match (&self.editing, &target) {
            (None, Some(target)) => Some(format!(
                "{}:{}",
                target.id,
                taken
                    .iter()
                    .map(|half| half.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            )),
            _ => None,
        };
        if key == self.applied_merge {
            return;
        }
        self.applied_merge = key.clone();
        if key.is_some() && taken.len() == 1 && taken[0] == self.appearance {
            self.appearance = self.appearance.other();
            self.preview(window, cx);
        }
    }

    fn preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let colors = preview_colors(self.colors.get(self.appearance), self.appearance);
        look::set_preview(Some((self.appearance, colors)), cx);
        self.sync_fields(window, cx);
    }

    /// Shows the current colors in every field but the one being typed in.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let colors = self.colors.get(self.appearance).clone();
        for (role, field) in &self.fields {
            let shown = shown_value(colors.get(role));
            let focused = field.read(cx).focus_handle(cx).is_focused(window);
            if !focused && field.read(cx).value().as_ref() != shown {
                field.update(cx, |field, cx| field.set_value(shown, window, cx));
            }
        }
    }

    fn field(
        &mut self,
        role: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(field) = self.fields.get(role) {
            return field.clone();
        }
        let shown = shown_value(self.colors.get(self.appearance).get(role));
        let field = cx.new(|cx| InputState::new(window, cx).default_value(shown));
        let subscription =
            cx.subscribe_in(&field, window, move |this, field, event, window, cx| {
                if let InputEvent::Change = event {
                    this.error = None;
                    let value = field.read(cx).value().to_string();
                    this.update_color(role, value, window, cx);
                }
            });
        self._subscriptions.push(subscription);
        self.fields.insert(role, field.clone());
        field
    }

    fn update_color(
        &mut self,
        role: &str,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let appearance = self.appearance;
        let simple = SIMPLE_ROLES.contains(&role);
        let current = self.colors.get(appearance).clone();
        let next = if self.advanced {
            update_color_family(&current, role, &value)
        } else {
            let mut next = current.clone();
            next.set(role, value);
            if simple {
                managed_colors(appearance, &next)
            } else {
                next
            }
        };
        *self.colors.get_mut(appearance) = next;
        if !self.advanced && simple {
            *self.simple_dirty.get_mut(appearance) = true;
        }
        if self.advanced {
            self.regenerate = true;
        }
        self.preview(window, cx);
        cx.notify();
    }

    fn set_appearance(
        &mut self,
        appearance: ThemeAppearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.appearance = appearance;
        self.preview(window, cx);
        cx.notify();
    }

    fn change_advanced(&mut self, advanced: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.advanced = advanced;
        if !advanced && self.regenerate {
            let modes = match &self.editing {
                Some(editing) if theme_modes(editing).len() > 1 => ThemeAppearance::ALL.to_vec(),
                _ => vec![self.appearance],
            };
            for mode in modes {
                *self.simple_dirty.get_mut(mode) = true;
                let rebuilt = managed_colors(mode, self.colors.get(mode));
                *self.colors.get_mut(mode) = rebuilt;
            }
            self.regenerate = false;
        }
        self.preview(window, cx);
        cx.notify();
    }

    fn open_picker(&mut self, role: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.colors.get(self.appearance).get(role).to_owned();
        let value = if is_color(&value) {
            value
        } else {
            "#000000".into()
        };
        let hex = opaque_hex(&value);
        self.snapshot = Some(Snapshot {
            colors: self.colors.clone(),
            simple_dirty: self.simple_dirty,
            regenerate: self.regenerate,
        });
        self.picker_role = Some(role);
        self.picker.update(cx, |picker, cx| {
            picker.set_color(to_gpui(color_to_rgb(&hex, BLACK)), cx)
        });
        let rgb = rgb_text(&hex);
        self.hex
            .update(cx, |field, cx| field.set_value(hex, window, cx));
        self.rgb
            .update(cx, |field, cx| field.set_value(rgb, window, cx));
    }

    fn close_picker(&mut self, cancelled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_role = None;
        let snapshot = self.snapshot.take();
        if cancelled && let Some(snapshot) = snapshot {
            self.colors = snapshot.colors;
            self.simple_dirty = snapshot.simple_dirty;
            self.regenerate = snapshot.regenerate;
            self.preview(window, cx);
        }
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let name = self.name_text(cx);
        let saved = save_editor(&EditorSave {
            name: &name,
            editing: self.editing.as_ref(),
            appearance: self.appearance,
            colors: &self.colors,
            advanced: self.advanced,
            simple_dirty: self.simple_dirty,
            custom_themes: &self.custom_themes,
        });
        match saved {
            Ok(result) => cx.emit(ThemeEditorEvent::Saved(result.themes, result.selection)),
            Err(error) => {
                self.error = Some(theme_error_message(&error));
                cx.notify();
            }
        }
    }

    fn color_field(
        &mut self,
        role: &'static str,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.hoshi().clone();
        let value = self.colors.get(self.appearance).get(role).to_owned();
        let swatch = look::color(&value);
        let valid = swatch.is_some();
        let field = self.field(role, window, cx);
        let mono = cx.theme().mono_font_family.clone();
        let editor = cx.weak_entity();
        let picker = self.picker.clone();
        let hex = self.hex.clone();
        let rgb = self.rgb.clone();
        let picker_label = label.clone();
        let ring = colors.foreground.opacity(0.3);
        let popover = Popover::new(SharedString::from(format!("color-{role}")))
            .side(Side::Bottom)
            .align(Align::End)
            .w(px(288.))
            .p_3()
            .on_open_change(move |change, window, cx| {
                editor
                    .update(cx, |this, cx| {
                        if change.open {
                            this.open_picker(role, window, cx);
                        } else {
                            let cancelled = change.reason == PopoverChangeReason::EscapeKey;
                            this.close_picker(cancelled, window, cx);
                        }
                    })
                    .ok();
            })
            .trigger(move |_| {
                div()
                    .id(SharedString::from(format!("swatch-{role}")))
                    .size_6()
                    .flex_none()
                    .rounded_full()
                    .border_1()
                    .border_color(ring)
                    .cursor_pointer()
                    .when_some(swatch, |this, color| this.bg(color))
            })
            .content(move |_, _, cx| {
                let current: Hsla = Hsv::from_rgb(picker.read(cx).color()).to_rgb().into();
                let muted = cx.hoshi().muted_foreground;
                let caption = |text: String| {
                    div()
                        .px_1()
                        .text_size(rems(0.625))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(muted)
                        .child(text.to_uppercase())
                };
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(picker_label.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child(m::choose_a_color()),
                                    ),
                            )
                            .child(div().size_7().flex_none().rounded_full().bg(current)),
                    )
                    .child(picker.clone())
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .gap_1()
                                    .child(caption(m::color_hex()))
                                    .child(Input::new(&hex)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .gap_1()
                                    .child(caption(m::color_rgb()))
                                    .child(Input::new(&rgb)),
                            ),
                    )
            });
        div()
            .flex()
            .min_h_10()
            .min_w_0()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_lg()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(colors.muted_foreground)
                    .child(label),
            )
            .child(popover)
            .child(
                div()
                    .w(px(112.))
                    .flex_none()
                    .font_family(mono)
                    .text_xs()
                    .when(!valid, |this| this.text_color(colors.destructive))
                    .child(Input::new(&field)),
            )
            .into_any_element()
    }

    fn appearance_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let target = self.merge_target(cx);
        div()
            .grid()
            .grid_cols(2)
            .gap(px(2.))
            .p(px(2.))
            .rounded_lg()
            .bg(colors.muted)
            .children(ThemeAppearance::ALL.map(|option| {
                let lock = appearance_lock(option, self.editing.as_ref(), target.as_ref());
                let selected = self.appearance == option;
                let label = match option {
                    ThemeAppearance::Light => m::theme_light(),
                    ThemeAppearance::Dark => m::theme_dark(),
                };
                let button = div()
                    .id(SharedString::from(format!("editor-{}", option.as_str())))
                    .h_6()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .map(|this| {
                        if selected {
                            this.bg(colors.background)
                                .text_color(colors.foreground)
                                .shadow_sm()
                        } else {
                            this.text_color(colors.muted_foreground)
                        }
                    })
                    .when(lock.is_some(), |this| this.opacity(0.5))
                    .when(lock.is_none(), |this| {
                        this.cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_appearance(option, window, cx)
                            }))
                    })
                    .child(label);
                match lock {
                    Some(lock) => button
                        .with_tooltip(Tooltip::new(lock_message(&lock)))
                        .into_any_element(),
                    None => button.into_any_element(),
                }
            }))
    }
}

impl Render for ThemeEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let editing = self.editing.is_some();
        let title = if editing {
            m::edit_theme()
        } else {
            m::create_theme()
        };
        let viewport = window.viewport_size();
        let width = px(WIDTH).min(viewport.width - px(2. * MARGIN));
        let max_height = px(672.).min(viewport.height - px(96.));
        let origin = self.position.get().unwrap_or_else(|| {
            let height = self.bounds.get().size.height.max(px(200.));
            point(
                viewport.width - width - px(MARGIN),
                viewport.height - height - px(MARGIN),
            )
        });
        let origin = point(
            origin
                .x
                .clamp(px(8.), (viewport.width - width - px(8.)).max(px(8.))),
            origin
                .y
                .clamp(px(8.), (viewport.height - px(48.)).max(px(8.))),
        );
        let name = self.name_text(cx);
        let target = self.merge_target(cx);
        let query = self.query.read(cx).value().trim().to_lowercase();

        let body = if self.minimized {
            None
        } else {
            let mut fields: Vec<AnyElement> = Vec::new();
            if self.advanced {
                let mut any = false;
                for (title, families) in groups() {
                    let shown: Vec<Family> = families
                        .into_iter()
                        .filter(|(_, label)| {
                            query.is_empty() || label().to_lowercase().contains(&query)
                        })
                        .collect();
                    if shown.is_empty() {
                        continue;
                    }
                    any = true;
                    let mut section = vec![
                        div()
                            .px_2()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(title)
                            .into_any_element(),
                    ];
                    for (role, label) in shown {
                        section.push(self.color_field(role, label(), window, cx));
                    }
                    fields.push(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(section)
                            .into_any_element(),
                    );
                }
                if !any {
                    fields.push(
                        div()
                            .px_2()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(m::no_matches())
                            .into_any_element(),
                    );
                }
            } else {
                for role in SIMPLE_ROLES {
                    fields.push(self.color_field(role, simple_label(role), window, cx));
                }
            }
            let row = |label: AnyElement, control: AnyElement| {
                div()
                    .grid()
                    .grid_cols(3)
                    .items_center()
                    .gap_3()
                    .child(label)
                    .child(div().col_span(2).min_w_0().child(control))
            };
            let label = |text: String| {
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(text)
                    .into_any_element()
            };
            let advanced = self.advanced;
            Some(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("theme-editor-body")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .gap_3()
                            .overflow_y_scroll()
                            .py_3()
                            .pl_3()
                            .pr_1p5()
                            .child(row(
                                label(m::theme_name()),
                                Input::new(&self.name).into_any_element(),
                            ))
                            .when_some(self.error.clone(), |this, error| {
                                this.child(
                                    div().text_sm().text_color(colors.destructive).child(error),
                                )
                            })
                            .child(row(
                                label(m::theme_appearance()),
                                self.appearance_switch(cx).into_any_element(),
                            ))
                            .child(
                                div()
                                    .grid()
                                    .grid_cols(3)
                                    .items_start()
                                    .gap_3()
                                    .child(
                                        div()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(m::theme_colors()),
                                            )
                                            .when(!advanced, |this| {
                                                this.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(colors.muted_foreground)
                                                        .child(m::theme_colors_simple_hint()),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .col_span(2)
                                            .flex()
                                            .min_w_0()
                                            .items_start()
                                            .gap_3()
                                            .when(advanced, |this| {
                                                this.child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .child(Input::new(&self.query)),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .ml_auto()
                                                    .flex()
                                                    .flex_none()
                                                    .items_center()
                                                    .gap_2()
                                                    .pt(px(2.))
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .child(m::advanced()),
                                                    )
                                                    .child(
                                                        Switch::new("theme-editor-advanced")
                                                            .checked(advanced)
                                                            .on_change({
                                                                let this = cx.weak_entity();
                                                                move |on, window, cx| {
                                                                    this.update(cx, |this, cx| {
                                                                        this.change_advanced(
                                                                            on, window, cx,
                                                                        )
                                                                    })
                                                                    .ok();
                                                                }
                                                            }),
                                                    ),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(if advanced { px(16.) } else { px(4.) })
                                    .children(fields),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .border_t_1()
                            .border_color(colors.border.opacity(0.7))
                            .child(
                                Button::new("theme-editor-cancel")
                                    .label(m::cancel())
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .on_click(
                                        cx.listener(|_, _, _, cx| cx.emit(ThemeEditorEvent::Close)),
                                    ),
                            )
                            .child({
                                let (icon, label) = match (editing, &target) {
                                    (true, Some(target)) => {
                                        (None, m::merge_into_theme(&target.label))
                                    }
                                    (true, None) => (None, m::save_changes()),
                                    (false, Some(_)) => (
                                        Some(IconName::Plus),
                                        match self.appearance {
                                            ThemeAppearance::Light => m::add_light_palette(),
                                            ThemeAppearance::Dark => m::add_dark_palette(),
                                        },
                                    ),
                                    (false, None) => (Some(IconName::Brush), m::create_theme()),
                                };
                                Button::new("theme-editor-save")
                                    .label(label)
                                    .size(ButtonSize::Sm)
                                    .disabled(name.trim().is_empty())
                                    .when_some(icon, |button, icon| button.icon(icon))
                                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx)))
                            }),
                    ),
            )
        };

        let grab = self.grab.clone();
        let position = self.position.clone();
        let bounds = self.bounds.clone();
        let entity = cx.entity().downgrade();
        let panel = div()
            .id("theme-editor")
            .occlude()
            .flex()
            .flex_col()
            .w(width)
            .when(!self.minimized, |this| this.max_h(max_height))
            .overflow_hidden()
            .rounded_xl()
            .border_1()
            .border_color(colors.foreground.opacity(0.1))
            .bg(colors.popover)
            .text_color(colors.popover_foreground)
            .shadow_2xl()
            .child(
                canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .id("theme-editor-header")
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(colors.border.opacity(0.7))
                    .cursor_grab()
                    .on_mouse_down(MouseButton::Left, {
                        let grab = grab.clone();
                        let bounds = self.bounds.clone();
                        move |event, _, _| {
                            let origin = bounds.get().origin;
                            grab.set(Some(event.position - origin));
                        }
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        Button::new("theme-editor-minimize")
                            .icon(if self.minimized {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            })
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::IconXs)
                            .tooltip(if self.minimized {
                                m::expand_theme_editor()
                            } else {
                                m::minimize_theme_editor()
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.minimized = !this.minimized;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("theme-editor-close")
                            .icon(IconName::X)
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::IconXs)
                            .tooltip(m::close_theme_editor())
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ThemeEditorEvent::Close))),
                    ),
            )
            .children(body)
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        let moving = (grab.clone(), position.clone(), entity.clone());
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase != DispatchPhase::Bubble || !event.dragging() {
                                return;
                            }
                            if let Some(offset) = moving.0.get() {
                                moving.1.set(Some(event.position - offset));
                                moving.2.update(cx, |_, cx| cx.notify()).ok();
                            }
                        });
                        let ending = grab.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, _| {
                            if phase == DispatchPhase::Bubble {
                                ending.set(None);
                            }
                        });
                    },
                )
                .absolute()
                .size_0(),
            );
        deferred(anchored().position(origin).child(panel)).with_priority(EDITOR_PRIORITY)
    }
}

impl SettingsView {
    pub(super) fn open_theme_editor(
        &mut self,
        editing: Option<String>,
        seed: Option<String>,
        name: Option<String>,
        initial: ThemeAppearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let custom = look::appearance(cx).custom_themes.clone();
        let editor =
            cx.new(|cx| ThemeEditor::new(editing, seed, name, initial, custom, window, cx));
        let subscription = cx.subscribe_in(&editor, window, |this, _, event, _, cx| {
            match event {
                ThemeEditorEvent::Close => {}
                ThemeEditorEvent::Saved(themes, selection) => {
                    let (themes, selection) = (themes.clone(), selection.clone());
                    look::update(cx, |look| {
                        look.custom_themes = themes;
                        if let Some(selection) = selection {
                            look.set_selection(selection);
                        }
                    });
                }
            }
            this.close_theme_editor(cx);
        });
        self.looks.editor = Some((editor, subscription));
        cx.notify();
    }

    pub(super) fn close_theme_editor(&mut self, cx: &mut Context<Self>) {
        if self.looks.editor.take().is_some() {
            look::set_preview(None, cx);
        }
        cx.notify();
    }

    pub(super) fn theme_editor(&self) -> Option<Entity<ThemeEditor>> {
        self.looks.editor.as_ref().map(|(editor, _)| editor.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{ThemeEditor, ThemeEditorEvent, is_hex, rgb_text, rgb_text_to_hex, shown_value};
    use crate::look::{self, Look};
    use crate::themes::color::color_to_hex;
    use crate::themes::model::ThemeAppearance;
    use gpui_kit::{
        AppContext as _, Bounds, Point, TestAppContext, VisualTestContext, WindowBounds,
        WindowOptions, px, size,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    #[gpui_kit::test]
    fn the_accent_previews_on_the_app_and_saving_adds_the_theme(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (window, editor) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1000.), px(800.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                look::init(None, window, cx);
                cx.new(|cx| {
                    ThemeEditor::new(
                        None,
                        None,
                        None,
                        ThemeAppearance::Light,
                        Vec::new(),
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
        let mut cx = VisualTestContext::from_window(window, cx);
        cx.run_until_parked();
        let saved = Rc::new(RefCell::new(None));
        let seen = saved.clone();
        cx.update(|_, cx| {
            cx.subscribe(&editor, move |_, event: &ThemeEditorEvent, _| {
                if let ThemeEditorEvent::Saved(themes, selection) = event {
                    *seen.borrow_mut() = Some((themes.clone(), selection.clone()));
                }
            })
            .detach()
        });

        editor.update_in(&mut cx, |editor, window, cx| {
            editor.update_color("primary", "#7c3aed".into(), window, cx);
            editor
                .name
                .update(cx, |name, cx| name.set_value("Violeta", window, cx));
        });
        let primary = cx.update(|_, cx| {
            cx.global::<Look>()
                .preview
                .as_ref()
                .and_then(|(_, colors)| color_to_hex(colors.get("primary")))
        });
        assert_eq!(primary.as_deref(), Some("#7c3aed"));

        editor.update(&mut cx, |editor, cx| editor.submit(cx));
        let (themes, selection) = saved.borrow_mut().take().expect("the editor saved");
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].label, "Violeta");
        assert_eq!(
            color_to_hex(themes[0].colors.get("primary")).as_deref(),
            Some("#7c3aed")
        );
        assert_eq!(
            selection.map(|selection| selection.theme),
            Some(themes[0].id.clone())
        );
    }

    #[test]
    fn rgb_text_reads_three_channels() {
        assert_eq!(rgb_text_to_hex("255, 128, 0").as_deref(), Some("#ff8000"));
        assert_eq!(rgb_text_to_hex("255 128 0").as_deref(), Some("#ff8000"));
        assert_eq!(rgb_text_to_hex("256, 0, 0"), None);
        assert_eq!(rgb_text_to_hex("1, 2"), None);
        assert_eq!(rgb_text("#ff8000"), "255, 128, 0");
    }

    #[test]
    fn oklch_shows_as_hex_and_hex_is_checked() {
        assert_eq!(shown_value("oklch(1 0 0)"), "#ffffff");
        assert_eq!(shown_value("#123456"), "#123456");
        assert!(is_hex("#a1b2c3"));
        assert!(!is_hex("#a1b2c"));
    }
}
