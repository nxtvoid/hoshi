//! Workarounds for WebKitGTK, the webview hoshi runs in on Linux.

use tauri::WebviewWindow;
use webkit2gtk::WebViewExt;

/// Loads the page again when WebKit's web process dies, as it can under GPU pressure on
/// Wayland, instead of leaving the window blank. The app comes back from the engine's state.
pub fn reload_after_crash(window: &WebviewWindow) {
    let watched = window.with_webview(|webview| {
        webview
            .inner()
            .connect_web_process_terminated(|view, reason| {
                eprintln!("hoshi: the webview's process ended ({reason:?}); reloading it");
                view.reload();
            });
    });
    if let Err(error) = watched {
        eprintln!("hoshi: can't watch the webview's process: {error}");
    }
}
