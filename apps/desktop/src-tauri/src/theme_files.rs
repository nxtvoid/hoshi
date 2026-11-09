use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

/// Saves an exported theme where the user picks; `false` when they cancelled.
#[tauri::command(async)]
pub fn export_theme(app: AppHandle, file_name: String, contents: String) -> Result<bool, String> {
    let Some(path) = app
        .dialog()
        .file()
        .set_file_name(&file_name)
        .add_filter("JSON", &["json"])
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|_| "invalid_path".to_owned())?;
    std::fs::write(path, contents).map_err(|_| "write_failed".to_owned())?;
    Ok(true)
}
