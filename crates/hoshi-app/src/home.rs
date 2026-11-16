//! hoshi keeps everything in one folder: `~/.hoshi`, or `~/.hoshi-dev` for development builds
//! so they never touch the installed app's data. `HOSHI_HOME` points it elsewhere.

use std::io;
use std::path::{Path, PathBuf};

const SETTINGS_FILE: &str = "settings.json";

pub fn settings_path(home: &Path) -> PathBuf {
    home.join(SETTINGS_FILE)
}

pub fn resolve() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("HOSHI_HOME").filter(|dir| !dir.is_empty()) {
        let dir = PathBuf::from(dir);
        return if dir.is_absolute() {
            Ok(dir)
        } else {
            Ok(std::env::current_dir()?.join(dir))
        };
    }
    let home = std::env::home_dir().ok_or_else(|| io::Error::other("no home folder"))?;
    let name = if cfg!(debug_assertions) {
        ".hoshi-dev"
    } else {
        ".hoshi"
    };
    Ok(home.join(name))
}
