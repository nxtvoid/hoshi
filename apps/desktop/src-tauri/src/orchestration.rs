use hoshi_core::{Command, CommandError, Snapshot};
use hoshi_engine::Engine;
use std::sync::{Mutex, MutexGuard};
use tauri::{App, AppHandle, Emitter, Manager, State};

/// Every committed event reaches the UI on this channel, in sequence order.
const EVENT_CHANNEL: &str = "hoshi://event";

pub struct EngineState(Mutex<Engine>);

impl EngineState {
    fn lock(&self) -> MutexGuard<'_, Engine> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub fn init(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let engine = Engine::open(&data_dir.join("state.sqlite"))?;
    app.manage(EngineState(Mutex::new(engine)));
    Ok(())
}

#[tauri::command(async)]
pub fn get_snapshot(state: State<'_, EngineState>) -> Snapshot {
    state.lock().snapshot()
}

#[tauri::command(async)]
pub fn dispatch(
    app: AppHandle,
    state: State<'_, EngineState>,
    command: Command,
) -> Result<(), CommandError> {
    let mut engine = state.lock();
    let envelopes = engine.dispatch(command).inspect_err(|error| {
        if let CommandError::Internal { detail } = error {
            eprintln!("hoshi: dispatch failed: {detail}");
        }
    })?;
    // Emitting under the lock keeps events from concurrent dispatches in order.
    for envelope in envelopes {
        if let Err(error) = app.emit(EVENT_CHANNEL, envelope) {
            eprintln!("hoshi: failed to emit event: {error}");
        }
    }
    Ok(())
}
