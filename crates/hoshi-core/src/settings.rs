use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct AppSettings {
    pub providers: ProviderSettings,
    /// Shows the composer's Plan toggle.
    pub plan_mode_enabled: bool,
    /// Listed first in the model picker.
    pub favorite_models: Vec<FavoriteModel>,
    pub thread_titles: ThreadTitleSettings,
    pub general: GeneralSettings,
    pub browser: BrowserSettings,
    /// When the first-run setup was finished or skipped; `None` until then.
    pub onboarding_completed_at: Option<String>,
}

/// What the app shows when it opens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StartupView {
    #[default]
    Inbox,
    /// The project and thread open when the app last closed.
    LastProject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct GeneralSettings {
    pub startup_view: StartupView,
    /// Idle threads untouched for this many days move to Inactive.
    pub inactive_after_days: u32,
    /// A system notification when a thread finishes, fails or asks for input while the
    /// window is in the background.
    pub notifications: bool,
    /// Where new projects and clones go; None is the home folder.
    pub projects_folder: Option<String>,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            startup_view: StartupView::Inbox,
            inactive_after_days: 3,
            notifications: true,
            projects_folder: None,
        }
    }
}

/// hoshi's headless browser: who uses it and how its pages open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LinkTarget {
    /// The user's default browser.
    #[default]
    System,
    /// The thread's page in hoshi's browser.
    Hoshi,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PageSize {
    /// The side panel's size, while the panel shows the page.
    Fill,
    #[default]
    Desktop,
    Tablet,
    Phone,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PageColorScheme {
    /// The browser's own, which follows the system.
    #[default]
    System,
    Light,
    Dark,
}

/// How a new thread gets its title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadTitleSettings {
    /// A small model reads the first message and names the thread; otherwise its
    /// first line does.
    pub generate: bool,
    /// `None` uses the provider's quick model.
    pub model: Option<String>,
    /// Added to the provider's own instructions, as the user wrote them.
    pub instructions: String,
}

impl Default for ThreadTitleSettings {
    fn default() -> Self {
        Self {
            generate: true,
            model: None,
            instructions: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FavoriteModel {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderSettings {
    pub claude_code: ClaudeCodeSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ClaudeCodeSettings {
    pub enabled: bool,
    pub binary_path: Option<String>,
    pub subagent_effort: SubagentEffort,
}

impl Default for ClaudeCodeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            binary_path: None,
            subagent_effort: SubagentEffort::default(),
        }
    }
}

/// The model and effort of the subagents and delegated tasks an agent starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SubagentEffort {
    /// The thread's own, unless the user asks for others.
    Thread,
    /// Cheaper ones for simple work, never above the thread's own.
    #[default]
    AgentDecides,
}
