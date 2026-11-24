//! The model picker: favorites and providers on a rail, a search, the models with Ctrl+digit
//! shortcuts, and the chosen model's effort and speed under them.

use super::Composer;
use super::footer::{FooterMenu, Target, effective_effort, effort_label, model_options, tag};
use crate::m;
use crate::state::{ModelChoice, Workspace};
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;
use hoshi_core::{FavoriteModel, ModelInfo, ProviderModels};
use hoshi_ui::assets::brand;
use hoshi_ui::prelude::*;

const SHORTCUTS: usize = 9;

/// What the list shows without a search: the starred models, or one provider's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModelTab {
    Favorites,
    Provider(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ModelRow {
    pub provider: String,
    pub model: ModelInfo,
}

impl ModelRow {
    fn favorite(&self) -> FavoriteModel {
        FavoriteModel {
            provider: self.provider.clone(),
            model: self.model.slug.clone(),
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub(super) struct Listing {
    pub rows: Vec<ModelRow>,
    /// Legacy models left out until "Show more".
    pub hidden_legacy: usize,
}

impl Listing {
    /// The rows Ctrl+1 to Ctrl+9 pick, by index in `rows`.
    pub fn shortcuts(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| !row.model.update_required)
            .map(|(index, _)| index)
            .take(SHORTCUTS)
            .collect()
    }
}

/// The tab a picker opens on: the favorites when there are some, else `current`'s provider,
/// or the first one listed.
pub(crate) fn default_tab(
    catalogs: &[ProviderModels],
    favorites: &[FavoriteModel],
    current: &str,
) -> ModelTab {
    if !favorites.is_empty() {
        return ModelTab::Favorites;
    }
    catalogs
        .iter()
        .find(|catalog| catalog.provider == current)
        .or(catalogs.first())
        .map_or(ModelTab::Favorites, |catalog| {
            ModelTab::Provider(catalog.provider.clone())
        })
}

fn matches(model: &ModelInfo, query: &str) -> bool {
    let haystack = format!("{} {}", model.name, model.short_name).to_lowercase();
    query
        .split_whitespace()
        .all(|token| haystack.contains(token))
}

/// The models listed for `tab`, or for `query` across every provider, starred ones first.
pub(crate) fn listing(
    catalogs: &[ProviderModels],
    favorites: &[FavoriteModel],
    tab: &ModelTab,
    query: &str,
    show_legacy: bool,
) -> Listing {
    let all: Vec<ModelRow> = catalogs
        .iter()
        .flat_map(|catalog| {
            catalog.models.iter().map(|model| ModelRow {
                provider: catalog.provider.clone(),
                model: model.clone(),
            })
        })
        .collect();
    let starred = |row: &ModelRow| favorites.contains(&row.favorite());
    let needle = query.trim().to_lowercase();
    if !needle.is_empty() {
        let (first, rest): (Vec<_>, Vec<_>) = all
            .into_iter()
            .filter(|row| matches(&row.model, &needle))
            .partition(starred);
        return Listing {
            rows: first.into_iter().chain(rest).collect(),
            hidden_legacy: 0,
        };
    }
    match tab {
        ModelTab::Favorites => Listing {
            rows: favorites
                .iter()
                .filter_map(|favorite| all.iter().find(|row| *favorite == row.favorite()).cloned())
                .collect(),
            hidden_legacy: 0,
        },
        ModelTab::Provider(provider) => {
            let (first, rest): (Vec<_>, Vec<_>) = all
                .into_iter()
                .filter(|row| &row.provider == provider)
                .partition(starred);
            let (legacy, current): (Vec<_>, Vec<_>) =
                rest.into_iter().partition(|row| row.model.legacy);
            Listing {
                hidden_legacy: legacy.len(),
                rows: first
                    .into_iter()
                    .chain(current)
                    .chain(legacy.into_iter().filter(|_| show_legacy))
                    .collect(),
            }
        }
    }
}

pub(super) fn provider_label(provider: &str) -> String {
    match provider {
        "claude" | "claudeCode" => m::claude_code(),
        "copilot" => m::copilot(),
        "codex" => m::codex(),
        other => other.to_owned(),
    }
}

/// The picker's trigger names a model by its short catalog name, else its slug, else its
/// provider.
pub(super) fn trigger_name(workspace: &Workspace, provider: &str, slug: Option<&str>) -> String {
    workspace
        .chosen_model(provider, slug)
        .map(|model| model.short_name.clone())
        .or_else(|| slug.map(str::to_owned))
        .unwrap_or_else(|| provider_label(provider))
}

/// The provider's mark, drawn in its tint.
pub(crate) fn provider_mark(provider: &str) -> &'static str {
    match provider {
        "copilot" => brand::COPILOT,
        "codex" => brand::CODEX,
        _ => brand::CLAUDE,
    }
}

pub(crate) fn provider_tint(provider: &str, colors: &Colors) -> Hsla {
    match provider {
        "copilot" => colors.provider_copilot,
        "codex" => colors.provider_codex,
        _ => colors.provider_claude,
    }
}

/// What owns a model picker: its search, tab and legacy toggle, and what a pick does.
pub(crate) trait PickerHost: Sized + 'static {
    fn picker_search(&self) -> Entity<InputState>;
    fn set_picker_tab(&mut self, tab: ModelTab);
    fn toggle_picker_legacy(&mut self);
    fn picker_workspace(&self) -> Entity<Workspace>;
    fn pick_model(
        &mut self,
        provider: String,
        slug: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    /// The chosen model's effort, speed or Ultracode, from the options under the list.
    fn change_choice(&mut self, change: impl FnOnce(&mut ModelChoice), cx: &mut Context<Self>);
}

impl PickerHost for Composer {
    fn change_choice(&mut self, change: impl FnOnce(&mut ModelChoice), cx: &mut Context<Self>) {
        self.set_model(change, cx);
    }

    fn picker_search(&self) -> Entity<InputState> {
        self.model_search.clone()
    }

    fn set_picker_tab(&mut self, tab: ModelTab) {
        self.model_tab = Some(tab);
    }

    fn toggle_picker_legacy(&mut self) {
        self.show_legacy = !self.show_legacy;
    }

    fn picker_workspace(&self) -> Entity<Workspace> {
        self.workspace.clone()
    }

    fn pick_model(
        &mut self,
        provider: String,
        slug: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_model(
            |choice| {
                choice.provider = provider;
                choice.model = Some(slug);
            },
            cx,
        );
        self.menu = None;
        self.focus(window, cx);
        cx.notify();
    }
}

impl Composer {
    /// The models the target can move to; a started thread moves with its next turn.
    pub(super) fn picker_catalogs(&self, cx: &App) -> Vec<ProviderModels> {
        self.workspace.read(cx).models.clone()
    }

    fn current_listing(&self, cx: &App) -> Listing {
        let catalogs = self.picker_catalogs(cx);
        let current = self
            .target(cx)
            .map(|target| target.choice.provider)
            .unwrap_or_default();
        let workspace = self.workspace.read(cx);
        let favorites = &workspace.settings.favorite_models;
        let tab = self
            .model_tab
            .clone()
            .unwrap_or_else(|| default_tab(&catalogs, favorites, &current));
        listing(
            &catalogs,
            favorites,
            &tab,
            &self.model_search.read(cx).value(),
            self.show_legacy,
        )
    }

    pub(super) fn open_model_menu(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if open {
            self.model_tab = None;
            self.model_search
                .update(cx, |search, cx| search.set_value("", window, cx));
            let legacy = self.target(cx).is_some_and(|target| {
                self.workspace
                    .read(cx)
                    .chosen_model(&target.choice.provider, target.choice.model.as_deref())
                    .is_some_and(|model| model.legacy)
            });
            self.show_legacy = legacy;
        }
        self.set_menu(FooterMenu::Model, open, cx);
    }

    /// Picks the `index`th shortcut row: Ctrl+1 is the first, Enter in the search too.
    pub(super) fn pick_shortcut(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let listing = self.current_listing(cx);
        let Some(row) = listing
            .shortcuts()
            .get(index)
            .and_then(|row| listing.rows.get(*row))
        else {
            return false;
        };
        self.pick_model(row.provider.clone(), row.model.slug.clone(), window, cx);
        true
    }

    /// Ctrl+digit while the picker is open.
    pub(super) fn model_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let modifiers = &keystroke.modifiers;
        if !(modifiers.control || modifiers.platform) || modifiers.alt || modifiers.shift {
            return false;
        }
        match keystroke.key.parse::<usize>() {
            Ok(digit @ 1..=9) => self.pick_shortcut(digit - 1, window, cx),
            _ => false,
        }
    }

    pub(super) fn model_menu(&mut self, target: &Target, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let catalogs = self.picker_catalogs(cx);
        let workspace = self.workspace.read(cx);
        let favorites = workspace.settings.favorite_models.clone();
        let provider = target.choice.provider.clone();
        let selected = workspace
            .chosen_model(&provider, target.choice.model.as_deref())
            .cloned();
        let selected_slug = selected.as_ref().map(|model| model.slug.clone());
        let effort = effective_effort(selected.as_ref(), target.choice.effort);
        let name = trigger_name(workspace, &provider, target.choice.model.as_deref());
        let (mark, tint) = (provider_mark(&provider), provider_tint(&provider, &colors));
        let choice = ModelChoice {
            provider: provider.clone(),
            model: target.choice.model.clone(),
            effort,
            fast_mode: target.choice.fast_mode,
            ultracode: target.choice.ultracode,
        };
        let query = self.model_search.read(cx).value().to_string();
        let tab = self
            .model_tab
            .clone()
            .unwrap_or_else(|| default_tab(&catalogs, &favorites, &provider));
        let show_legacy = self.show_legacy;
        let search = self.model_search.clone();
        let search_focus = search.read(cx).focus_handle(cx);
        let this = cx.weak_entity();

        let trigger = div()
            .id("model-trigger")
            .flex()
            .items_center()
            .gap_1()
            .h_6()
            .px_2()
            .rounded_md()
            .cursor_pointer()
            .text_xs()
            .hover(|style| style.bg(colors.muted))
            .child(svg().path(mark).size_3().flex_none().text_color(tint))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors.foreground)
                    .child(name),
            )
            .when_some(effort, |row, effort| row.child(effort_label(effort)))
            .when(choice.fast_mode, |row| {
                row.child(Icon::new(IconName::Zap).size_3().text_color(tint))
            })
            .when(choice.ultracode, |row| {
                row.child(Icon::new(IconName::Workflow).size_3().text_color(tint))
            })
            .child(Icon::new(IconName::ChevronDown).size_3());
        let muted = colors.muted;

        Popover::new("model-menu")
            .side(Side::Top)
            .align(Align::End)
            .modal(false)
            .w_96()
            .p_0()
            .gap_0()
            .rounded_xl()
            .overflow_hidden()
            .initial_focus(PopoverFocus::Handle(search_focus))
            .open(self.menu == Some(FooterMenu::Model))
            .on_open_change(cx.listener(|this, change: &PopoverChange, window, cx| {
                this.open_model_menu(change.open, window, cx)
            }))
            .trigger(move |open| trigger.when(open, |trigger| trigger.bg(muted)))
            .content(move |_, _, cx| {
                let colors = cx.hoshi().clone();
                let list = PickerList {
                    catalogs: catalogs.clone(),
                    favorites: favorites.clone(),
                    tab: tab.clone(),
                    query: query.clone(),
                    show_legacy,
                    selected: selected_slug.clone().map(|slug| (provider.clone(), slug)),
                    search: search.clone(),
                }
                .render(this.clone(), &colors);
                let options = selected
                    .as_ref()
                    .and_then(|model| model_options(model, &choice, this.clone(), &colors));
                div().flex().flex_col().child(list).children(options)
            })
            .into_any_element()
    }
}

