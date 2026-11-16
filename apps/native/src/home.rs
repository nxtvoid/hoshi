//! The app's home: `~/.hoshi`, or `~/.hoshi-dev` for debug builds, as `hoshi_app::home` decides.

use std::fs;
use std::io;
use std::path::PathBuf;

pub fn resolve() -> io::Result<PathBuf> {
    let home = hoshi_app::home::resolve()?;
    fs::create_dir_all(&home)?;
    Ok(home)
}
