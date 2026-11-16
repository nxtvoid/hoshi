use crate::m;
use hoshi_ui::prelude::IconName;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    General,
    Appearance,
    Providers,
    Threads,
    Browser,
    Forges,
    About,
}

pub const SECTIONS: [Section; 7] = [
    Section::General,
    Section::Appearance,
    Section::Providers,
    Section::Threads,
    Section::Browser,
    Section::Forges,
    Section::About,
];

impl Section {
    pub fn label(self) -> String {
        match self {
            Self::General => m::settings_general(),
            Self::Appearance => m::settings_appearance(),
            Self::Providers => m::settings_providers(),
            Self::Threads => m::settings_threads(),
            Self::Browser => m::settings_browser(),
            Self::Forges => m::settings_source_control(),
            Self::About => m::settings_about(),
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::General => IconName::Settings,
            Self::Appearance => IconName::Palette,
            Self::Providers => IconName::Plug,
            Self::Threads => IconName::MessagesSquare,
            Self::Browser => IconName::Globe,
            Self::Forges => IconName::GitBranch,
            Self::About => IconName::Info,
        }
    }

    /// For `HOSHI_NATIVE_SETTINGS`, the web route's names.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "general" => Self::General,
            "appearance" => Self::Appearance,
            "providers" => Self::Providers,
            "threads" => Self::Threads,
            "browser" => Self::Browser,
            "forges" => Self::Forges,
            "about" => Self::About,
            _ => return None,
        })
    }
}

/// The sidebar's groups, in order, each with its heading if it has one.
pub fn nav_groups() -> [(Option<String>, &'static [Section]); 3] {
    [
        (None, &[Section::General, Section::Appearance]),
        (
            Some(m::settings_group_agents()),
            &[
                Section::Providers,
                Section::Threads,
                Section::Browser,
                Section::Forges,
            ],
        ),
        (None, &[Section::About]),
    ]
}

/// Every setting a search can find, with the section that holds it.
fn search_items() -> [(String, Section); 33] {
    [
        (m::startup_view(), Section::General),
        (m::notifications_setting(), Section::General),
        (m::theme(), Section::Appearance),
        (m::claude_code(), Section::Providers),
        (m::binary_path(), Section::Providers),
        (m::plan_mode_setting(), Section::Threads),
        (m::settings_new_threads(), Section::Threads),
        (m::new_thread_model_setting(), Section::Threads),
        (m::new_thread_access_setting(), Section::Threads),
        (m::new_thread_workspace_setting(), Section::Threads),
        (m::settings_new_projects(), Section::Threads),
        (m::projects_folder(), Section::Threads),
        (m::clone_into_host_folders(), Section::Threads),
        (m::settings_thread_states(), Section::Threads),
        (m::inactive_after(), Section::Threads),
        (m::delete_merged_worktrees_setting(), Section::Threads),
        (m::continue_after_restart_setting(), Section::Threads),
        (m::resume_after_usage_limit_setting(), Section::Threads),
        (m::snooze_until_limit_resets_setting(), Section::Threads),
        (m::settings_snooze_options(), Section::Threads),
        (m::settings_thread_titles(), Section::Threads),
        (m::thread_titles_generate(), Section::Threads),
        (m::health_check_interval(), Section::Providers),
        (m::compact_idle_threads(), Section::Providers),
        (m::browser_agent_access(), Section::Browser),
        (m::browser_open_links_in(), Section::Browser),
        (m::float_agent_pages_setting(), Section::Browser),
        (m::browser_search_engine(), Section::Browser),
        (m::browser_page_size(), Section::Browser),
        (m::browser_zoom(), Section::Browser),
        (m::browser_color_scheme(), Section::Browser),
        (m::github(), Section::Forges),
        (m::app_version_label(), Section::About),
    ]
}

/// Sections whose name matches, then settings whose title does.
pub fn search(query: &str) -> Vec<(String, Section)> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let sections = SECTIONS
        .into_iter()
        .map(|section| (section.label(), section))
        .filter(|(label, _)| label.to_lowercase().contains(&needle));
    let settings = search_items()
        .into_iter()
        .filter(|(title, _)| title.to_lowercase().contains(&needle));
    sections.chain(settings).collect()
}

#[cfg(test)]
mod tests {
    use super::{Section, search};

    #[test]
    fn an_empty_query_finds_nothing() {
        assert!(search("  ").is_empty());
    }

    #[test]
    fn sections_come_before_settings() {
        let found = search("browser");
        assert_eq!(found[0].1, Section::Browser);
        assert!(found.len() > 1);
        assert!(
            found
                .iter()
                .all(|(_, section)| *section == Section::Browser)
        );
    }

    #[test]
    fn finds_a_setting_by_title_in_any_case() {
        let found = search("ZOOM");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, Section::Browser);
    }

    #[test]
    fn the_moved_project_and_state_rows_are_found_in_threads() {
        for query in [
            "projects folder",
            "inactive after",
            "host folders",
            "new threads",
        ] {
            let found = search(query);
            assert!(!found.is_empty(), "{query}");
            assert!(
                found
                    .iter()
                    .all(|(_, section)| *section == Section::Threads),
                "{query}"
            );
        }
    }

    #[test]
    fn web_route_names_pick_sections() {
        assert_eq!(Section::from_name("forges"), Some(Section::Forges));
        assert_eq!(Section::from_name("nope"), None);
    }
}
