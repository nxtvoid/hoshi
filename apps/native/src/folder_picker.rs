//! The system's folder picker, opened at a given folder when there is one. GPUI's own prompt can't
//! start anywhere, so that case goes through `rfd`.

use futures::FutureExt as _;
use futures::future::LocalBoxFuture;
use gpui_kit::{App, PathPromptOptions, SharedString, Window};
use std::path::PathBuf;

/// The projects folder from settings, when it is set to something.
pub fn configured(folder: Option<&str>) -> Option<String> {
    folder
        .map(str::trim)
        .filter(|folder| !folder.is_empty())
        .map(str::to_owned)
}

/// The first of the candidates that is a folder on disk.
pub fn start_folder(candidates: impl IntoIterator<Item = Option<String>>) -> Option<PathBuf> {
    candidates
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .find(|folder| folder.is_dir())
}

pub fn pick_folder(
    start: Option<PathBuf>,
    prompt: Option<SharedString>,
    window: &Window,
    cx: &mut App,
) -> LocalBoxFuture<'static, Option<PathBuf>> {
    if let Some(start) = start {
        return rfd::AsyncFileDialog::new()
            .set_directory(start)
            .set_parent(window)
            .pick_folder()
            .map(|picked| picked.map(|handle| handle.path().to_path_buf()))
            .boxed_local();
    }
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt,
    });
    async move { picked.await.ok()?.ok()??.into_iter().next() }.boxed_local()
}

#[cfg(test)]
mod tests {
    use super::{configured, start_folder};

    #[test]
    fn a_blank_projects_folder_is_not_set() {
        assert_eq!(configured(None), None);
        assert_eq!(configured(Some("   ")), None);
        assert_eq!(configured(Some(" /code ")), Some("/code".to_owned()));
    }

    #[test]
    fn starts_at_the_first_existing_folder() {
        let existing = std::env::temp_dir();
        let missing = existing.join("hoshi-folder-picker-missing");
        let existing = existing.to_string_lossy().into_owned();
        let missing = missing.to_string_lossy().into_owned();
        assert_eq!(start_folder([None, None]), None);
        assert_eq!(start_folder([Some(missing.clone())]), None);
        assert_eq!(
            start_folder([None, Some(missing), Some(existing.clone())]),
            Some(existing.into())
        );
    }
}