/// A picker's rail, search and rows, for whatever `H` does with a pick.
pub(crate) struct PickerList {
    pub catalogs: Vec<ProviderModels>,
    pub favorites: Vec<FavoriteModel>,
    pub tab: ModelTab,
    pub query: String,
    pub show_legacy: bool,
    /// The chosen model's provider and slug.
    pub selected: Option<(String, String)>,
    pub search: Entity<InputState>,
}

impl PickerList {
    pub fn render<H: PickerHost>(&self, this: WeakEntity<H>, colors: &Colors) -> Div {
        let Self {
            catalogs,
            favorites,
            tab,
            query,
            show_legacy,
            selected,
            search,
        } = self;
        let searching = !query.trim().is_empty();
        let listing = listing(catalogs, favorites, tab, query, *show_legacy);
        let rows_height = rows_height(catalogs, favorites, *show_legacy);
        let rail = rail(catalogs, tab, searching, this.clone(), colors);
        let heading = if searching {
            m::search_results()
        } else {
            match tab {
                ModelTab::Favorites => m::model_favorites(),
                ModelTab::Provider(provider) => provider_label(provider),
            }
        };
        let empty = listing.rows.is_empty().then(|| {
            div()
                .px_2()
                .py_6()
                .text_center()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(if searching {
                    m::no_matching_models()
                } else if *tab == ModelTab::Favorites {
                    m::no_favorite_models()
                } else {
                    m::no_models()
                })
        });
        let shortcuts = listing.shortcuts();
        let rows = listing.rows.iter().enumerate().map(|(index, row)| {
            let shortcut = shortcuts.iter().position(|shown| *shown == index);
            let chosen = selected.as_ref().is_some_and(|(provider, slug)| {
                row.provider == *provider && row.model.slug == *slug
            });
            model_row(
                index,
                row,
                chosen,
                favorites.contains(&row.favorite()),
                shortcut,
                this.clone(),
                colors,
            )
        });
        let toggle_legacy = (listing.hidden_legacy > 0).then(|| {
            let this = this.clone();
            div()
                .id("model-legacy")
                .px_2()
                .py_1p5()
                .rounded_lg()
                .cursor_pointer()
                .text_xs()
                .text_color(colors.muted_foreground)
                .hover(|style| style.text_color(colors.foreground))
                .child(if *show_legacy {
                    m::model_show_less()
                } else {
                    m::model_show_more()
                })
                .on_click(move |_, _, cx| {
                    this.update(cx, |this, cx| {
                        this.toggle_picker_legacy();
                        cx.notify();
                    })
                    .ok();
                })
        });
        let list = div()
            .min_w_0()
            .flex_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h_9()
                    .px_3()
                    .border_b_1()
                    .border_color(colors.border)
                    .text_color(colors.muted_foreground)
                    .child(Icon::new(IconName::Search).size_3p5().flex_none())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .child(Input::new(search).appearance(false)),
                    ),
            )
            .child(
                div()
                    .id("model-rows")
                    .h(rows_height)
                    .overflow_y_scroll()
                    .p_1()
                    .child(
                        div()
                            .px_2()
                            .pt_1p5()
                            .pb_1()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors.muted_foreground)
                            .child(heading),
                    )
                    .children(empty)
                    .children(rows)
                    .children(toggle_legacy),
            );
        div().flex().child(rail).child(list)
    }
}

