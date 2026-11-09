// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    if let Some(code) = relaunch_without_dmabuf() {
        std::process::exit(code);
    }
    hoshi_desktop_lib::run()
}

/// WebKitGTK's DMA-BUF renderer shows a stale, wrongly scaled frame when GNOME brings the
/// window back from the overview. The variable must be set before WebKit starts, which safe
/// Rust can only do for a new process: hoshi runs itself again with it and waits. A value the
/// user set wins, and the relaunched process finds it set and runs normally.
#[cfg(target_os = "linux")]
fn relaunch_without_dmabuf() -> Option<i32> {
    const VARIABLE: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    if std::env::var_os(VARIABLE).is_some() {
        return None;
    }
    let status = std::process::Command::new(std::env::current_exe().ok()?)
        .args(std::env::args_os().skip(1))
        .env(VARIABLE, "1")
        .status()
        .ok()?;
    Some(status.code().unwrap_or(1))
}
