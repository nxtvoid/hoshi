pub mod app_release;
pub mod app_update;
pub mod fonts;
pub mod home;
pub mod settings;
pub mod theme_import;

use hoshi_core::AppSettings;
use hoshi_engine::{Engine, Notify};
use hoshi_providers::{
    ClaudeCodeAdapter, CodexAdapter, CodexInstall, CodexSetup, CopilotAdapter, ManagedCodex,
    ProviderAdapter,
};
use hoshi_store::StoreError;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Opens the engine on `home` and configures it from the saved settings.
pub fn open_engine(home: &Path, notify: Notify) -> Result<Engine, StoreError> {
    let engine = Engine::open(home, notify)?;
    let settings = settings::load(&home::settings_path(home));
    engine.set_adapters(adapters_for(&settings, home));
    configure(&engine, &settings, home);
    engine.set_forges(hoshi_engine::default_forges());
    engine.continue_cut_threads();
    Ok(engine)
}

/// Saves `settings` to `path`, the settings file in hoshi's home, and hands the engine what
/// changed from `previous`.
pub fn apply_settings(
    engine: &Engine,
    path: &Path,
    previous: &AppSettings,
    settings: &AppSettings,
) -> std::io::Result<()> {
    settings::save(path, settings)?;
    let home = path.parent().unwrap_or(Path::new(""));
    let (before, after) = (&previous.providers, &settings.providers);
    if before.claude_code != after.claude_code
        || before.copilot != after.copilot
        || before.codex != after.codex
    {
        engine.set_adapters(adapters_for(settings, home));
    }
    configure(engine, settings, home);
    Ok(())
}

fn configure(engine: &Engine, settings: &AppSettings, home: &Path) {
    engine.set_thread_titles(settings.thread_titles.clone());
    engine.set_browser(settings.browser.clone());
    engine.set_subagent_effort(settings.providers.claude_code.subagent_effort);
    engine.set_idle_compaction(settings.providers.idle_compaction);
    engine.set_continue_after_restart(settings.general.continue_after_restart);
    engine.set_resume_after_usage_limit(settings.general.resume_after_usage_limit);
    engine.set_snooze_until_limit_resets(settings.general.snooze_until_limit_resets);
    engine.set_delete_merged_worktrees(settings.general.delete_merged_worktrees);
    engine.set_codex(codex_setup(settings, home));
}

/// Where Codex is found and keeps its sign-in, while the settings add and enable it. hoshi's
/// own installs of it live in `home`.
pub fn codex_setup(settings: &AppSettings, home: &Path) -> Option<CodexSetup> {
    let codex = &settings.providers.codex;
    (codex.added && codex.enabled).then(|| CodexSetup {
        binary_path: codex
            .binary_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_owned),
        home: codex.home.as_deref().and_then(expand_home),
        install: CodexInstall::new(home),
    })
}

/// Codex refuses a `CODEX_HOME` that starts with `~`, which no shell expanded.
fn expand_home(path: &str) -> Option<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let rest = path
        .strip_prefix("~/")
        .or_else(|| path.strip_prefix("~\\"))
        .or_else(|| (path == "~").then_some(""));
    match (rest, std::env::home_dir()) {
        (Some(rest), Some(user_home)) => Some(user_home.join(rest)),
        _ => Some(PathBuf::from(path)),
    }
}

/// The adapters of the providers the settings add and enable; hoshi's home holds the Codex
/// it installs.
pub fn adapters_for(settings: &AppSettings, home: &Path) -> Vec<Arc<dyn ProviderAdapter>> {
    let (claude, copilot) = (&settings.providers.claude_code, &settings.providers.copilot);
    let mut adapters: Vec<Arc<dyn ProviderAdapter>> = Vec::new();
    if claude.added && claude.enabled {
        adapters.push(Arc::new(ClaudeCodeAdapter::new(
            claude.binary_path.clone(),
            None,
        )));
    }
    if copilot.added && copilot.enabled {
        adapters.push(Arc::new(CopilotAdapter::new(copilot.binary_path.clone())));
    }
    if let Some(setup) = codex_setup(settings, home) {
        adapters.push(Arc::new(ManagedCodex::new(
            setup,
            Box::new(|binary, codex_home| Arc::new(CodexAdapter::new(binary, codex_home))),
        )));
    }
    adapters
}

#[cfg(test)]
mod tests {
    use super::{adapters_for, codex_setup, expand_home};
    use hoshi_core::AppSettings;
    use hoshi_providers::CodexInstall;
    use std::path::{Path, PathBuf};

    fn providers(settings: &AppSettings) -> Vec<&'static str> {
        adapters_for(settings, Path::new("/hoshi"))
            .iter()
            .map(|adapter| adapter.provider())
            .collect()
    }

    #[test]
    fn only_added_and_enabled_providers_get_an_adapter() {
        let mut settings = AppSettings::default();
        assert_eq!(providers(&settings), ["claudeCode", "copilot", "codex"]);

        settings.providers.copilot.added = false;
        assert_eq!(providers(&settings), ["claudeCode", "codex"]);

        settings.providers.codex.enabled = false;
        assert_eq!(providers(&settings), ["claudeCode"]);

        settings.providers.claude_code.enabled = false;
        assert!(providers(&settings).is_empty());
    }

    #[test]
    fn codex_runs_from_the_settings_with_hoshis_install_beside_them() {
        let mut settings = AppSettings::default();
        settings.providers.codex.binary_path = Some("  ".into());
        settings.providers.codex.home = Some("/work/.codex".into());

        let setup = codex_setup(&settings, Path::new("/hoshi")).unwrap();

        assert_eq!(setup.binary_path, None);
        assert_eq!(setup.home, Some(PathBuf::from("/work/.codex")));
        assert_eq!(setup.install, CodexInstall::new(Path::new("/hoshi")));
        settings.providers.codex.added = false;
        assert_eq!(codex_setup(&settings, Path::new("/hoshi")), None);
    }

    #[test]
    fn a_codex_home_under_the_user_folder_is_expanded() {
        let user_home = std::env::home_dir().unwrap();

        assert_eq!(
            expand_home("~/.codex-work"),
            Some(user_home.join(".codex-work"))
        );
        assert_eq!(expand_home("~"), Some(user_home));
        assert_eq!(expand_home(" "), None);
        assert_eq!(expand_home("/srv/codex"), Some(PathBuf::from("/srv/codex")));
    }
}