/// The list's height: what its fullest tab needs, up to a cap, so switching tabs or
/// searching never resizes the picker.
fn rows_height(
    catalogs: &[ProviderModels],
    favorites: &[FavoriteModel],
    show_legacy: bool,
) -> Pixels {
    const PADDING: f32 = 8.;
    const HEADING: f32 = 26.;
    const ROW: f32 = 40.;
    const EMPTY: f32 = 64.;
    const TOGGLE: f32 = 28.;
    const MAX: f32 = 320.;
    let tabs = std::iter::once(ModelTab::Favorites).chain(
        catalogs
            .iter()
            .map(|catalog| ModelTab::Provider(catalog.provider.clone())),
    );
    let tallest = tabs
        .map(|tab| {
            let shown = listing(catalogs, favorites, &tab, "", show_legacy);
            let rows = match shown.rows.len() {
                0 => EMPTY,
                count => count as f32 * ROW,
            };
            let has_legacy =
                shown.hidden_legacy > 0 || shown.rows.iter().any(|row| row.model.legacy);
            PADDING + HEADING + rows + if has_legacy { TOGGLE } else { 0. }
        })
        .fold(0., f32::max);
    px(tallest.min(MAX))
}

fn rail<H: PickerHost>(
    catalogs: &[ProviderModels],
    tab: &ModelTab,
    searching: bool,
    this: WeakEntity<H>,
    colors: &Colors,
) -> Div {
    let favorites = rail_button(
        "model-tab-favorites",
        m::model_favorites(),
        !searching && *tab == ModelTab::Favorites,
        colors.favorite,
        brand::STAR_FILLED,
        ModelTab::Favorites,
        this.clone(),
        colors,
    );
    let providers = catalogs.iter().map(|catalog| {
        let provider = ModelTab::Provider(catalog.provider.clone());
        rail_button(
            SharedString::from(format!("model-tab-{}", catalog.provider)),
            provider_label(&catalog.provider),
            !searching && *tab == provider,
            provider_tint(&catalog.provider, colors),
            provider_mark(&catalog.provider),
            provider,
            this.clone(),
            colors,
        )
    });
    div()
        .flex()
        .flex_col()
        .flex_none()
        .items_center()
        .gap_1()
        .w_11()
        .py_1p5()
        .border_r_1()
        .border_color(colors.border)
        .bg(colors.muted.opacity(0.4))
        .child(favorites)
        .child(div().my_0p5().h_px().w_4().bg(colors.border))
        .children(providers)
}

