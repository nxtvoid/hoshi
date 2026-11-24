//! "Add a theme", like the web's `ThemeImportDialog`: community themes from Open VSX, theme
//! files picked or dropped, or JSON pasted in.

use super::SettingsView;
use super::theme_editor::theme_error_message;
use crate::format::compact_number as compact;
use crate::look;
use crate::m;
use crate::themes::appearance::Appearance;
use crate::themes::model::{
    THEME_NAME_MAX, ThemeDefinition, ThemeError, ThemeErrorCode, choose_card, install_theme,
    parse_theme_file, replace_collection, selection_without, theme_file_json, update_theme,
};
use crate::themes::vscode::{
    extension_themes, humanize_theme_name, pair_vscode_themes, parse_theme_import,
    resolve_theme_label_collisions,
};
use gpui_kit::component::input::{EditorState, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{ThemeExtension, ThemeExtensionSort};
use hoshi_ui::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const MAX_THEME_FILE_BYTES: u64 = 256 * 1024;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(350);
const SUGGESTED_SEARCHES: [&str; 4] = ["Dracula", "Catppuccin", "Nord", "Tokyo Night"];
const SORTS: [ThemeExtensionSort; 4] = [
    ThemeExtensionSort::DownloadCount,
    ThemeExtensionSort::Rating,
    ThemeExtensionSort::Timestamp,
    ThemeExtensionSort::Relevance,
];

fn sort_label(sort: ThemeExtensionSort) -> String {
    match sort {
        ThemeExtensionSort::DownloadCount => m::sort_most_downloaded(),
        ThemeExtensionSort::Rating => m::sort_best_rated(),
        ThemeExtensionSort::Timestamp => m::sort_newest(),
        ThemeExtensionSort::Relevance => m::sort_most_relevant(),
    }
}

/// What a failed Open VSX request or import says, from the code the engine gives.
fn failure_message(code: &str) -> String {
    match code {
        "search_unavailable" => m::open_vsx_search_unavailable(),
        "search_unreadable" | "details_unreadable" => m::open_vsx_unreadable(),
        "details_unavailable" => m::open_vsx_details_unavailable(),
        "no_manifest" => m::open_vsx_no_manifest(),
        "no_themes" => m::open_vsx_no_themes(),
        "too_many_themes" => m::open_vsx_too_many_themes(),
        "download_failed" => m::open_vsx_download_failed(),
        "no_checksum" | "invalid_checksum" => m::open_vsx_no_checksum(),
        "integrity_failed" => m::open_vsx_integrity_failed(),
        "unsafe_package" => m::open_vsx_unsafe_package(),
        "package_mismatch" | "license_mismatch" => m::open_vsx_package_mismatch(),
        _ => m::open_vsx_theme_unreadable(),
    }
}

fn size_label(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024. * 1024.))
    } else if bytes >= 1024 {
        format!("{} KB", (bytes as f64 / 1024.).round())
    } else {
        format!("{bytes} B")
    }
}

fn oversized(bytes: u64) -> Option<String> {
    (bytes > MAX_THEME_FILE_BYTES)
        .then(|| m::theme_file_too_large(size_label(MAX_THEME_FILE_BYTES), size_label(bytes)))
}

/// A copy of `theme` under a name no installed theme has: the file's name, else "Name (2)".
fn versioned_copy(
    theme: &ThemeDefinition,
    themes: &[ThemeDefinition],
    preferred: Option<&str>,
) -> Result<ThemeDefinition, ThemeError> {
    let copy = |name: &str| {
        let mut file = theme_file_json(theme);
        if let Some(object) = file.as_object_mut() {
            object.remove("id");
            object.remove("collection");
            object.insert("name".into(), Value::String(name.to_owned()));
        }
        parse_theme_file(&file)
    };
    let free =
        |candidate: &ThemeDefinition| !themes.iter().any(|existing| existing.id == candidate.id);
    if let Some(preferred) = preferred
        && preferred.to_lowercase() != theme.label.to_lowercase()
    {
        let candidate = copy(&preferred.chars().take(THEME_NAME_MAX).collect::<String>())?;
        if free(&candidate) {
            return Ok(candidate);
        }
    }
    for count in 1..100 {
        let tail = format!(" ({count})");
        let keep = THEME_NAME_MAX.saturating_sub(tail.chars().count());
        let name: String = theme.label.chars().take(keep).collect();
        let candidate = copy(&format!("{name}{tail}"))?;
        if free(&candidate) {
            return Ok(candidate);
        }
    }
    Err(ThemeError::about(
        ThemeErrorCode::AlreadyInstalled,
        theme.label.clone(),
    ))
}

