mod clipboard;
mod home;
mod orchestration;
mod settings;

use tauri::{App, Manager, WebviewWindowBuilder};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(setup)
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(true) = event {
                let app = window.app_handle().clone();
                std::thread::spawn(move || orchestration::engine(&app).refresh_pull_requests());
            }
        })
        .invoke_handler(tauri::generate_handler![
            orchestration::get_snapshot,
            orchestration::dispatch,
            orchestration::get_thread_items,
            orchestration::get_thread_item,
            orchestration::send_task_answer,
            orchestration::cancel_task,
            orchestration::get_forge_accounts,
            orchestration::list_folders,
            orchestration::create_folder,
            orchestration::clone_repository,
            orchestration::find_repository,
            orchestration::connect_bitbucket,
            orchestration::disconnect_forge,
            orchestration::release_worktree,
            orchestration::clean_up_thread,
            orchestration::get_pull_request_detail,
            orchestration::get_pull_request_files,
            orchestration::prefetch_pull_request,
            orchestration::get_linked_pull_request,
            orchestration::get_editors,
            orchestration::get_thread_branches,
            orchestration::get_thread_changes,
            orchestration::open_in_editor,
            orchestration::get_thread_folder,
            orchestration::switch_branch,
            orchestration::open_terminal,
            orchestration::write_terminal,
            orchestration::resize_terminal,
            orchestration::detach_terminal,
            orchestration::close_terminal,
            orchestration::run_pull_request_action,
            orchestration::get_reviewer_candidates,
            orchestration::get_repository_labels,
            orchestration::get_pull_request_file_diff,
            orchestration::get_thread_diff_files,
            orchestration::get_thread_file_diff,
            orchestration::get_turn_diff_files,
            orchestration::rewind_thread,
            orchestration::answer_secret_request,
            orchestration::compact_then_send,
            orchestration::browser_view,
            orchestration::browser_open,
            orchestration::browser_go,
            orchestration::browser_resize,
            orchestration::browser_fit,
            orchestration::browser_input,
            orchestration::browser_close,
            orchestration::worktree_setups,
            orchestration::rerun_worktree_setup,
            orchestration::skip_worktree_setup,
            orchestration::get_git_status,
            orchestration::run_git_action,
            orchestration::get_turn_file_diff,
            orchestration::get_models,
            orchestration::get_known_usage_limits,
            orchestration::get_usage_limits,
            orchestration::get_provider_version,
            orchestration::check_provider_version,
            orchestration::get_provider_account_status,
            orchestration::update_provider,
            orchestration::get_project_icon,
            orchestration::search_paths,
            orchestration::get_slash_commands,
            clipboard::read_image_file,
            clipboard::read_clipboard_image,
            orchestration::save_image,
            orchestration::discard_image,
            orchestration::get_image,
            orchestration::search_pull_requests,
            orchestration::regenerate_thread_title,
            settings::get_settings,
            settings::update_settings
        ])
        .build(tauri::generate_context!())
        .expect("error while building hoshi")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event
                && let Some(engine) = app.try_state::<hoshi_engine::Engine>()
            {
                engine.close_browser();
            }
        });
}

/// Everything lives under one folder, the webview's storage included, so the window is
/// built here rather than from the config.
fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let home = home::resolve()?;
    let paths = app.path();
    let legacy = home::Legacy {
        data: paths.app_data_dir()?,
        config: paths.app_config_dir()?,
        local: paths.app_local_data_dir()?,
    };
    if home::adopt_legacy(&home, &legacy).map_err(|error| {
        format!(
            "could not move hoshi's data into {}: {error}",
            home.display()
        )
    })? {
        eprintln!("hoshi: moved its data into {}", home.display());
    }
    std::fs::create_dir_all(&home)?;
    app.manage(home::Home(home.clone()));
    orchestration::init(app, &home)?;
    let window = app
        .config()
        .app
        .windows
        .first()
        .cloned()
        .ok_or("tauri.conf.json declares no window")?;
    WebviewWindowBuilder::from_config(app.handle(), &window)?
        .data_directory(home.join(home::WEBVIEW_DIR))
        .build()?;
    Ok(())
}