#[allow(clippy::too_many_arguments)]
fn rail_button<H: PickerHost>(
    id: impl Into<ElementId>,
    label: String,
    active: bool,
    tint: Hsla,
    icon: &'static str,
    tab: ModelTab,
    this: WeakEntity<H>,
    colors: &Colors,
) -> impl IntoElement {
    let foreground = colors.foreground;
    div()
        .id(id)
        .size_8()
        .flex()
        .items_center()
        .justify_center()
        .rounded_lg()
        .cursor_pointer()
        .border_1()
        .border_color(transparent_black())
        .map(|button| {
            if active {
                button
                    .bg(colors.popover)
                    .border_color(colors.border)
                    .shadow_xs()
                    .text_color(tint)
            } else {
                button
                    .text_color(colors.muted_foreground)
                    .hover(move |style| style.text_color(foreground))
            }
        })
        .child(svg().path(icon).size_4().text_color(if active {
            tint
        } else {
            colors.muted_foreground
        }))
        .on_click(move |_, window, cx| {
            let tab = tab.clone();
            this.update(cx, |this, cx| {
                this.set_picker_tab(tab);
                this.picker_search()
                    .update(cx, |search, cx| search.set_value("", window, cx));
                cx.notify();
            })
            .ok();
        })
        .with_tooltip(Tooltip::new(label).side(Side::Left))
}

