use crate::{AccessMode, Effort, SnoozePreset, default_snooze_presets};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub providers: ProviderSettings,
    /// Shows the composer's Plan toggle.
    pub plan_mode_enabled: bool,
    /// Listed first in the model picker.
    pub favorite_models: Vec<FavoriteModel>,
    pub thread_titles: ThreadTitleSettings,
    pub new_threads: NewThreadSettings,
    pub general: GeneralSettings,
    pub browser: BrowserSettings,
    /// The thread's card shows its provider's usage limits.
    pub usage_limits_in_details: bool,
    /// Where the thread's context window shows.
    pub context_meter: ContextMeterPlace,
    /// When the first-run setup was finished or skipped; `None` until then.
    pub onboarding_completed_at: Option<String>,
}

/// Where the thread's context window shows: its card, with the button that compacts it, or
/// under the composer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextMeterPlace {
    #[default]
    Details,
    Composer,
}

/// The language of the app's own text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Language {
    /// The system's, or English when hoshi doesn't have it.
    #[default]
    System,
    English,
    Spanish,
}

/// What the app shows when it opens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupView {
    #[default]
    Inbox,
    /// The project and thread open when the app last closed.
    LastProject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GeneralSettings {
    pub language: Language,
    pub startup_view: StartupView,
    /// Idle threads untouched for this many days move to Inactive.
    pub inactive_after_days: u32,
    /// A system notification when a thread finishes, fails or asks for input while the
    /// window is in the background.
    pub notifications: bool,
    /// Where new projects and clones go; None is the home folder.
    pub projects_folder: Option<String>,
    /// Clones go to `<projects folder>/<host>/<owner>/<repo>`.
    pub clone_into_host_folders: bool,
    /// Threads whose turn a restart cut short go on by themselves once hoshi starts again.
    pub continue_after_restart: bool,
    /// A thread that hit its plan's usage limit goes on by itself once the limit resets.
    pub resume_after_usage_limit: bool,
    /// A thread that hit its plan's usage limit is snoozed until the limit resets.
    pub snooze_until_limit_resets: bool,
    /// The choices of every snooze menu; menus take the first
    /// [`crate::MAX_SNOOZE_PRESETS`] valid ones.
    pub snooze_presets: Vec<SnoozePreset>,
    /// The worktree folders of threads whose pull request merged are removed by themselves;
    /// their branches stay.
    pub delete_merged_worktrees: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            language: Language::System,
            startup_view: StartupView::Inbox,
            inactive_after_days: 3,
            notifications: true,
            projects_folder: None,
            clone_into_host_folders: false,
            continue_after_restart: true,
            resume_after_usage_limit: false,
            snooze_until_limit_resets: false,
            snooze_presets: default_snooze_presets(),
            delete_merged_worktrees: false,
        }
    }
}

/// What a new thread starts with; each `None` keeps what the last one was started with.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NewThreadSettings {
    pub model: Option<NewThreadModel>,
    pub access_mode: Option<AccessMode>,
    pub workspace: Option<NewThreadWorkspace>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewThreadModel {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default)]
    pub fast_mode: bool,
    #[serde(default)]
    pub ultracode: bool,
}

/// Where a new thread in a project runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NewThreadWorkspace {
    Local,
    NewWorktree,
}

/// hoshi's headless browser: who uses it and how its pages open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrowserSettings {
    /// Agents get the browser's tools. A change reaches an agent when its session next
    /// starts.
    pub agent_access: bool,
    /// A page the agent opens floats over the chat, instead of waiting in the side panel.
    pub float_agent_pages: bool,
    pub open_links_in: LinkTarget,
    /// What the address bar searches when what was typed isn't an address.
    pub search_engine: SearchEngine,
    /// With [`SearchEngine::Custom`]: an http(s) URL with `%s` where the terms go.
    pub custom_search_url: String,
    /// The size new pages open at.
    pub page_size: PageSize,
    /// The zoom new pages open at, in percent.
    pub zoom: u32,
    /// The color scheme pages are told to prefer.
    pub color_scheme: PageColorScheme,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            agent_access: true,
            float_agent_pages: true,
            open_links_in: LinkTarget::default(),
            search_engine: SearchEngine::default(),
            custom_search_url: String::new(),
            page_size: PageSize::default(),
            zoom: 100,
            color_scheme: PageColorScheme::default(),
        }
    }
}

