use crate::m;
use hoshi_core::Editor;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What the card remembers across restarts. The web keeps it in the webview's storage; here it
/// lives beside the settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    /// The editor "Open in" uses; the first installed one until the user picks.
    pub preferred_editor: Option<Editor>,
    /// Pull requests open as drafts.
    pub draft_pull_requests: bool,
    /// The threads whose card the user closed; it shows on the others.
    pub closed_threads: BTreeSet<String>,
}

pub fn file(home: &Path) -> PathBuf {
    home.join("details.json")
}

pub fn parse(text: &str) -> Prefs {
    let value: Value = serde_json::from_str(text).unwrap_or_default();
    Prefs {
        preferred_editor: value
            .get("preferredEditor")
            .and_then(|editor| serde_json::from_value(editor.clone()).ok()),
        draft_pull_requests: value
            .get("draftPullRequests")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        closed_threads: value
            .get("closedThreads")
            .and_then(|ids| serde_json::from_value(ids.clone()).ok())
            .unwrap_or_default(),
    }
}

pub fn serialize(prefs: &Prefs) -> String {
    json!({
        "preferredEditor": prefs.preferred_editor,
        "draftPullRequests": prefs.draft_pull_requests,
        "closedThreads": prefs.closed_threads,
    })
    .to_string()
}

pub fn load(home: &Path) -> Prefs {
    std::fs::read_to_string(file(home))
        .map(|text| parse(&text))
        .unwrap_or_default()
}

pub fn save(home: &Path, prefs: &Prefs) -> std::io::Result<()> {
    let file = file(home);
    let partial = file.with_extension("json.tmp");
    std::fs::write(&partial, serialize(prefs))?;
    std::fs::rename(partial, file)
}

/// The target "Open in" uses: the one the user picked, else the first found.
pub fn open_target(editors: &[Editor], preferred: Option<Editor>) -> Option<Editor> {
    preferred
        .filter(|editor| editors.contains(editor))
        .or_else(|| editors.first().copied())
}

pub fn editor_label(editor: Editor) -> String {
    match editor {
        Editor::Vscode => m::editor_vscode(),
        Editor::Cursor => m::editor_cursor(),
        Editor::Windsurf => m::editor_windsurf(),
        Editor::Zed => m::editor_zed(),
        Editor::Idea => m::editor_idea(),
        Editor::Webstorm => m::editor_webstorm(),
        Editor::Pycharm => m::editor_pycharm(),
        Editor::Goland => m::editor_goland(),
        Editor::Rider => m::editor_rider(),
        Editor::Clion => m::editor_clion(),
        Editor::Phpstorm => m::editor_phpstorm(),
        Editor::Rubymine => m::editor_rubymine(),
        Editor::Rustrover => m::editor_rustrover(),
        Editor::FileManager if cfg!(target_os = "macos") => m::finder(),
        Editor::FileManager if cfg!(target_os = "windows") => m::file_explorer(),
        Editor::FileManager => m::files_app(),
    }
}

/// How an editor's mark is drawn, by its name in hoshi-ui's `brand/editors`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorMark {
    /// In the brand's own colors.
    Colored(&'static str),
    /// One color, drawn in the text color.
    Tinted(&'static str),
    /// No mark of its own: the code icon.
    Generic,
}

pub fn editor_mark(editor: Editor) -> EditorMark {
    match editor {
        Editor::Vscode => EditorMark::Colored("vscode"),
        Editor::Cursor => EditorMark::Tinted("cursor"),
        Editor::Zed => EditorMark::Tinted("zed"),
        Editor::Windsurf => EditorMark::Generic,
        Editor::Idea => EditorMark::Colored("idea"),
        Editor::Webstorm => EditorMark::Colored("webstorm"),
        Editor::Pycharm => EditorMark::Colored("pycharm"),
        Editor::Goland => EditorMark::Colored("goland"),
        Editor::Rider => EditorMark::Colored("rider"),
        Editor::Clion => EditorMark::Colored("clion"),
        Editor::Phpstorm => EditorMark::Colored("phpstorm"),
        Editor::Rubymine => EditorMark::Colored("rubymine"),
        Editor::Rustrover => EditorMark::Colored("rustrover"),
        Editor::FileManager if cfg!(target_os = "macos") => EditorMark::Colored("finder"),
        Editor::FileManager => EditorMark::Colored("file-explorer"),
    }
}

/// The open-in shortcut as the menu shows it.
pub fn open_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘O"
    } else {
        "Ctrl+O"
    }
}

#[cfg(test)]
mod tests {
    use super::{Prefs, open_target, parse, serialize};
    use hoshi_core::Editor;

    #[test]
    fn open_in_uses_the_picked_editor_while_it_is_installed() {
        let found = [Editor::Vscode, Editor::Zed, Editor::FileManager];
        assert_eq!(open_target(&found, Some(Editor::Zed)), Some(Editor::Zed));
        assert_eq!(
            open_target(&found, Some(Editor::Cursor)),
            Some(Editor::Vscode)
        );
        assert_eq!(open_target(&found, None), Some(Editor::Vscode));
        assert_eq!(open_target(&[], Some(Editor::Zed)), None);
    }

    #[test]
    fn the_prefs_survive_a_round_trip_and_bad_files_read_as_defaults() {
        let prefs = Prefs {
            preferred_editor: Some(Editor::FileManager),
            draft_pull_requests: true,
            closed_threads: ["t".to_owned()].into(),
        };
        assert_eq!(parse(&serialize(&prefs)), prefs);
        assert!(serialize(&prefs).contains(r#""preferredEditor":"fileManager""#));
        assert_eq!(parse("not json"), Prefs::default());
        assert_eq!(parse(r#"{"preferredEditor":"nano"}"#), Prefs::default());
    }
}