fn model_row<H: PickerHost>(
    index: usize,
    row: &ModelRow,
    selected: bool,
    favorite: bool,
    shortcut: Option<usize>,
    this: WeakEntity<H>,
    colors: &Colors,
) -> impl IntoElement {
    let group = SharedString::from(format!("model-row-{index}"));
    let model = &row.model;
    let slug = model.slug.clone();
    let provider = row.provider.clone();
    let tint = provider_tint(&row.provider, colors);
    let mark = provider_mark(&row.provider);
    let star = row.favorite();
    let pick = this.clone();
    let foreground = colors.foreground;
    div()
        .id(("model-row", index))
        .group(group.clone())
        .flex()
        .items_center()
        .gap_1()
        .pr_2()
        .rounded_lg()
        .hover(|style| style.bg(colors.accent.opacity(0.6)))
        .when(selected, |row| row.bg(colors.accent))
        .child(
            div()
                .id(("model-pick", index))
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2p5()
                .py_1p5()
                .pl_2()
                .when(!model.update_required, |row| row.cursor_pointer())
                .when(model.update_required, |row| row.opacity(0.5))
                .child(
                    div()
                        .size_7()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .bg(tint.opacity(0.15))
                        .child(svg().path(mark).size_3p5().text_color(tint)),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(model.name.clone()),
                )
                .when(model.is_new, |row| row.child(tag(m::model_new(), colors)))
                .when(model.update_required, |row| {
                    row.child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(m::model_update_required()),
                    )
                })
                .when(selected, |row| {
                    row.child(div().ml_auto().child(Icon::new(IconName::Check).size_4()))
                })
                .when(!model.update_required, |row| {
                    row.on_click(move |_, window, cx| {
                        let (provider, slug) = (provider.clone(), slug.clone());
                        pick.update(cx, |this, cx| this.pick_model(provider, slug, window, cx))
                            .ok();
                    })
                }),
        )
        .child(
            div()
                .id(("model-star", index))
                .size_6()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .cursor_pointer()
                .map(|button| {
                    if favorite {
                        button.text_color(colors.favorite).child(
                            svg()
                                .path(brand::STAR_FILLED)
                                .size_3p5()
                                .text_color(colors.favorite),
                        )
                    } else {
                        button
                            .text_color(colors.muted_foreground)
                            .opacity(0.)
                            .group_hover(group.clone(), |style| style.opacity(1.))
                            .hover(move |style| style.text_color(foreground))
                            .child(Icon::new(IconName::Star).size_3p5())
                    }
                })
                .on_click(move |_, _, cx| {
                    let star = star.clone();
                    this.update(cx, |this, cx| {
                        this.picker_workspace()
                            .update(cx, |workspace, cx| workspace.toggle_favorite(star, cx));
                    })
                    .ok();
                })
                .with_tooltip(Tooltip::new(if favorite {
                    m::remove_favorite()
                } else {
                    m::add_favorite()
                })),
        )
        .when_some(shortcut, |row, shortcut| {
            row.child(
                div()
                    .flex_none()
                    .h_5()
                    .px_1()
                    .flex()
                    .items_center()
                    .rounded_sm()
                    .border_1()
                    .border_color(colors.border)
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors.muted_foreground)
                    .child(m::model_shortcut(shortcut + 1)),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{Listing, ModelTab, default_tab, listing};
    use hoshi_core::{FavoriteModel, ModelInfo, ProviderModels};

    fn model(slug: &str, legacy: bool) -> ModelInfo {
        ModelInfo {
            slug: slug.into(),
            name: format!("Claude {slug}"),
            short_name: slug.into(),
            legacy,
            is_new: false,
            update_required: false,
            efforts: Vec::new(),
            default_effort: None,
            fast_mode: false,
            ultracode: false,
        }
    }

    fn catalog() -> ProviderModels {
        ProviderModels {
            provider: "claudeCode".into(),
            models: vec![
                model("opus", false),
                model("sonnet", false),
                model("haiku", false),
                model("old", true),
            ],
            default_model: Some("opus".into()),
            ..ProviderModels::default()
        }
    }

    fn star(slug: &str) -> FavoriteModel {
        FavoriteModel {
            provider: "claudeCode".into(),
            model: slug.into(),
        }
    }

    fn slugs(listing: &Listing) -> Vec<&str> {
        listing
            .rows
            .iter()
            .map(|row| row.model.slug.as_str())
            .collect()
    }

    #[test]
    fn a_provider_lists_starred_models_first_and_hides_legacy_ones() {
        let catalog = catalog();
        let tab = ModelTab::Provider("claudeCode".into());
        let listing = listing(
            std::slice::from_ref(&catalog),
            &[star("haiku")],
            &tab,
            "",
            false,
        );
        assert_eq!(slugs(&listing), ["haiku", "opus", "sonnet"]);
        assert_eq!(listing.hidden_legacy, 1);

        let shown = super::listing(std::slice::from_ref(&catalog), &[], &tab, "", true);
        assert_eq!(slugs(&shown), ["opus", "sonnet", "haiku", "old"]);
    }

    #[test]
    fn a_search_spans_every_model_and_favorites_keep_their_order() {
        let catalog = catalog();
        let favorites = [star("sonnet"), star("opus")];
        let starred = listing(
            std::slice::from_ref(&catalog),
            &favorites,
            &ModelTab::Favorites,
            "",
            false,
        );
        assert_eq!(slugs(&starred), ["sonnet", "opus"]);

        let found = listing(
            std::slice::from_ref(&catalog),
            &[star("haiku")],
            &ModelTab::Favorites,
            "CLAUDE",
            false,
        );
        assert_eq!(slugs(&found), ["haiku", "opus", "sonnet", "old"]);
        let narrowed = listing(
            std::slice::from_ref(&catalog),
            &[],
            &ModelTab::Favorites,
            "claude op",
            false,
        );
        assert_eq!(slugs(&narrowed), ["opus"]);
    }

    #[test]
    fn the_picker_opens_on_favorites_only_when_there_are_some() {
        let catalog = catalog();
        assert_eq!(
            default_tab(std::slice::from_ref(&catalog), &[], "claudeCode"),
            ModelTab::Provider("claudeCode".into())
        );
        assert_eq!(
            default_tab(
                std::slice::from_ref(&catalog),
                &[star("opus")],
                "claudeCode"
            ),
            ModelTab::Favorites
        );
    }

    #[test]
    fn shortcuts_skip_models_that_need_an_update() {
        let mut catalog = catalog();
        catalog.models[0].update_required = true;
        let tab = ModelTab::Provider("claudeCode".into());
        let listing = listing(std::slice::from_ref(&catalog), &[], &tab, "", false);
        assert_eq!(listing.shortcuts(), [1, 2]);
    }
}

#[cfg(test)]
mod startup_tests {
    use super::trigger_name;
    use crate::state::{Workspace, channel};
    use gpui_kit::{AppContext as _, TestAppContext};
    use hoshi_core::{DEFAULT_PROVIDER, ModelInfo, ProviderModels, TurnFailure};
    use hoshi_providers::{OutputSink, ProviderAdapter, RunningTurn, TurnRequest};
    use std::sync::Arc;

    const SLUG: &str = "claude-haiku-5-5";

    fn catalog(short_name: &str) -> ProviderModels {
        ProviderModels {
            provider: DEFAULT_PROVIDER.into(),
            default_model: Some(SLUG.into()),
            models: vec![ModelInfo {
                slug: SLUG.into(),
                name: format!("Claude {short_name}"),
                short_name: short_name.into(),
                legacy: false,
                is_new: false,
                update_required: false,
                efforts: Vec::new(),
                default_effort: None,
                fast_mode: false,
                ultracode: false,
            }],
            compacts_idle_threads: false,
        }
    }

    /// Lists one model under `0`, its short name.
    struct Listing(&'static str);

    impl ProviderAdapter for Listing {
        fn provider(&self) -> &'static str {
            DEFAULT_PROVIDER
        }

        fn models(&self) -> ProviderModels {
            catalog(self.0)
        }

        fn start_turn(
            &self,
            _request: TurnRequest,
            _sink: OutputSink,
        ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
            Err(TurnFailure::ProviderDisabled)
        }
    }

    #[gpui_kit::test]
    fn the_trigger_names_the_model_the_last_run_listed_before_the_cli_answers(
        cx: &mut TestAppContext,
    ) {
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let home =
            std::env::temp_dir().join(format!("hoshi-native-known-{}", uuid::Uuid::new_v4()));
        let kept = home.join("caches").join("providers");
        std::fs::create_dir_all(&kept).unwrap();
        std::fs::write(
            kept.join(format!("{DEFAULT_PROVIDER}.json")),
            serde_json::to_string(&catalog("Haiku 5.5")).unwrap(),
        )
        .unwrap();
        let (notify, notifications) = channel();
        let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
        engine.set_adapters(vec![Arc::new(Listing("Haiku 5.5 listed"))]);

        let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
        let shown = workspace.read_with(cx, |workspace, _| {
            trigger_name(workspace, DEFAULT_PROVIDER, Some(SLUG))
        });
        assert_eq!(shown, "Haiku 5.5");

        cx.run_until_parked();
        let shown = workspace.read_with(cx, |workspace, _| {
            trigger_name(workspace, DEFAULT_PROVIDER, Some(SLUG))
        });
        assert_eq!(shown, "Haiku 5.5 listed");
        let kept: ProviderModels = serde_json::from_str(
            &std::fs::read_to_string(kept.join(format!("{DEFAULT_PROVIDER}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(kept, catalog("Haiku 5.5 listed"));
    }

    #[gpui_kit::test]
    fn a_model_no_catalog_knows_is_named_by_its_slug(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let home =
            std::env::temp_dir().join(format!("hoshi-native-unknown-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let (notify, notifications) = channel();
        let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());

        let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));

        let shown = workspace.read_with(cx, |workspace, _| {
            trigger_name(workspace, DEFAULT_PROVIDER, Some(SLUG))
        });
        assert_eq!(shown, SLUG);
    }
}