/// Where links in the chat open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkTarget {
    /// The user's default browser.
    #[default]
    System,
    /// The thread's page in hoshi's browser.
    Hoshi,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchEngine {
    #[default]
    DuckDuckGo,
    Google,
    Bing,
    Brave,
    Kagi,
    Ecosia,
    Custom,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageSize {
    /// The side panel's size, while the panel shows the page.
    Fill,
    #[default]
    Desktop,
    Tablet,
    Phone,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageColorScheme {
    /// The browser's own, which follows the system.
    #[default]
    System,
    Light,
    Dark,
}

/// How a new thread gets its title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ThreadTitleSettings {
    /// A small model reads the first message and names the thread; otherwise its
    /// first line does.
    pub generate: bool,
    /// The provider whose model names threads; `None` uses the default provider.
    pub provider: Option<String>,
    /// `None` uses the provider's quick model.
    pub model: Option<String>,
    /// Added to the provider's own instructions, as the user wrote them.
    pub instructions: String,
}

impl Default for ThreadTitleSettings {
    fn default() -> Self {
        Self {
            generate: true,
            provider: None,
            model: None,
            instructions: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteModel {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderSettings {
    pub claude_code: ClaudeCodeSettings,
    /// Settings saved before Copilot was a key ran it added and enabled.
    #[serde(default = "copilot_before_its_key")]
    pub copilot: CopilotSettings,
    pub codex: CodexSettings,
    /// How often, in seconds, the providers' status, versions and models refresh in the
    /// background; 0 never.
    pub health_check_interval_secs: u32,
    /// When an idle Claude thread over [`IDLE_COMPACTION_TOKENS`] compacts.
    pub idle_compaction: IdleCompaction,
}

fn copilot_before_its_key() -> CopilotSettings {
    CopilotSettings {
        added: true,
        enabled: true,
        ..CopilotSettings::default()
    }
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            claude_code: ClaudeCodeSettings::default(),
            copilot: CopilotSettings::default(),
            codex: CodexSettings::default(),
            health_check_interval_secs: 300,
            idle_compaction: IdleCompaction::default(),
        }
    }
}

/// The context size from which an idle Claude thread compacts.
pub const IDLE_COMPACTION_TOKENS: u64 = 200_000;

/// When an idle Claude thread with a large context compacts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IdleCompaction {
    /// Once the prompt cache expired, the composer offers to compact before the next message.
    #[default]
    WhenYouReturn,
    /// Shortly before the prompt cache expires, while the history is still cached.
    BeforeCacheExpires,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CodexSettings {
    /// Listed among the providers; one not added is never run.
    pub added: bool,
    pub enabled: bool,
    /// `None` runs the Codex hoshi installed, else `codex` on the PATH.
    pub binary_path: Option<String>,
    /// Where Codex keeps its configuration and sign-in (`CODEX_HOME`); `None` is its default.
    pub home: Option<String>,
}

impl Default for CodexSettings {
    fn default() -> Self {
        Self {
            added: true,
            enabled: true,
            binary_path: None,
            home: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CopilotSettings {
    /// Listed among the providers; one not added is never run.
    pub added: bool,
    pub enabled: bool,
    pub binary_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClaudeCodeSettings {
    /// Listed among the providers; one not added is never run.
    pub added: bool,
    pub enabled: bool,
    pub binary_path: Option<String>,
    pub subagent_effort: SubagentEffort,
}

impl Default for ClaudeCodeSettings {
    fn default() -> Self {
        Self {
            added: true,
            enabled: true,
            binary_path: None,
            subagent_effort: SubagentEffort::default(),
        }
    }
}

/// The model and effort of the subagents and delegated tasks an agent starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubagentEffort {
    /// The thread's own, unless the user asks for others.
    Thread,
    /// Cheaper ones for simple work, never above the thread's own.
    #[default]
    AgentDecides,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_saved_before_codex_add_it_enabled() {
        let saved = json!({
            "providers": {
                "claudeCode": { "added": true, "enabled": true, "binaryPath": null },
                "copilot": { "added": false, "enabled": false },
            },
        });

        let settings: AppSettings = serde_json::from_value(saved).unwrap();

        assert_eq!(settings.providers.codex, CodexSettings::default());
        assert!(settings.providers.codex.added && settings.providers.codex.enabled);
        assert!(!settings.providers.copilot.added);
    }

    #[test]
    fn copilot_waits_to_be_added() {
        let settings = AppSettings::default().providers;

        assert!(settings.claude_code.added && settings.codex.added);
        assert!(!settings.copilot.added && !settings.copilot.enabled);

        let saved: AppSettings = serde_json::from_value(json!({
            "providers": { "copilot": { "added": true, "enabled": true } },
        }))
        .unwrap();
        assert!(saved.providers.copilot.added && saved.providers.copilot.enabled);
    }

    #[test]
    fn settings_saved_before_copilot_keep_running_it() {
        let saved: AppSettings = serde_json::from_value(json!({
            "providers": { "claudeCode": { "enabled": true, "binaryPath": null } },
        }))
        .unwrap();

        assert!(saved.providers.copilot.added && saved.providers.copilot.enabled);
    }

    #[test]
    fn new_thread_defaults_keep_the_last_used_until_set() {
        let settings: AppSettings = serde_json::from_value(json!({
            "general": { "projectsFolder": "/code" },
        }))
        .unwrap();
        assert_eq!(settings.new_threads, NewThreadSettings::default());
        assert_eq!(settings.general.projects_folder.as_deref(), Some("/code"));
        assert!(!settings.general.clone_into_host_folders);
        assert!(settings.general.continue_after_restart);
        assert!(!settings.general.resume_after_usage_limit);
        assert!(!settings.general.snooze_until_limit_resets);
        assert_eq!(settings.general.snooze_presets, default_snooze_presets());

        let settings = AppSettings {
            new_threads: NewThreadSettings {
                model: Some(NewThreadModel {
                    provider: "claudeCode".into(),
                    model: "opus".into(),
                    effort: Some(Effort::High),
                    fast_mode: false,
                    ultracode: false,
                }),
                access_mode: Some(AccessMode::FullAccess),
                workspace: Some(NewThreadWorkspace::NewWorktree),
            },
            ..AppSettings::default()
        };
        let text = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            text["newThreads"],
            json!({
                "model": {
                    "provider": "claudeCode",
                    "model": "opus",
                    "effort": "high",
                    "fastMode": false,
                    "ultracode": false,
                },
                "accessMode": "fullAccess",
                "workspace": "newWorktree",
            })
        );
        assert_eq!(
            serde_json::from_value::<AppSettings>(text).unwrap(),
            settings
        );
    }

    #[test]
    fn snooze_presets_default_until_saved_and_an_empty_list_stays_empty() {
        let text = serde_json::to_value(AppSettings::default()).unwrap();
        assert_eq!(
            text["general"]["snoozePresets"],
            json!([
                { "kind": "after", "minutes": 60 },
                { "kind": "after", "minutes": 180 },
                { "kind": "at", "day": "today", "time": "18:00" },
                { "kind": "at", "day": "tomorrow", "time": "09:00" },
                { "kind": "at", "day": "monday", "time": "09:00" },
            ])
        );
        let saved: AppSettings = serde_json::from_value(json!({
            "general": { "snoozePresets": [], "snoozeUntilLimitResets": true },
        }))
        .unwrap();
        assert!(saved.general.snooze_presets.is_empty());
        assert!(saved.general.snooze_until_limit_resets);
    }

    #[test]
    fn codex_settings_round_trip() {
        let mut settings = AppSettings::default();
        settings.providers.codex = CodexSettings {
            added: true,
            enabled: false,
            binary_path: Some("/opt/codex/bin/codex".into()),
            home: Some("/home/me/.codex-work".into()),
        };

        let text = serde_json::to_value(&settings).unwrap();

        assert_eq!(
            text["providers"]["codex"],
            json!({
                "added": true,
                "enabled": false,
                "binaryPath": "/opt/codex/bin/codex",
                "home": "/home/me/.codex-work",
            })
        );
        assert_eq!(
            serde_json::from_value::<AppSettings>(text).unwrap(),
            settings
        );
    }
}
