use super::{SettingsView, field};
use crate::m;
use gpui_kit::*;
use hoshi_core::{AppSettings, LinkTarget, PageColorScheme, PageSize, SearchEngine};
use hoshi_ui::prelude::*;

const ZOOMS: [u32; 11] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200];

const ENGINES: [SearchEngine; 7] = [
    SearchEngine::DuckDuckGo,
    SearchEngine::Google,
    SearchEngine::Bing,
    SearchEngine::Brave,
    SearchEngine::Kagi,
    SearchEngine::Ecosia,
    SearchEngine::Custom,
];

fn link_label(target: LinkTarget) -> String {
    match target {
        LinkTarget::System => m::browser_links_system(),
        LinkTarget::Hoshi => m::browser_links_hoshi(),
    }
}

fn engine_label(engine: SearchEngine) -> String {
    match engine {
        SearchEngine::DuckDuckGo => "DuckDuckGo".into(),
        SearchEngine::Google => "Google".into(),
        SearchEngine::Bing => "Bing".into(),
        SearchEngine::Brave => "Brave Search".into(),
        SearchEngine::Kagi => "Kagi".into(),
        SearchEngine::Ecosia => "Ecosia".into(),
        SearchEngine::Custom => m::browser_search_custom(),
    }
}

fn size_label(size: PageSize) -> String {
    match size {
        PageSize::Fill => m::browser_size_fill(),
        PageSize::Desktop => format!("{} · 1280 × 800", m::browser_size_desktop()),
        PageSize::Tablet => format!("{} · 768 × 1024", m::browser_size_tablet()),
        PageSize::Phone => format!("{} · 390 × 844", m::browser_size_phone()),
    }
}

fn scheme_label(scheme: PageColorScheme) -> String {
    match scheme {
        PageColorScheme::System => m::theme_system(),
        PageColorScheme::Light => m::theme_light(),
        PageColorScheme::Dark => m::theme_dark(),
    }
}

/// An http(s) URL with `%s` for the terms, as the engine takes it.
pub fn usable_search_url(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://")) && url.contains("%s")
}

impl SettingsView {
    /// A select whose options set one field of the browser settings.
    fn browser_choice<T: Copy + PartialEq + std::fmt::Debug + 'static>(
        &self,
        id: &'static str,
        current: T,
        values: impl IntoIterator<Item = T>,
        label: impl Fn(T) -> String,
        set: fn(&mut AppSettings, T),
        cx: &Context<Self>,
    ) -> Select {
        Select::new(id, label(current)).options(values.into_iter().map(|value| {
            SelectOption::new(
                SharedString::from(format!("{id}-{value:?}")),
                label(value),
                value == current,
            )
            .on_select(self.setter(cx, move |settings| set(settings, value)))
        }))
    }

    pub(super) fn browser(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let browser = self.settings(cx).browser.clone();
        let mut zooms = ZOOMS.to_vec();
        if !zooms.contains(&browser.zoom) {
            zooms.push(browser.zoom);
            zooms.sort_unstable();
        }

        let mut browsing = SettingsGroup::new()
            .title(m::browser_browsing_heading())
            .child(
                SettingsRow::new(m::browser_open_links_in())
                    .description(m::browser_open_links_in_description())
                    .control(self.browser_choice(
                        "settings-open-links-in",
                        browser.open_links_in,
                        [LinkTarget::System, LinkTarget::Hoshi],
                        link_label,
                        |settings, value| settings.browser.open_links_in = value,
                        cx,
                    )),
            )
            .child(
                SettingsRow::new(m::float_agent_pages_setting())
                    .description(m::float_agent_pages_setting_description())
                    .control(
                        Switch::new("settings-float-agent-pages")
                            .checked(browser.float_agent_pages)
                            .on_change(self.toggler(cx, |settings, on| {
                                settings.browser.float_agent_pages = on
                            })),
                    ),
            )
            .child(
                SettingsRow::new(m::browser_search_engine())
                    .description(m::browser_search_engine_description())
                    .control(self.browser_choice(
                        "settings-search-engine",
                        browser.search_engine,
                        ENGINES,
                        engine_label,
                        |settings, value| settings.browser.search_engine = value,
                        cx,
                    )),
            );
        if browser.search_engine == SearchEngine::Custom {
            browsing = browsing.child(
                SettingsRow::new(m::browser_custom_search_url())
                    .description(if usable_search_url(&browser.custom_search_url) {
                        m::browser_custom_search_url_description()
                    } else {
                        m::browser_custom_search_url_invalid()
                    })
                    .control(field(&self.custom_search_url, px(256.))),
            );
        }
        let advanced = self.advanced_group(
            "browser",
            [
                SettingsRow::new(m::browser_page_size())
                    .description(m::browser_page_size_description())
                    .control(self.browser_choice(
                        "settings-page-size",
                        browser.page_size,
                        [
                            PageSize::Fill,
                            PageSize::Desktop,
                            PageSize::Tablet,
                            PageSize::Phone,
                        ],
                        size_label,
                        |settings, value| settings.browser.page_size = value,
                        cx,
                    ))
                    .into_any_element(),
                SettingsRow::new(m::browser_zoom())
                    .description(m::browser_zoom_description())
                    .control(self.browser_choice(
                        "settings-page-zoom",
                        browser.zoom,
                        zooms,
                        |zoom| format!("{zoom}%"),
                        |settings, value| settings.browser.zoom = value,
                        cx,
                    ))
                    .into_any_element(),
                SettingsRow::new(m::browser_color_scheme())
                    .description(m::browser_color_scheme_description())
                    .control(self.browser_choice(
                        "settings-page-color-scheme",
                        browser.color_scheme,
                        [
                            PageColorScheme::System,
                            PageColorScheme::Light,
                            PageColorScheme::Dark,
                        ],
                        scheme_label,
                        |settings, value| settings.browser.color_scheme = value,
                        cx,
                    ))
                    .into_any_element(),
            ],
            cx,
        );

        let mut page = vec![
            SettingsGroup::new()
                .title(m::browser_agent_access_heading())
                .child(
                    SettingsRow::new(m::browser_agent_access())
                        .description(m::browser_agent_access_description())
                        .control(
                            Switch::new("settings-agent-browser-access")
                                .checked(browser.agent_access)
                                .on_change(self.toggler(cx, |settings, on| {
                                    settings.browser.agent_access = on
                                })),
                        ),
                )
                .into_any_element(),
            browsing.child(advanced).into_any_element(),
        ];
        page.extend(self.save_error(cx));
        page
    }
}

#[cfg(test)]
mod tests {
    use super::usable_search_url;

    #[test]
    fn a_search_url_needs_http_and_the_terms() {
        assert!(usable_search_url("https://search.example/?q=%s"));
        assert!(!usable_search_url("https://search.example/?q="));
        assert!(!usable_search_url("ftp://search.example/?q=%s"));
    }
}