pub(super) enum ThemeImportEvent {
    Close,
}

pub(super) struct ThemeImport {
    query: Entity<InputState>,
    sort: ThemeExtensionSort,
    results: Option<Vec<ThemeExtension>>,
    search_error: Option<String>,
    searching: bool,
    search_request: u64,
    debounce: Option<Task<()>>,
    installing: Option<String>,
    pending_update: Option<ThemeExtension>,
    json: Entity<EditorState>,
    file_name: Option<String>,
    reading: bool,
    drop_target: bool,
    conflicts: Option<Vec<ThemeDefinition>>,
    error: Option<String>,
    /// Extensions' icons by address: `None` while one loads or when it couldn't.
    icons: HashMap<String, Option<Arc<Image>>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ThemeImportEvent> for ThemeImport {}

impl ThemeImport {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| crate::locale::input(m::search_themes_placeholder, window, cx));
        let json = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(false)
                .indent_guides(false)
                .folding(false)
                .soft_wrap(true)
        });
        let subscriptions = vec![
            cx.subscribe(&query, |this, field, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    let text = field.read(cx).value().to_string();
                    this.change_query(text, cx);
                }
                InputEvent::PressEnter { .. } if this.installing.is_none() => {
                    this.debounce = None;
                    let text = field.read(cx).value().to_string();
                    this.run_search(&text, cx);
                }
                _ => {}
            }),
            cx.subscribe(&json, |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }),
        ];
        query.update(cx, |query, cx| query.focus(window, cx));
        Self {
            query,
            sort: ThemeExtensionSort::DownloadCount,
            results: None,
            search_error: None,
            searching: false,
            search_request: 0,
            debounce: None,
            installing: None,
            pending_update: None,
            json,
            file_name: None,
            reading: false,
            drop_target: false,
            conflicts: None,
            error: None,
            icons: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    fn installed(cx: &App) -> Vec<ThemeDefinition> {
        look::appearance(cx).custom_themes.clone()
    }

    fn change_query(&mut self, text: String, cx: &mut Context<Self>) {
        if text.trim().is_empty() {
            self.debounce = None;
            self.run_search("", cx);
            return;
        }
        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            this.update(cx, |this, cx| this.run_search(&text, cx)).ok();
        }));
    }

    fn run_search(&mut self, text: &str, cx: &mut Context<Self>) {
        self.search_request += 1;
        let request = self.search_request;
        let query = text.trim().to_owned();
        if query.is_empty() {
            self.results = None;
            self.search_error = None;
            self.searching = false;
            cx.notify();
            return;
        }
        self.searching = true;
        self.search_error = None;
        cx.notify();
        let sort = self.sort;
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    hoshi_app::theme_import::search_theme_extensions(&query, sort)
                })
                .await;
            this.update(cx, |this, cx| {
                if request != this.search_request {
                    return;
                }
                this.searching = false;
                match found {
                    Ok(found) => this.results = Some(found),
                    Err(code) => {
                        this.results = None;
                        this.search_error = Some(failure_message(&code));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn search_for(&mut self, suggestion: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |query, cx| query.set_value(suggestion, window, cx));
        self.debounce = None;
        self.run_search(suggestion, cx);
    }

    fn install(&mut self, extension: ThemeExtension, allow_update: bool, cx: &mut Context<Self>) {
        self.search_error = None;
        let before: Vec<ThemeDefinition> = Self::installed(cx)
            .into_iter()
            .filter(|theme| {
                theme
                    .collection
                    .as_ref()
                    .map(|collection| collection.id.as_str())
                    == Some(extension.collection_id.as_str())
            })
            .collect();
        if !before.is_empty() && !allow_update {
            self.pending_update = Some(extension);
            cx.notify();
            return;
        }
        self.search_request += 1;
        self.searching = false;
        self.installing = Some(extension.id.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let fetched = extension.clone();
            let files = cx
                .background_spawn(async move {
                    hoshi_app::theme_import::import_theme_extension(&fetched)
                })
                .await;
            this.update(cx, |this, cx| {
                this.installing = None;
                let themes = match files {
                    Ok(files) => extension_themes(&extension, &files)
                        .map_err(|error| theme_error_message(&error)),
                    Err(code) => Err(failure_message(&code)),
                };
                let themes = match themes {
                    Ok(themes) => themes,
                    Err(message) => {
                        this.search_error = Some(message);
                        cx.notify();
                        return;
                    }
                };
                let current = Self::installed(cx);
                let still: Vec<ThemeDefinition> = current
                    .iter()
                    .filter(|theme| {
                        theme
                            .collection
                            .as_ref()
                            .map(|collection| collection.id.as_str())
                            == Some(extension.collection_id.as_str())
                    })
                    .cloned()
                    .collect();
                if still != before {
                    this.search_error = Some(theme_error_message(&ThemeError::new(
                        ThemeErrorCode::CollectionChanged,
                    )));
                    cx.notify();
                    return;
                }
                let removed: Vec<String> = still
                    .iter()
                    .filter(|old| !themes.iter().any(|added| added.id == old.id))
                    .map(|old| old.id.clone())
                    .collect();
                match replace_collection(&current, &extension.collection_id, themes) {
                    Ok(next) => {
                        this.apply(cx, move |look| {
                            look.custom_themes = next;
                            if !removed.is_empty() {
                                let selection = selection_without(&look.selection(), &removed);
                                look.set_selection(selection);
                            }
                        });
                        cx.emit(ThemeImportEvent::Close);
                    }
                    Err(error) => this.search_error = Some(theme_error_message(&error)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Appearance)) {
        look::update(cx, change);
    }

    fn choose_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.read_files(paths, window, cx))
                .ok();
        })
        .detach();
    }

    fn read_files(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        self.reading = true;
        cx.notify();
        let single = paths.len() == 1;
        cx.spawn_in(window, async move |this, cx| {
            let files: Vec<(String, Result<String, String>)> = cx
                .background_spawn(async move {
                    paths
                        .into_iter()
                        .map(|path| {
                            let name = path
                                .file_name()
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let read = match std::fs::metadata(&path) {
                                Ok(meta) => match oversized(meta.len()) {
                                    Some(too_large) => Err(too_large),
                                    None => std::fs::read_to_string(&path)
                                        .map_err(|_| m::theme_file_unreadable()),
                                },
                                Err(_) => Err(m::theme_file_unreadable()),
                            };
                            (name, read)
                        })
                        .collect()
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.reading = false;
                if single {
                    let (name, read) = files.into_iter().next().expect("one file");
                    match read {
                        Ok(text) => {
                            this.json
                                .update(cx, |json, cx| json.set_value(text, window, cx));
                            this.file_name = Some(name);
                            this.error = None;
                        }
                        Err(message) => this.error = Some(message),
                    }
                    cx.notify();
                } else {
                    this.install_batch(files, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn install_batch(
        &mut self,
        files: Vec<(String, Result<String, String>)>,
        cx: &mut Context<Self>,
    ) {
        let fallback = m::vscode_theme();
        let mut failures = Vec::new();
        let mut parsed: Vec<(ThemeDefinition, String)> = Vec::new();
        for (name, read) in files {
            match read.and_then(|text| {
                parse_theme_import(&text, &fallback).map_err(|error| theme_error_message(&error))
            }) {
                Ok(theme) => parsed.push((theme, name)),
                Err(reason) => failures.push(m::theme_file_failure(&name, reason)),
            }
        }
        let entries: Vec<(ThemeDefinition, Option<&str>)> = parsed
            .iter()
            .map(|(theme, name)| (theme.clone(), Some(name.as_str())))
            .collect();
        let mut themes = Self::installed(cx);
        let mut installed = 0;
        let mut conflicting = Vec::new();
        for theme in pair_vscode_themes(&resolve_theme_label_collisions(&entries), None) {
            if themes.iter().any(|existing| existing.id == theme.id) {
                conflicting.push(theme);
                continue;
            }
            match install_theme(&themes, theme.clone()) {
                Ok(next) => {
                    themes = next;
                    installed += 1;
                }
                Err(error) => failures.push(m::theme_file_failure(
                    &theme.label,
                    theme_error_message(&error),
                )),
            }
        }
        if installed > 0 {
            self.apply(cx, move |look| look.custom_themes = themes);
        }
        self.error = (!failures.is_empty()).then(|| failures.join("; "));
        if !conflicting.is_empty() {
            self.conflicts = Some(conflicting);
        } else if installed > 0 && failures.is_empty() {
            cx.emit(ThemeImportEvent::Close);
        }
        cx.notify();
    }

    fn resolve_conflicts(&mut self, update: bool, cx: &mut Context<Self>) {
        let Some(conflicts) = self.conflicts.take() else {
            return;
        };
        let preferred = (conflicts.len() == 1)
            .then(|| self.file_name.clone())
            .flatten()
            .map(|name| {
                let stem = name
                    .rsplit_once('.')
                    .map_or(name.as_str(), |(stem, _)| stem);
                humanize_theme_name(stem)
            });
        let mut failures = Vec::new();
        let mut themes = Self::installed(cx);
        for theme in conflicts {
            let result = if update {
                let existing = themes.iter().find(|candidate| candidate.id == theme.id);
                let mut replacement = theme.clone();
                if let Some(collection) = existing.and_then(|existing| existing.collection.clone())
                {
                    replacement.collection = Some(collection);
                }
                update_theme(&themes, replacement)
            } else {
                versioned_copy(&theme, &themes, preferred.as_deref())
                    .and_then(|copy| install_theme(&themes, copy))
            };
            match result {
                Ok(next) => themes = next,
                Err(error) => failures.push(m::theme_file_failure(
                    &theme.label,
                    theme_error_message(&error),
                )),
            }
        }
        self.apply(cx, move |look| look.custom_themes = themes);
        if !failures.is_empty() {
            let mut all: Vec<String> = self.error.take().into_iter().collect();
            all.extend(failures);
            self.error = Some(all.join("; "));
        } else if self.error.is_none() {
            cx.emit(ThemeImportEvent::Close);
        }
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let text = self.json.read(cx).value().to_string();
        if let Some(too_large) = oversized(text.len() as u64) {
            self.error = Some(too_large);
            cx.notify();
            return;
        }
        match parse_theme_import(&text, &m::vscode_theme()) {
            Ok(theme) => {
                let installed = Self::installed(cx);
                if installed.iter().any(|existing| existing.id == theme.id) {
                    self.error = None;
                    self.conflicts = Some(vec![theme]);
                    cx.notify();
                    return;
                }
                match install_theme(&installed, theme.clone()) {
                    Ok(next) => {
                        self.apply(cx, move |look| {
                            let selection = choose_card(&look.selection(), &theme);
                            look.custom_themes = next;
                            look.set_selection(selection);
                        });
                        cx.emit(ThemeImportEvent::Close);
                    }
                    Err(error) => self.error = Some(theme_error_message(&error)),
                }
            }
            Err(error) => self.error = Some(theme_error_message(&error)),
        }
        cx.notify();
    }

    fn search_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let installed = Self::installed(cx);
        let busy = self.installing.is_some();
        let has_results = self
            .results
            .as_ref()
            .is_some_and(|results| !results.is_empty());
        let suggestions = SUGGESTED_SEARCHES.map(|suggestion| {
            Button::new(SharedString::from(format!("suggest-{suggestion}")))
                .label(suggestion)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Xs)
                .disabled(busy)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.search_for(suggestion, window, cx)),
                )
        });
        let sort = Select::new("theme-sort", sort_label(self.sort))
            .disabled(busy)
            .options(SORTS.map(|option| {
                let this = cx.weak_entity();
                SelectOption::new(
                    SharedString::from(format!("theme-sort-{option:?}")),
                    sort_label(option),
                    option == self.sort,
                )
                .on_select(move |_, cx| {
                    this.update(cx, |this, cx| {
                        this.sort = option;
                        let text = this.query.read(cx).value().to_string();
                        this.run_search(&text, cx);
                    })
                    .ok();
                })
            }));
        let icons: HashMap<String, Arc<Image>> = self
            .results
            .clone()
            .unwrap_or_default()
            .iter()
            .filter_map(|extension| extension.icon_url.clone())
            .filter_map(|url| Some((url.clone(), self.icon(&url, cx)?)))
            .collect();
        let results = self.results.clone().map(|results| {
            if results.is_empty() {
                div()
                    .flex()
                    .flex_col()
                    .min_h_40()
                    .items_center()
                    .justify_center()
                    .rounded_2xl()
                    .border_1()
                    .border_dashed()
                    .border_color(colors.border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(m::no_open_source_themes()),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(m::try_broader_search()),
                    )
                    .into_any_element()
            } else {
                div()
                    .grid()
                    .grid_cols(2)
                    .gap_2()
                    .children(results.into_iter().map(|extension| {
                        let installing = self.installing.as_deref() == Some(extension.id.as_str());
                        let has_it = installed.iter().any(|theme| {
                            theme
                                .collection
                                .as_ref()
                                .map(|collection| collection.id.as_str())
                                == Some(extension.collection_id.as_str())
                        });
                        let (icon, label) = match (installing, has_it) {
                            (true, true) => (IconName::LoaderCircle, m::updating()),
                            (true, false) => (IconName::LoaderCircle, m::installing()),
                            (false, true) => (IconName::RefreshCw, m::update()),
                            (false, false) => (IconName::Plus, m::install()),
                        };
                        let source = extension.source_url.clone();
                        let chosen = extension.clone();
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .gap_3()
                            .p_3()
                            .rounded_xl()
                            .border_1()
                            .border_color(colors.border.opacity(0.7))
                            .bg(colors.card.opacity(0.6))
                            .child(
                                div()
                                    .flex()
                                    .min_w_0()
                                    .gap_3()
                                    .child(
                                        div()
                                            .size_10()
                                            .flex()
                                            .flex_none()
                                            .items_center()
                                            .justify_center()
                                            .rounded_lg()
                                            .overflow_hidden()
                                            .bg(colors.muted)
                                            .text_color(colors.muted_foreground)
                                            .child(
                                                match extension
                                                    .icon_url
                                                    .as_ref()
                                                    .and_then(|url| icons.get(url))
                                                {
                                                    Some(icon) => img(icon.clone())
                                                        .size_full()
                                                        .object_fit(ObjectFit::Cover)
                                                        .into_any_element(),
                                                    None => Icon::new(IconName::Palette)
                                                        .size_4()
                                                        .into_any_element(),
                                                },
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(extension.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_xs()
                                                    .text_color(colors.muted_foreground)
                                                    .child(m::extension_byline(
                                                        compact(extension.download_count),
                                                        &extension.publisher,
                                                    )),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .min_h_8()
                                    .text_xs()
                                    .line_height(rems(1.))
                                    .line_clamp(2)
                                    .text_color(colors.muted_foreground)
                                    .child(if extension.description.is_empty() {
                                        m::community_theme_description()
                                    } else {
                                        extension.description.clone()
                                    }),
                            )
                            .child(
                                div()
                                    .mt_auto()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .child(div().when_some(source, |this, url| {
                                        this.child(
                                            Button::new(SharedString::from(format!(
                                                "source-{}",
                                                extension.id
                                            )))
                                            .icon(IconName::ExternalLink)
                                            .variant(ButtonVariant::Ghost)
                                            .size(ButtonSize::IconXs)
                                            .tooltip(m::view_source_for(&extension.name))
                                            .on_click(move |_, _, cx| cx.open_url(&url)),
                                        )
                                    }))
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "install-{}",
                                            extension.id
                                        )))
                                        .icon(icon)
                                        .loading(installing)
                                        .label(label)
                                        .variant(ButtonVariant::Outline)
                                        .size(ButtonSize::Xs)
                                        .disabled(busy)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.install(chosen.clone(), false, cx)
                                            }),
                                        ),
                                    ),
                            )
                    }))
                    .into_any_element()
            }
        });
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(m::search_community_themes()),
                    )
                    .child(
                        div()
                            .mt(px(2.))
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(m::search_community_themes_description()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h_9()
                    .px_2p5()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.input)
                    .child(if self.searching {
                        spinner(px(16.), colors.muted_foreground)
                    } else {
                        Icon::new(IconName::Search)
                            .size_4()
                            .flex_none()
                            .text_color(colors.muted_foreground)
                            .into_any_element()
                    })
                    .child(
                        div()
                            .flex_1()
                            .child(text_input(&self.query).appearance(false)),
                    ),
            )
            .when(!self.searching || self.results.is_some(), |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .min_w_0()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(m::popular()),
                                )
                                .children(suggestions),
                        )
                        .when(has_results, |this| {
                            this.child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(colors.muted_foreground)
                                            .child(m::sort()),
                                    )
                                    .child(sort),
                            )
                        }),
                )
            })
            .when_some(self.search_error.clone(), |this, error| {
                this.child(error_box(error, &colors))
            })
            .when(self.searching && self.results.is_none(), |this| {
                this.child(
                    div()
                        .flex()
                        .min_h_20()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .text_sm()
                        .text_color(colors.muted_foreground)
                        .child(spinner(px(16.), colors.muted_foreground))
                        .child(m::searching_themes()),
                )
            })
            .children(results)
    }

    fn file_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let mono = cx.theme().mono_font_family.clone();
        let empty = self.json.read(cx).value().trim().is_empty();
        let drop_target = self.drop_target;
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .id("theme-file-drop")
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_3()
                    .py_3()
                    .rounded_xl()
                    .border_1()
                    .border_dashed()
                    .map(|this| {
                        if drop_target {
                            this.border_color(colors.ring)
                                .bg(colors.accent.opacity(0.4))
                        } else {
                            this.border_color(colors.border)
                                .bg(colors.muted.opacity(0.2))
                        }
                    })
                    .on_drag_move::<ExternalPaths>(cx.listener(
                        |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                            let inside = event.bounds.contains(&event.event.position);
                            if inside != this.drop_target {
                                this.drop_target = inside;
                                cx.notify();
                            }
                        },
                    ))
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                        this.drop_target = false;
                        this.read_files(paths.paths().to_vec(), window, cx);
                    }))
                    .child(
                        div()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(m::theme_file()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(
                                        self.file_name
                                            .clone()
                                            .unwrap_or_else(m::theme_file_drop_hint),
                                    ),
                            ),
                    )
                    .child(
                        Button::new("choose-theme-files")
                            .label(m::choose_files())
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .disabled(self.reading)
                            .loading(self.reading)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.choose_files(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(m::theme_json()),
                    )
                    .child(
                        div()
                            .font_family(mono)
                            .text_xs()
                            .child(code_editor(&self.json).h(px(240.))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("import-cancel")
                            .label(m::cancel())
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ThemeImportEvent::Close))),
                    )
                    .child(
                        Button::new("import-add")
                            .icon(IconName::Plus)
                            .label(m::add_theme())
                            .disabled(empty || self.reading)
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
            )
    }

    fn conflict_section(
        &mut self,
        conflicts: &[ThemeDefinition],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let names = conflicts
            .iter()
            .map(|theme| theme.label.clone())
            .collect::<Vec<_>>()
            .join(", ");
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .p_3()
                    .rounded_xl()
                    .border_1()
                    .border_color(colors.border.opacity(0.7))
                    .bg(colors.muted.opacity(0.3))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(m::theme_already_installed()),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(names),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("conflict-update")
                            .label(m::update_existing())
                            .size(ButtonSize::Sm)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.resolve_conflicts(true, cx)),
                            ),
                    )
                    .child(
                        Button::new("conflict-copy")
                            .label(m::keep_both())
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.resolve_conflicts(false, cx)),
                            ),
                    )
                    .child(
                        Button::new("conflict-back")
                            .label(m::back())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.conflicts = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("conflict-cancel")
                            .label(m::cancel())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ThemeImportEvent::Close))),
                    ),
            )
    }
}

