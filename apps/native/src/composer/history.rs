//! Past prompts, recalled with ↑ in an empty composer and stepped back with ↓: sent from any
//! thread, oldest first, kept across restarts in `prompt-history.json` beside the settings.

use super::Composer;
use gpui_kit::*;
use std::path::{Path, PathBuf};

const LIMIT: usize = 200;

/// Which past prompt the composer shows, as an index into the history.
pub type Position = Option<usize>;

/// Moves through past prompts: back from the newest, forward to an empty composer. `None`
/// when there is nowhere to go.
pub fn step(entries: &[String], position: Position, back: bool) -> Option<(Position, String)> {
    if back {
        let next = match position {
            None => entries.len().checked_sub(1)?,
            Some(position) => position.checked_sub(1)?,
        };
        return Some((Some(next), entries[next].clone()));
    }
    let position = position?;
    Some(match entries.get(position + 1) {
        Some(text) => (Some(position + 1), text.clone()),
        None => (None, String::new()),
    })
}

/// Adds a sent prompt, dropping a repeat of the last one and the oldest past `limit`.
pub fn remember(entries: &mut Vec<String>, prompt: &str, limit: usize) {
    let text = prompt.trim();
    if text.is_empty() || entries.last().is_some_and(|last| last == text) {
        return;
    }
    entries.push(text.to_owned());
    let excess = entries.len().saturating_sub(limit);
    entries.drain(..excess);
}

fn file(home: &Path) -> PathBuf {
    home.join("prompt-history.json")
}

pub fn load(home: &Path) -> Vec<String> {
    std::fs::read_to_string(file(home))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The past prompt the composer shows, while its text is still that prompt.
#[derive(Clone, Debug, Default)]
pub struct Recalled {
    pub position: usize,
    pub text: String,
}

impl Composer {
    fn history_home(&self, cx: &App) -> Option<PathBuf> {
        self.workspace
            .read(cx)
            .settings_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    pub(super) fn load_history(&mut self, cx: &App) {
        if let Some(home) = self.history_home(cx) {
            self.history = load(&home);
        }
    }

    pub(super) fn remember_prompt(&mut self, prompt: &str, cx: &mut Context<Self>) {
        remember(&mut self.history, prompt, LIMIT);
        let Some(home) = self.history_home(cx) else {
            return;
        };
        let entries = self.history.clone();
        cx.background_spawn(async move {
            let text = serde_json::to_string(&entries).map_err(std::io::Error::other)?;
            std::fs::write(file(&home), text)
        })
        .detach_and_log_err(cx);
    }

    /// ↑ on the first line of an empty composer, or of a recalled prompt, shows the one before;
    /// ↓ on the last line of a recalled prompt, the one after. `true` when the key was used.
    pub(super) fn history_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let modifiers = &keystroke.modifiers;
        if modifiers.shift || modifiers.control || modifiers.alt || modifiers.platform {
            return false;
        }
        let back = match keystroke.key.as_str() {
            "up" => true,
            "down" => false,
            _ => return false,
        };
        let showing = self
            .recalled
            .as_ref()
            .filter(|recalled| recalled.text == self.text(cx))
            .map(|recalled| recalled.position);
        let editor = self.editor.read(cx);
        let allowed = if back {
            (editor.is_empty(cx) || showing.is_some()) && editor.on_first_line(cx)
        } else {
            showing.is_some() && editor.on_last_line(cx)
        };
        if !allowed {
            return false;
        }
        let Some((position, text)) = step(&self.history, showing, back) else {
            return false;
        };
        self.recalled = position.map(|position| Recalled {
            position,
            text: text.clone(),
        });
        self.set_text(&text, window, cx);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{remember, step};

    #[test]
    fn steps_back_from_the_newest_and_forward_to_empty() {
        let entries = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(step(&entries, None, true), Some((Some(1), "b".into())));
        assert_eq!(step(&entries, Some(1), true), Some((Some(0), "a".into())));
        assert_eq!(step(&entries, Some(0), true), None);
        assert_eq!(step(&entries, Some(0), false), Some((Some(1), "b".into())));
        assert_eq!(step(&entries, Some(1), false), Some((None, String::new())));
        assert_eq!(step(&entries, None, false), None);
        assert_eq!(step(&[], None, true), None);
    }

    #[test]
    fn repeats_and_the_oldest_past_the_limit_are_dropped() {
        let mut entries = vec!["a".to_owned()];
        remember(&mut entries, " a ", 3);
        remember(&mut entries, "", 3);
        remember(&mut entries, "b", 3);
        remember(&mut entries, "c", 3);
        remember(&mut entries, "d", 3);
        assert_eq!(entries, ["b", "c", "d"]);
    }
}
