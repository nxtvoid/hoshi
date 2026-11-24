use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use hoshi_core::AppSettings;

pub fn load(path: &Path) -> AppSettings {
    let settings = match fs::read_to_string(path) {
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
    };
    with_written(|written| {
        written
            .entry(path.to_path_buf())
            .or_insert_with(|| Written {
                order: 0,
                settings: Some(settings.clone()),
            });
    });
    settings
}

/// Settings as they were when a change was made, ordered against every other snapshot by when
/// it was taken, so a save that runs late never replaces a newer one.
pub struct Snapshot {
    pub settings: AppSettings,
    order: u64,
}

impl Snapshot {
    pub fn of(settings: &AppSettings) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            settings: settings.clone(),
            order: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }
}

/// A settings file as this run knows it.
struct Written {
    /// The newest snapshot a save was tried with, written or not.
    order: u64,
    /// What the file holds, loaded or last written; `None` when not known.
    settings: Option<AppSettings>,
}

/// Every settings file this run loaded or saved. One lock for all of them: there is one file.
static WRITTEN: Mutex<Option<HashMap<PathBuf, Written>>> = Mutex::new(None);

fn with_written<T>(run: impl FnOnce(&mut HashMap<PathBuf, Written>) -> T) -> T {
    let mut written = WRITTEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    run(written.get_or_insert_with(HashMap::new))
}

/// Writes `settings` now; the snapshot is taken by this call, on the caller's thread.
pub fn save(path: &Path, settings: &AppSettings) -> io::Result<()> {
    save_latest(path, &Snapshot::of(settings), |_| ()).map(|_| ())
}

/// Writes `next` unless a save with a newer snapshot was tried already, then runs `then` with
/// what the file held before, when known. Whether it wrote: a skipped save is `Ok(false)`.
/// Saves run one at a time, `then` included: it must not load or save settings itself.
pub fn save_latest(
    path: &Path,
    next: &Snapshot,
    then: impl FnOnce(Option<&AppSettings>),
) -> io::Result<bool> {
    with_written(|written| {
        if written
            .get(path)
            .is_some_and(|known| known.order > next.order)
        {
            return Ok(false);
        }
        let saved = write(path, &next.settings);
        if let Err(error) = saved {
            written
                .entry(path.to_path_buf())
                .or_insert(Written {
                    order: 0,
                    settings: None,
                })
                .order = next.order;
            return Err(error);
        }
        let before = written.insert(
            path.to_path_buf(),
            Written {
                order: next.order,
                settings: Some(next.settings.clone()),
            },
        );
        then(before.as_ref().and_then(|before| before.settings.as_ref()));
        Ok(true)
    })
}

fn write(path: &Path, settings: &AppSettings) -> io::Result<()> {
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

    #[test]
    fn a_save_that_runs_late_keeps_the_newer_settings_on_disk() {
        let path = settings_file("late-save");
        let older = Snapshot::of(&AppSettings::default());
        let newer = Snapshot::of(&AppSettings {
            usage_limits_in_details: true,
            ..AppSettings::default()
        });
        let mut before = None;
        assert!(save_latest(&path, &newer, |written| before = Some(written.cloned())).unwrap());
        assert_eq!(before, Some(None));
        let mut ran = false;
        assert!(!save_latest(&path, &older, |_| ran = true).unwrap());
        assert!(!ran, "the older save is skipped");
        assert!(load(&path).usage_limits_in_details);

        let latest = Snapshot::of(&AppSettings::default());
        let mut before = None;
        save_latest(&path, &latest, |written| before = written.cloned()).unwrap();
        assert!(before.is_some_and(|before| before.usage_limits_in_details));
        assert!(!load(&path).usage_limits_in_details);
    }

    #[test]
    fn a_loaded_file_is_what_the_first_save_tells_its_changes_against() {
        let path = settings_file("loaded-baseline");
        save(
            &path,
            &AppSettings {
                plan_mode_enabled: true,
                ..AppSettings::default()
            },
        )
        .unwrap();
        assert!(load(&path).plan_mode_enabled);
        let mut before = None;
        save_latest(&path, &Snapshot::of(&AppSettings::default()), |written| {
            before = written.cloned()
        })
        .unwrap();
        assert!(before.is_some_and(|before| before.plan_mode_enabled));
    }

    #[test]
    fn a_failed_save_still_turns_away_older_snapshots() {
        let dir =
            std::env::temp_dir().join(format!("hoshi-settings-failed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("settings.json.tmp")).unwrap();
        let path = dir.join("settings.json");
        let older = Snapshot::of(&AppSettings::default());
        save(&path, &AppSettings::default()).unwrap_err();
        assert!(!save_latest(&path, &older, |_| ()).unwrap_or(true));
    }

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
            usage_limits_in_details: true,
            context_meter: hoshi_core::ContextMeterPlace::Composer,
            onboarding_completed_at: Some("2026-11-09T08:00:00.000Z".into()),
        };

        save(&path, &settings).unwrap();
        save(&path, &settings).unwrap();

        assert_eq!(load(&path), settings);
        assert!(!path.with_extension("json.tmp").exists());
    }
}