fn error_box(error: String, colors: &Colors) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .rounded_lg()
        .border_1()
        .border_color(colors.destructive.opacity(0.3))
        .bg(colors.destructive.opacity(0.1))
        .text_sm()
        .text_color(colors.destructive)
        .child(error)
}

impl ThemeImport {
    /// The extension's icon once it loaded; starts loading it the first time.
    fn icon(&mut self, url: &str, cx: &mut Context<Self>) -> Option<Arc<Image>> {
        if let Some(icon) = self.icons.get(url) {
            return icon.clone();
        }
        self.icons.insert(url.to_owned(), None);
        let url = url.to_owned();
        cx.spawn(async move |this, cx| {
            let fetched = url.clone();
            let icon = cx
                .background_spawn(async move {
                    let bytes = hoshi_app::theme_import::extension_icon(&fetched)?;
                    let format = image_format(&bytes)?;
                    Some(Arc::new(Image::from_bytes(format, bytes)))
                })
                .await;
            if icon.is_some() {
                this.update(cx, |this, cx| {
                    this.icons.insert(url, icon);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        None
    }
}

/// What an icon's bytes are, by their first bytes.
fn image_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF8") {
        Some(ImageFormat::Gif)
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else if String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).contains("<svg") {
        Some(ImageFormat::Svg)
    } else {
        None
    }
}

impl Render for ThemeImport {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let lower = match self.conflicts.clone() {
            Some(conflicts) => self.conflict_section(&conflicts, cx).into_any_element(),
            None => self.file_section(cx).into_any_element(),
        };
        let pending = self.pending_update.clone();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(self.search_section(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().h_px().flex_1().bg(colors.border))
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(m::or().to_uppercase()),
                    )
                    .child(div().h_px().flex_1().bg(colors.border)),
            )
            .child(lower)
            .when_some(self.error.clone(), |this, error| {
                this.child(error_box(error, &colors))
            })
            .when_some(pending, |this, extension| {
                this.child(
                    Dialog::new("update-theme")
                        .alert()
                        .open(true)
                        .title(m::update_theme_confirm(&extension.name))
                        .description(m::update_theme_description())
                        .on_open_change(cx.listener(|this, change: &OpenChange, _, cx| {
                            if !change.open {
                                this.pending_update = None;
                                cx.notify();
                            }
                        }))
                        .footer(
                            DialogButton::new("update-cancel")
                                .label(m::cancel())
                                .variant(DialogButtonVariant::Outline)
                                .close(),
                        )
                        .footer(
                            DialogButton::new("update-confirm")
                                .label(m::update_theme())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.pending_update = None;
                                    this.install(extension.clone(), true, cx);
                                })),
                        ),
                )
            })
    }
}

