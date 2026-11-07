use hoshi_core::{AppSettings, Command, CommandError, Snapshot, ThreadItem};
use hoshi_engine::{Engine, Notification, Notify};
use hoshi_providers::{ClaudeCodeAdapter, ProviderAdapter};
use std::sync::Arc;
use tauri::{App, AppHandle, Emitter, Manager, State};

/// Every committed event reaches the UI here, in sequence order.
const EVENT_CHANNEL: &str = "hoshi://event";
/// Assistant text as it streams; not stored, not sequenced.
const STREAM_CHANNEL: &str = "hoshi://stream";

/// Edits apply on their own; commands the agent's settings don't allow are refused.
const PERMISSION_MODE: &str = "acceptEdits";

pub fn init(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let handle = app.handle().clone();
    let notify: Notify = Arc::new(move |notification| {
        let result = match notification {
            Notification::Event(envelope) => handle.emit(EVENT_CHANNEL, envelope),
            Notification::Stream(delta) => handle.emit(STREAM_CHANNEL, delta),
        };
        if let Err(error) = result {
            eprintln!("hoshi: failed to emit to the UI: {error}");
        }
    });
    let engine = Engine::open(&data_dir.join("state.sqlite"), notify)?;
    let settings = crate::settings::load(&crate::settings::settings_path(app.handle())?);
    engine.set_adapter(adapter_for(&settings));
    app.manage(engine);
    Ok(())
}

pub fn adapter_for(settings: &AppSettings) -> Option<Arc<dyn ProviderAdapter>> {
    let claude = &settings.providers.claude_code;
    claude.enabled.then(|| {
        Arc::new(ClaudeCodeAdapter {
            binary_path: claude.binary_path.clone(),
            model: None,
            permission_mode: PERMISSION_MODE.to_owned(),
        }) as Arc<dyn ProviderAdapter>
    })
}

#[tauri::command(async)]
pub fn get_snapshot(engine: State<'_, Engine>) -> Snapshot {
    engine.snapshot()
}

#[tauri::command(async)]
pub fn get_thread_items(
    engine: State<'_, Engine>,
    thread_id: String,
) -> Result<Vec<ThreadItem>, CommandError> {
    engine.thread_items(&thread_id)
}

#[tauri::command(async)]
pub fn dispatch(engine: State<'_, Engine>, command: Command) -> Result<(), CommandError> {
    engine.dispatch(command).inspect_err(|error| {
        if let CommandError::Internal { detail } = error {
            eprintln!("hoshi: dispatch failed: {detail}");
        }
    })
}

/// Lets `AppHandle` users reach the engine without a `State` parameter.
pub fn engine(app: &AppHandle) -> State<'_, Engine> {
    app.state::<Engine>()
}
