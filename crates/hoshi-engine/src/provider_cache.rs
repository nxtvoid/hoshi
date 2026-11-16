//! Each provider's models as its CLI last listed them, one file per provider, so the next
//! start shows them before the CLIs answer.

use hoshi_core::ProviderModels;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Where the files go under hoshi's home.
pub(crate) fn dir(home: &Path) -> PathBuf {
    home.join("caches").join("providers")
}

fn path(dir: &Path, provider: &str) -> PathBuf {
    dir.join(format!("{provider}.json"))
}

/// Replaces the file whole, so a reader never finds half of it.
pub(crate) fn write(dir: &Path, catalog: &ProviderModels) -> io::Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(catalog).map_err(io::Error::other)?;
    let target = path(dir, &catalog.provider);
    let temp = dir.join(format!(
        "{}.{}-{}.tmp",
        catalog.provider,
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temp, json)?;
    fs::rename(&temp, &target).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

/// `None` when nothing was kept, or what was kept can't be read or is another provider's.
pub(crate) fn read(dir: &Path, provider: &str) -> Option<ProviderModels> {
    let path = path(dir, provider);
    let text = fs::read_to_string(&path).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    match serde_json::from_str::<ProviderModels>(&text) {
        Ok(catalog) if catalog.provider == provider => Some(catalog),
        Ok(catalog) => {
            eprintln!(
                "hoshi: ignoring {}, which holds the models of {}",
                path.display(),
                catalog.provider
            );
            None
        }
        Err(error) => {
            eprintln!("hoshi: ignoring the unreadable {}: {error}", path.display());
            None
        }
    }
}

/// What a provider offers before its CLI answers: its offline catalog, then the kept models
/// that catalog lacks.
pub(crate) fn hydrate(
    offline: Option<ProviderModels>,
    kept: Option<ProviderModels>,
) -> Option<ProviderModels> {
    let Some(mut catalog) = offline else {
        return kept;
    };
    if let Some(kept) = kept.filter(|kept| kept.provider == catalog.provider) {
        let missing: Vec<_> = kept
            .models
            .into_iter()
            .filter(|model| !catalog.models.iter().any(|known| known.slug == model.slug))
            .collect();
        catalog.models.extend(missing);
        catalog.default_model = catalog.default_model.or(kept.default_model);
    }
    Some(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::ModelInfo;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hoshi-provider-cache-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn model(slug: &str, name: &str) -> ModelInfo {
        ModelInfo {
            slug: slug.into(),
            name: name.into(),
            short_name: name.into(),
            legacy: false,
            is_new: false,
            update_required: false,
            efforts: vec![hoshi_core::Effort::High],
            default_effort: Some(hoshi_core::Effort::High),
            fast_mode: true,
            ultracode: false,
        }
    }

    fn catalog(
        provider: &str,
        models: Vec<ModelInfo>,
        default_model: Option<&str>,
    ) -> ProviderModels {
        ProviderModels {
            provider: provider.into(),
            default_model: default_model.map(str::to_owned),
            models,
            compacts_idle_threads: true,
        }
    }

    #[test]
    fn a_kept_catalog_reads_back_as_written() {
        let dir = scratch("round-trip");
        let written = catalog("codex", vec![model("gpt-6", "GPT-6")], Some("gpt-6"));

        write(&dir, &written).unwrap();
        write(&dir, &written).unwrap();

        assert_eq!(read(&dir, "codex"), Some(written));
        let left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["codex.json"]);
    }

    #[test]
    fn nothing_kept_reads_as_none() {
        assert_eq!(read(&scratch("missing"), "codex"), None);
    }

    #[test]
    fn a_corrupt_or_empty_file_is_ignored() {
        let dir = scratch("corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            path(&dir, "codex"),
            "{\"provider\": \"codex\", \"models\": [",
        )
        .unwrap();
        assert_eq!(read(&dir, "codex"), None);

        fs::write(path(&dir, "codex"), "  \n").unwrap();
        assert_eq!(read(&dir, "codex"), None);
    }

    #[test]
    fn a_file_holding_another_provider_s_models_is_ignored() {
        let dir = scratch("identity");
        fs::create_dir_all(&dir).unwrap();
        let other = catalog("copilot", vec![model("gpt-6", "GPT-6")], None);
        fs::write(path(&dir, "codex"), serde_json::to_string(&other).unwrap()).unwrap();

        assert_eq!(read(&dir, "codex"), None);
    }

    #[test]
    fn the_offline_catalog_comes_first_and_the_kept_one_adds_what_it_lacks() {
        let offline = catalog(
            "claudeCode",
            vec![model("opus", "Opus offline"), model("sonnet", "Sonnet")],
            Some("opus"),
        );
        let kept = catalog(
            "claudeCode",
            vec![model("haiku", "Haiku"), model("opus", "Opus kept")],
            Some("haiku"),
        );

        let hydrated = hydrate(Some(offline), Some(kept)).unwrap();

        let names: Vec<_> = hydrated
            .models
            .iter()
            .map(|model| model.name.as_str())
            .collect();
        assert_eq!(names, ["Opus offline", "Sonnet", "Haiku"]);
        assert_eq!(hydrated.default_model.as_deref(), Some("opus"));
    }

    #[test]
    fn either_catalog_alone_is_enough() {
        let offline = catalog("claudeCode", vec![model("opus", "Opus")], None);
        let kept = catalog("codex", vec![model("gpt-6", "GPT-6")], Some("gpt-6"));

        assert_eq!(hydrate(Some(offline.clone()), None), Some(offline));
        assert_eq!(hydrate(None, Some(kept.clone())), Some(kept));
        assert_eq!(hydrate(None, None), None);
    }

    #[test]
    fn a_kept_catalog_of_another_provider_adds_nothing() {
        let offline = catalog("claudeCode", vec![model("opus", "Opus")], None);
        let kept = catalog("codex", vec![model("gpt-6", "GPT-6")], Some("gpt-6"));

        assert_eq!(hydrate(Some(offline.clone()), Some(kept)), Some(offline));
    }
}
