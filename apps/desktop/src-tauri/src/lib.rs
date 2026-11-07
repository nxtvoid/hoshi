mod orchestration;
mod settings;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| orchestration::init(app))
        .invoke_handler(tauri::generate_handler![
            orchestration::get_snapshot,
            orchestration::dispatch,
            orchestration::get_thread_items,
            settings::get_settings,
            settings::update_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running hoshi");
}
