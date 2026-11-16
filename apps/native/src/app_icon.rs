//! The app's icon where the platform takes it at runtime: the dock on macOS, the window and its
//! desktop entry on Linux. Windows reads it from the executable, embedded by `build.rs`.

use std::sync::Arc;

/// Debug builds get their own id so the desktop keeps them apart from an installed hoshi.
#[cfg(debug_assertions)]
pub const APP_ID: &str = "dev.hoshi.app.dev";
#[cfg(not(debug_assertions))]
pub const APP_ID: &str = "dev.hoshi.app";

#[cfg(all(target_os = "macos", debug_assertions))]
const ICNS: &[u8] = include_bytes!("../icons/dev/icon.icns");
#[cfg(all(target_os = "macos", not(debug_assertions)))]
const ICNS: &[u8] = include_bytes!("../icons/icon.icns");

#[cfg(all(target_os = "linux", debug_assertions))]
const PNG: &[u8] = include_bytes!("../icons/dev/icon.png");
#[cfg(all(target_os = "linux", not(debug_assertions)))]
const PNG: &[u8] = include_bytes!("../icons/icon.png");

/// Sets what the platform can't read from a package: on macOS the dock icon, on Linux a desktop
/// entry for `APP_ID`, which Wayland needs to show any icon.
pub fn install() {
    #[cfg(target_os = "macos")]
    macos::set_dock_icon(ICNS);
    #[cfg(target_os = "linux")]
    std::thread::spawn(|| {
        if let Err(error) = linux::install_desktop_entry(PNG) {
            eprintln!("hoshi: could not install the desktop entry: {error}");
        }
    });
}

/// The window's own icon, which only X11 reads.
pub fn window_icon() -> Option<Arc<image::RgbaImage>> {
    #[cfg(target_os = "linux")]
    {
        image::load_from_memory_with_format(PNG, image::ImageFormat::Png)
            .ok()
            .map(|icon| Arc::new(icon.into_rgba8()))
    }
    #[cfg(not(target_os = "linux"))]
    None
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::AnyThread;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    pub fn set_dock_icon(icns: &[u8]) {
        let Some(main_thread) = MainThreadMarker::new() else {
            return;
        };
        let data = NSData::with_bytes(icns);
        let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
            return;
        };
        let app = NSApplication::sharedApplication(main_thread);
        unsafe { app.setApplicationIconImage(Some(&image)) };
    }
}

#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod linux {
    use super::APP_ID;
    use std::io;
    use std::path::{Path, PathBuf};

    pub fn install_desktop_entry(png: &[u8]) -> io::Result<()> {
        if installed_by_package() {
            return Ok(());
        }
        let Some(data_home) = data_home() else {
            return Ok(());
        };
        let icon = data_home
            .join("icons/hicolor/512x512/apps")
            .join(format!("{APP_ID}.png"));
        write_if_changed(&icon, png)?;
        let entry = desktop_entry(&std::env::current_exe()?, &icon, cfg!(debug_assertions));
        let path = data_home
            .join("applications")
            .join(format!("{APP_ID}.desktop"));
        write_if_changed(&path, entry.as_bytes())
    }

    fn installed_by_package() -> bool {
        let dirs = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|dirs| !dirs.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
        dirs.split(':').any(|dir| {
            Path::new(dir)
                .join("applications")
                .join(format!("{APP_ID}.desktop"))
                .exists()
        })
    }

    fn data_home() -> Option<PathBuf> {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")))
    }

    fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
        if std::fs::read(path).is_ok_and(|current| current == bytes) {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, bytes)
    }

    pub(super) fn desktop_entry(exe: &Path, icon: &Path, hidden: bool) -> String {
        let mut entry = format!(
            "[Desktop Entry]\nType=Application\nName=hoshi\nExec={}\nIcon={}\n\
             StartupWMClass={APP_ID}\nCategories=Development;\nTerminal=false\n",
            exec_value(&exe.to_string_lossy()),
            icon.to_string_lossy().replace('\\', "\\\\"),
        );
        if hidden {
            entry.push_str("NoDisplay=true\n");
        }
        entry
    }

    /// The program quoted as the Desktop Entry spec asks, then escaped as a string value.
    fn exec_value(program: &str) -> String {
        let mut quoted = String::from('"');
        for ch in program.chars() {
            if matches!(ch, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            quoted.push(ch);
        }
        quoted.push('"');
        quoted.replace('\\', "\\\\")
    }
}

#[cfg(test)]
mod tests {
    use super::linux::desktop_entry;
    use std::path::Path;

    #[test]
    fn the_desktop_entry_quotes_the_program() {
        let entry = desktop_entry(
            Path::new("/home/nxtvoid/my apps/$hoshi"),
            Path::new("/home/nxtvoid/.local/share/icons/hoshi.png"),
            false,
        );
        assert!(entry.contains("Exec=\"/home/nxtvoid/my apps/\\\\$hoshi\"\n"));
        assert!(entry.contains("Icon=/home/nxtvoid/.local/share/icons/hoshi.png\n"));
        assert!(!entry.contains("NoDisplay"));
    }

    #[test]
    fn debug_builds_stay_out_of_the_launcher() {
        let entry = desktop_entry(Path::new("/opt/hoshi"), Path::new("/opt/hoshi.png"), true);
        assert!(entry.ends_with("NoDisplay=true\n"));
    }
}