impl SettingsView {
    pub(super) fn open_theme_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let import = cx.new(|cx| ThemeImport::new(window, cx));
        let subscription = cx.subscribe(&import, |this, _, event, cx| match event {
            ThemeImportEvent::Close => {
                this.looks.import = None;
                cx.notify();
            }
        });
        self.looks.import = Some((import, subscription));
        cx.notify();
    }

    pub(super) fn theme_import_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (import, _) = self.looks.import.as_ref()?;
        Some(
            Dialog::new("add-theme")
                .open(true)
                .size(DialogSize::Custom(px(768.)))
                .title(m::add_a_theme())
                .close_label(m::cancel())
                .on_open_change(cx.listener(|this, change: &OpenChange, _, cx| {
                    if !change.open {
                        this.looks.import = None;
                        cx.notify();
                    }
                }))
                .child(import.clone())
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{compact, size_label, versioned_copy};
    use crate::themes::model::{ThemeAppearance, ThemeDefinition, standard_colors};

    #[test]
    fn download_counts_read_compactly() {
        assert_eq!(compact(950), "950");
        assert_eq!(compact(1_000), "1K");
        assert_eq!(compact(1_234), "1.2K");
        assert_eq!(compact(3_400_000), "3.4M");
        assert_eq!(size_label(300 * 1024), "300 KB");
    }

    #[test]
    fn a_copy_takes_the_first_free_name() {
        let theme = ThemeDefinition::new(
            "nord",
            "Nord",
            ThemeAppearance::Dark,
            standard_colors(ThemeAppearance::Dark).clone(),
        );
        let copy = versioned_copy(&theme, std::slice::from_ref(&theme), None).unwrap();
        assert_eq!(copy.label, "Nord (1)");
        let named =
            versioned_copy(&theme, std::slice::from_ref(&theme), Some("Nord Frost")).unwrap();
        assert_eq!(named.label, "Nord Frost");
    }
}
