//! The window paints the theme's background before the webview has, so starting, restoring
//! or resizing it never flashes white. The UI resolves the theme and mirrors it here.

use crate::home::Home;
use std::path::Path;
use std::time::Duration;
use tauri::window::Color;
use tauri::{AppHandle, Manager, State, Theme, WebviewWindow};

const APPEARANCE_FILE: &str = "appearance";
const DARK: &str = "dark";
const LIGHT: &str = "light";
/// `--background` in `packages/ui/src/styles/globals.css`.
const LIGHT_BACKGROUND: Color = Color(255, 255, 255, 255);
const DARK_BACKGROUND: Color = Color(10, 10, 10, 255);
/// The page shows the window once it painted; this shows it anyway if the page can't.
const SHOW_AT_THE_LATEST: Duration = Duration::from_secs(2);

/// `None` until the UI saved it once.
pub fn saved(home: &Path) -> Option<bool> {
    match std::fs::read_to_string(home.join(APPEARANCE_FILE))
        .ok()?
        .trim()
    {
        DARK => Some(true),
        LIGHT => Some(false),
        _ => None,
    }
}

pub fn background(dark: bool) -> Color {
    if dark {
        DARK_BACKGROUND
    } else {
        LIGHT_BACKGROUND
    }
}

/// Without a saved appearance the window follows the system's.
pub fn follow_system(window: &WebviewWindow) {
    if window.theme().is_ok_and(|theme| theme == Theme::Dark)
        && let Err(error) = window.set_background_color(Some(DARK_BACKGROUND))
    {
        eprintln!("hoshi: failed to paint the window dark: {error}");
    }
}

pub fn show_at_the_latest(window: WebviewWindow) {
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_AT_THE_LATEST);
        if !window.is_visible().unwrap_or(true) {
            let _ = window.show();
        }
    });
}

#[tauri::command]
pub fn set_appearance(app: AppHandle, home: State<'_, Home>, dark: bool) {
    for window in app.webview_windows().values() {
        if let Err(error) = window.set_background_color(Some(background(dark))) {
            eprintln!("hoshi: failed to repaint the window: {error}");
        }
    }
    if saved(&home.0) == Some(dark) {
        return;
    }
    let value = if dark { DARK } else { LIGHT };
    if let Err(error) = std::fs::write(home.0.join(APPEARANCE_FILE), value) {
        eprintln!("hoshi: failed to save the appearance: {error}");
    }
}
