use std::fs;
use std::io;
use std::path::Path;

use hoshi_core::AppSettings;

pub fn load(path: &Path) -> AppSettings {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
            eprintln!(
                "using default settings, {} is invalid: {error}",
                path.display()
            );
            AppSettings::default()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let settings = AppSettings::default();
            if let Err(error) = save(path, &settings) {
                eprintln!("could not create {}: {error}", path.display());
            }
            settings
        }
        Err(error) => {
            eprintln!(
                "using default settings, could not read {}: {error}",
                path.display()
            );
            AppSettings::default()
        }
    }
}

pub fn save(path: &Path, settings: &AppSettings) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, json)?;
    fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::{
        AccessMode, BrowserSettings, ClaudeCodeSettings, CodexSettings, CopilotSettings, Effort,
        FavoriteModel, GeneralSettings, IdleCompaction, LinkTarget, NewThreadModel,
        NewThreadSettings, NewThreadWorkspace, PageColorScheme, PageSize, ProviderSettings,
        SearchEngine, SnoozeDay, SnoozePreset, StartupView, SubagentEffort, ThreadTitleSettings,
    };
    use std::path::PathBuf;

    fn settings_file(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hoshi-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("settings.json")
    }

    #[test]
    fn missing_file_yields_defaults_and_creates_it() {
        let path = settings_file("missing");

        let settings = load(&path);

        assert_eq!(settings, AppSettings::default());
        assert!(settings.providers.claude_code.enabled);
        assert_eq!(settings.providers.claude_code.binary_path, None);
        assert_eq!(
            settings.providers.claude_code.subagent_effort,
            SubagentEffort::AgentDecides
        );
        assert!(path.exists());
    }

    #[test]
    fn partial_json_fills_missing_fields_with_defaults() {
        let path = settings_file("partial");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"providers":{"claudeCode":{"binaryPath":"/bin/claude"}}}"#,
        )
        .unwrap();

        let settings = load(&path);

        assert!(settings.providers.claude_code.enabled);
        assert_eq!(
            settings.providers.claude_code.binary_path.as_deref(),
            Some("/bin/claude")
        );
        assert_eq!(
            settings.providers.claude_code.subagent_effort,
            SubagentEffort::AgentDecides
        );

        fs::write(&path, "{}").unwrap();
        assert_eq!(load(&path), AppSettings::default());
    }

    #[test]
    fn corrupt_json_yields_defaults() {
        let path = settings_file("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ not json").unwrap();

        assert_eq!(load(&path), AppSettings::default());
    }

    #[test]
    fn saved_settings_round_trip() {
        let path = settings_file("round-trip");
        let settings = AppSettings {
            providers: ProviderSettings {
                claude_code: ClaudeCodeSettings {
                    added: true,
                    enabled: false,
                    binary_path: Some("C:\\tools\\claude.exe".into()),
                    subagent_effort: SubagentEffort::Thread,
                },
                copilot: CopilotSettings {
                    added: false,
                    enabled: false,
                    binary_path: Some("C:\\tools\\copilot.exe".into()),
                },
                codex: CodexSettings {
                    added: true,
                    enabled: false,
                    binary_path: Some("C:\\tools\\codex.exe".into()),
                    home: Some("D:\\codex-home".into()),
                },
                health_check_interval_secs: 60,
                idle_compaction: IdleCompaction::BeforeCacheExpires,
            },
            plan_mode_enabled: true,
            favorite_models: vec![FavoriteModel {
                provider: "claudeCode".into(),
                model: "claude-opus-5-5".into(),
            }],
            thread_titles: ThreadTitleSettings {
                generate: false,
                provider: Some("copilot".into()),
                model: Some("haiku".into()),
                instructions: "Use Spanish.".into(),
            },
            new_threads: NewThreadSettings {
                model: Some(NewThreadModel {
                    provider: "codex".into(),
                    model: "gpt-5.5".into(),
                    effort: Some(Effort::Xhigh),
                    fast_mode: true,
                    ultracode: false,
                }),
                access_mode: Some(AccessMode::Auto),
                workspace: Some(NewThreadWorkspace::Local),
            },
            general: GeneralSettings {
                startup_view: StartupView::LastProject,
                inactive_after_days: 7,
                notifications: false,
                projects_folder: Some("D:\\code".into()),
                clone_into_host_folders: true,
                continue_after_restart: false,
                resume_after_usage_limit: true,
                snooze_until_limit_resets: true,
                snooze_presets: vec![
                    SnoozePreset::After { minutes: 30 },
                    SnoozePreset::At {
                        day: SnoozeDay::Friday,
                        time: "16:30".into(),
                    },
                ],
                delete_merged_worktrees: true,
            },
            browser: BrowserSettings {
                agent_access: false,
                float_agent_pages: false,
                open_links_in: LinkTarget::Hoshi,
                search_engine: SearchEngine::Custom,
                custom_search_url: "https://search.example/?q=%s".into(),
                page_size: PageSize::Phone,
                zoom: 125,
                color_scheme: PageColorScheme::Dark,
            },
            onboarding_completed_at: Some("2026-11-09T08:00:00.000Z".into()),
        };

        save(&path, &settings).unwrap();
        save(&path, &settings).unwrap();

        assert_eq!(load(&path), settings);
        assert!(!path.with_extension("json.tmp").exists());
    }
}
