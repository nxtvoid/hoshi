//! Windows shows toasts only for a registered AppUserModelID. Without an installer the app
//! registers its own under `HKCU\Software\Classes\AppUserModelId`, with the name and icon toasts
//! show.

use std::path::Path;

#[cfg(debug_assertions)]
pub const APP_ID: &str = "dev.hoshi.app.native-dev";
#[cfg(not(debug_assertions))]
pub const APP_ID: &str = crate::app_icon::APP_ID;

#[cfg(debug_assertions)]
const DISPLAY_NAME: &str = "hoshi native dev";
#[cfg(not(debug_assertions))]
const DISPLAY_NAME: &str = "hoshi";

#[cfg(debug_assertions)]
const ICON: &[u8] = include_bytes!("../icons/dev/128x128.png");
#[cfg(not(debug_assertions))]
const ICON: &[u8] = include_bytes!("../icons/128x128.png");

const ICON_FILE: &str = "notification-icon.png";

fn key_path(app_id: &str) -> String {
    format!(r"Software\Classes\AppUserModelId\{app_id}")
}

fn values(home: &Path) -> [(&'static str, String); 2] {
    [
        ("DisplayName", DISPLAY_NAME.to_owned()),
        (
            "IconUri",
            home.join(ICON_FILE).to_string_lossy().into_owned(),
        ),
    ]
}

/// Writes only what differs, so a start with everything in place reads and leaves.
pub fn register(home: &Path) {
    if let Err(error) = ensure(home) {
        eprintln!("hoshi: could not register for notifications: {error}");
    }
}

fn ensure(home: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let icon = home.join(ICON_FILE);
    if std::fs::read(&icon).ok().as_deref() != Some(ICON) {
        std::fs::write(&icon, ICON)?;
    }
    let key = windows_registry::CURRENT_USER.create(key_path(APP_ID))?;
    for (name, value) in values(home) {
        if key.get_string(name).ok().as_deref() != Some(value.as_str()) {
            key.set_string(name, &value)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{key_path, values};
    use std::path::Path;

    #[cfg(debug_assertions)]
    #[test]
    fn debug_builds_never_share_the_installed_id() {
        assert_ne!(super::APP_ID, crate::app_icon::APP_ID);
        assert_ne!(super::APP_ID, "dev.hoshi.app");
        assert_ne!(super::DISPLAY_NAME, "hoshi");
    }

    #[test]
    fn registers_the_name_and_the_icon_in_the_home() {
        assert_eq!(
            key_path("dev.hoshi.app"),
            r"Software\Classes\AppUserModelId\dev.hoshi.app"
        );
        let [(name, _), (icon, path)] = values(Path::new(r"C:\Users\nxtvoid\.hoshi"));
        assert_eq!(name, "DisplayName");
        assert_eq!(icon, "IconUri");
        assert_eq!(path, r"C:\Users\nxtvoid\.hoshi\notification-icon.png");
    }
}
