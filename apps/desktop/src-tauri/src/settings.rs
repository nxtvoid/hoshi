use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use hoshi_core::AppSettings;
use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Result<AppSettings, String> {
    Ok(load(&settings_path(&app)?))
}

#[tauri::command]
pub fn update_settings(app: AppHandle, settings: AppSettings) -> Result<AppSettings, String> {
    let path = settings_path(&app)?;
    let previous = load(&path);
    save(&path, &settings).map_err(|error| error.to_string())?;
    let engine = crate::orchestration::engine(&app);
    if previous.providers != settings.providers {
        engine.set_adapter(crate::orchestration::adapter_for(&settings));
    }
    engine.set_thread_titles(settings.thread_titles.clone());
    engine.set_vault(settings.vault);
    Ok(settings)
}

pub fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.state::<crate::home::Home>().settings())
}

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
        ClaudeCodeSettings, FavoriteModel, GeneralSettings, ProviderSettings, StartupView,
        ThreadTitleSettings, VaultSettings,
    };

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
                    enabled: false,
                    binary_path: Some("C:\\tools\\claude.exe".into()),
                },
            },
            plan_mode_enabled: true,
            favorite_models: vec![FavoriteModel {
                provider: "claudeCode".into(),
                model: "claude-opus-5-5".into(),
            }],
            thread_titles: ThreadTitleSettings {
                generate: false,
                model: Some("haiku".into()),
                instructions: "Use Spanish.".into(),
            },
            general: GeneralSettings {
                startup_view: StartupView::LastProject,
                inactive_after_days: 7,
                notifications: false,
                projects_folder: Some("D:\\code".into()),
            },
            vault: VaultSettings { always_allow: true },
            onboarding_completed_at: Some("2026-11-09T08:00:00.000Z".into()),
        };

        save(&path, &settings).unwrap();
        save(&path, &settings).unwrap();

        assert_eq!(load(&path), settings);
        assert!(!path.with_extension("json.tmp").exists());
    }
}
