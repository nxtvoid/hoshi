use hoshi_core::{
    AppSettings, ChangesSummary, Command, CommandError, DiffFile, DiffScope, Editor, FileDiff,
    FolderListing, ForgeAccount, ForgeKind, MessageAttachment, ProviderModels, ProviderVersion,
    PullRequest, PullRequestAction, PullRequestDetail, PullRequestLabel, RemoteRepository,
    SlashCommand, Snapshot, TerminalAttach, TerminalEvent, ThreadItem, TranscriptPage, UsageLimits,
    WorkspaceEntry,
};
use hoshi_engine::{Engine, Notification, Notify};
use hoshi_providers::{ClaudeCodeAdapter, ProviderAdapter};
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::{App, AppHandle, Emitter, Manager, State};

/// Every committed event reaches the UI here, in sequence order.
const EVENT_CHANNEL: &str = "hoshi://event";
/// Assistant text as it streams; not stored, not sequenced.
const STREAM_CHANNEL: &str = "hoshi://stream";
/// A thread's pull request as last read from its forge.
const PULL_REQUEST_CHANNEL: &str = "hoshi://pull-request";
/// A fresh read of a pull request's detail, for the side panel.
const PULL_REQUEST_DETAIL_CHANNEL: &str = "hoshi://pull-request-detail";
/// A thread's context usage after each model call of its agent.
const CONTEXT_USAGE_CHANNEL: &str = "hoshi://context-usage";
/// The provider plan's limits, as a running turn reported them.
const USAGE_LIMITS_CHANNEL: &str = "hoshi://usage-limits";
/// The provider's CLI against its latest release, and how its update goes.
const PROVIDER_VERSION_CHANNEL: &str = "hoshi://provider-version";

pub fn init(app: &App, data_dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let notify: Notify = Arc::new(move |notification| {
        let result = match notification {
            Notification::Event(envelope) => handle.emit(EVENT_CHANNEL, envelope),
            Notification::Stream(delta) => handle.emit(STREAM_CHANNEL, delta),
            Notification::PullRequest(update) => handle.emit(PULL_REQUEST_CHANNEL, update),
            Notification::PullRequestDetail(update) => {
                handle.emit(PULL_REQUEST_DETAIL_CHANNEL, update)
            }
            Notification::ContextUsage(update) => handle.emit(CONTEXT_USAGE_CHANNEL, update),
            Notification::UsageLimits(limits) => handle.emit(USAGE_LIMITS_CHANNEL, limits),
            Notification::ProviderVersion(version) => {
                handle.emit(PROVIDER_VERSION_CHANNEL, version)
            }
        };
        if let Err(error) = result {
            eprintln!("hoshi: failed to emit to the UI: {error}");
        }
    });
    let engine = Engine::open(data_dir, notify)?;
    let settings = crate::settings::load(&crate::settings::settings_path(app.handle())?);
    engine.set_adapter(adapter_for(&settings));
    engine.set_thread_titles(settings.thread_titles.clone());
    engine.set_forges(hoshi_engine::default_forges());
    app.manage(engine);
    Ok(())
}

pub fn adapter_for(settings: &AppSettings) -> Option<Arc<dyn ProviderAdapter>> {
    let claude = &settings.providers.claude_code;
    claude.enabled.then(|| {
        Arc::new(ClaudeCodeAdapter::new(claude.binary_path.clone(), None))
            as Arc<dyn ProviderAdapter>
    })
}

#[tauri::command(async)]
pub fn get_snapshot(engine: State<'_, Engine>) -> Snapshot {
    engine.snapshot()
}

/// The latest turns before item `before`, or the last ones when it is `None`.
#[tauri::command(async)]
pub fn get_thread_items(
    engine: State<'_, Engine>,
    thread_id: String,
    before: Option<String>,
) -> Result<TranscriptPage, CommandError> {
    engine.transcript(&thread_id, before.as_deref())
}

/// One transcript item with what `get_thread_items` leaves out, such as a diff.
#[tauri::command(async)]
pub fn get_thread_item(
    engine: State<'_, Engine>,
    thread_id: String,
    item_id: String,
) -> Result<Option<ThreadItem>, CommandError> {
    engine.thread_item(&thread_id, &item_id)
}

#[tauri::command(async)]
pub fn dispatch(engine: State<'_, Engine>, command: Command) -> Result<(), CommandError> {
    engine.dispatch(command).inspect_err(|error| {
        if let CommandError::Internal { detail } = error {
            eprintln!("hoshi: dispatch failed: {detail}");
        }
    })
}

/// Hands a delegated task's answer back now, as it stands.
#[tauri::command(async)]
pub fn send_task_answer(engine: State<'_, Engine>, thread_id: String) {
    engine.send_task_answer(&thread_id);
}

/// `None` when the project has no icon among its files.
#[tauri::command(async)]
pub fn get_project_icon(engine: State<'_, Engine>, project_id: String) -> Option<String> {
    engine.project_icon(&project_id)
}

/// `None` while no provider is enabled.
#[tauri::command(async)]
pub fn get_models(engine: State<'_, Engine>) -> Option<ProviderModels> {
    engine.models()
}

/// The limits last read, kept across restarts; `None` until the provider was read once.
#[tauri::command(async)]
pub fn get_known_usage_limits(engine: State<'_, Engine>) -> Option<UsageLimits> {
    engine.known_usage_limits()
}

/// Asks the provider unless it answered within the last minute, or `force` is set; asking
/// takes a few seconds. `None` when it can't say.
#[tauri::command(async)]
pub fn get_usage_limits(engine: State<'_, Engine>, force: bool) -> Option<UsageLimits> {
    engine.usage_limits(force)
}

/// The provider's CLI as last checked; `None` until checked or while disabled.
#[tauri::command(async)]
pub fn get_provider_version(engine: State<'_, Engine>) -> Option<ProviderVersion> {
    engine.provider_version()
}

/// Runs the CLI and looks up its latest release, which takes a moment.
#[tauri::command(async)]
pub fn check_provider_version(engine: State<'_, Engine>) -> Option<ProviderVersion> {
    engine.check_provider_version()
}

/// Returns at once; progress arrives on `hoshi://provider-version`.
#[tauri::command]
pub fn update_provider(engine: State<'_, Engine>) {
    engine.update_provider();
}

/// For `@` mentions, in the thread's folder or, before it exists, the project's.
#[tauri::command(async)]
pub fn search_paths(
    engine: State<'_, Engine>,
    project_id: Option<String>,
    thread_id: Option<String>,
    query: String,
) -> Result<Vec<WorkspaceEntry>, CommandError> {
    engine.search_paths(project_id.as_deref(), thread_id.as_deref(), &query)
}

/// Saves an image the user is about to send; `data` is its bytes in base64.
#[tauri::command(async)]
pub fn save_image(
    engine: State<'_, Engine>,
    name: String,
    mime_type: String,
    data: String,
) -> Result<MessageAttachment, CommandError> {
    engine.save_image(&name, &mime_type, &data)
}

/// An image removed from the composer before it was sent.
#[tauri::command(async)]
pub fn discard_image(engine: State<'_, Engine>, image: MessageAttachment) {
    engine.discard_image(&image);
}

/// A sent image as a data URL, read when the transcript shows it.
#[tauri::command(async)]
pub fn get_image(engine: State<'_, Engine>, id: String, mime_type: String) -> Option<String> {
    engine.image_data_url(&id, &mime_type)
}

/// Names the thread again from its conversation, in the background.
#[tauri::command(async)]
pub fn regenerate_thread_title(
    engine: State<'_, Engine>,
    thread_id: String,
) -> Result<(), CommandError> {
    engine.regenerate_title(&thread_id)
}

/// For `#` mentions. Asks the forge.
#[tauri::command(async)]
pub fn search_pull_requests(
    engine: State<'_, Engine>,
    project_id: String,
    query: String,
) -> Result<Vec<PullRequest>, CommandError> {
    engine.search_pull_requests(&project_id, &query)
}

/// The first read of a folder runs the provider's CLI, which takes a few seconds.
#[tauri::command(async)]
pub fn get_slash_commands(
    engine: State<'_, Engine>,
    project_id: Option<String>,
    thread_id: Option<String>,
) -> Result<Option<Vec<SlashCommand>>, CommandError> {
    engine.slash_commands(project_id.as_deref(), thread_id.as_deref())
}

#[tauri::command(async)]
pub fn get_forge_accounts(engine: State<'_, Engine>) -> Vec<ForgeAccount> {
    engine.forge_accounts()
}

#[tauri::command(async)]
pub fn list_folders(
    engine: State<'_, Engine>,
    path: String,
) -> Result<FolderListing, CommandError> {
    engine.list_folders(&path)
}

#[tauri::command(async)]
pub fn create_folder(
    engine: State<'_, Engine>,
    path: String,
    init_repository: bool,
) -> Result<String, CommandError> {
    engine.create_folder(&path, init_repository)
}

#[tauri::command(async)]
pub fn clone_repository(
    engine: State<'_, Engine>,
    url: String,
    destination: String,
) -> Result<String, CommandError> {
    engine.clone_repository(&url, &destination)
}

#[tauri::command(async)]
pub fn find_repository(
    engine: State<'_, Engine>,
    forge: ForgeKind,
    input: String,
) -> Result<RemoteRepository, CommandError> {
    engine.find_repository(forge, &input)
}

#[tauri::command(async)]
pub fn connect_bitbucket(
    engine: State<'_, Engine>,
    email: String,
    token: String,
) -> Result<ForgeAccount, CommandError> {
    engine.connect_bitbucket(&email, &token)
}

#[tauri::command(async)]
pub fn release_worktree(engine: State<'_, Engine>, thread_id: String) -> Result<(), CommandError> {
    engine.release_worktree(&thread_id)
}

#[tauri::command(async)]
pub fn clean_up_thread(engine: State<'_, Engine>, thread_id: String) -> Result<(), CommandError> {
    engine.clean_up_thread(&thread_id)
}

#[tauri::command(async)]
pub fn disconnect_forge(engine: State<'_, Engine>, forge: ForgeKind) -> Result<(), CommandError> {
    engine.disconnect_forge(forge)
}

#[tauri::command(async)]
pub fn get_pull_request_detail(
    engine: State<'_, Engine>,
    thread_id: String,
) -> Option<PullRequestDetail> {
    engine.pull_request_detail(&thread_id)
}

#[tauri::command(async)]
pub fn run_pull_request_action(
    engine: State<'_, Engine>,
    thread_id: String,
    forge_id: String,
    action: PullRequestAction,
) -> Result<(), CommandError> {
    engine.run_pull_request_action(&thread_id, &forge_id, &action)
}

#[tauri::command(async)]
pub fn get_reviewer_candidates(
    engine: State<'_, Engine>,
    thread_id: String,
    query: String,
) -> Result<Vec<String>, CommandError> {
    engine.reviewer_candidates(&thread_id, &query)
}

#[tauri::command(async)]
pub fn get_repository_labels(
    engine: State<'_, Engine>,
    thread_id: String,
) -> Result<Vec<PullRequestLabel>, CommandError> {
    engine.repository_labels(&thread_id)
}

#[tauri::command(async)]
pub fn get_editors(engine: State<'_, Engine>) -> Vec<Editor> {
    engine.editors()
}

#[tauri::command(async)]
pub fn get_thread_branches(
    engine: State<'_, Engine>,
    thread_id: String,
) -> Result<Vec<String>, CommandError> {
    engine.thread_branches(&thread_id)
}

#[tauri::command(async)]
pub fn get_thread_changes(
    engine: State<'_, Engine>,
    thread_id: String,
) -> Result<ChangesSummary, CommandError> {
    engine.thread_changes(&thread_id)
}

#[tauri::command(async)]
pub fn get_thread_folder(engine: State<'_, Engine>, thread_id: String) -> Option<String> {
    engine.thread_folder(&thread_id)
}

#[tauri::command(async)]
pub fn open_in_editor(
    engine: State<'_, Engine>,
    thread_id: String,
    editor: Editor,
) -> Result<(), CommandError> {
    engine.open_in_editor(&thread_id, editor)
}

#[tauri::command(async)]
pub fn open_terminal(
    engine: State<'_, Engine>,
    thread_id: String,
    terminal_id: String,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> Result<TerminalAttach, CommandError> {
    engine.open_terminal(
        &thread_id,
        &terminal_id,
        (cols, rows),
        Box::new(move |event| {
            let _ = on_event.send(event);
        }),
    )
}

#[tauri::command(async)]
pub fn write_terminal(
    engine: State<'_, Engine>,
    terminal_id: String,
    data: String,
) -> Result<(), CommandError> {
    engine.write_terminal(&terminal_id, &data)
}

#[tauri::command(async)]
pub fn resize_terminal(
    engine: State<'_, Engine>,
    terminal_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), CommandError> {
    engine.resize_terminal(&terminal_id, (cols, rows))
}

#[tauri::command(async)]
pub fn detach_terminal(engine: State<'_, Engine>, terminal_id: String) {
    engine.detach_terminal(&terminal_id);
}

#[tauri::command(async)]
pub fn close_terminal(engine: State<'_, Engine>, terminal_id: String) {
    engine.close_terminal(&terminal_id);
}

#[tauri::command(async)]
pub fn switch_branch(
    engine: State<'_, Engine>,
    thread_id: String,
    branch: String,
) -> Result<(), CommandError> {
    engine.switch_branch(&thread_id, &branch)
}

#[tauri::command(async)]
pub fn get_linked_pull_request(
    engine: State<'_, Engine>,
    url: String,
) -> Result<Option<PullRequest>, CommandError> {
    engine.linked_pull_request(&url)
}

#[tauri::command(async)]
pub fn prefetch_pull_request(engine: State<'_, Engine>, thread_id: String) {
    engine.prefetch_pull_request(&thread_id);
}

#[tauri::command(async)]
pub fn get_pull_request_files(
    engine: State<'_, Engine>,
    thread_id: String,
) -> Option<Vec<DiffFile>> {
    engine.pull_request_files(&thread_id)
}

#[tauri::command(async)]
pub fn get_pull_request_file_diff(
    engine: State<'_, Engine>,
    thread_id: String,
    path: String,
) -> Option<FileDiff> {
    engine.pull_request_file_diff(&thread_id, &path)
}

#[tauri::command(async)]
pub fn get_thread_diff_files(
    engine: State<'_, Engine>,
    thread_id: String,
    scope: DiffScope,
) -> Result<Vec<DiffFile>, CommandError> {
    engine.thread_diff_files(&thread_id, scope)
}

#[tauri::command(async)]
pub fn get_thread_file_diff(
    engine: State<'_, Engine>,
    thread_id: String,
    scope: DiffScope,
    file: DiffFile,
) -> Result<FileDiff, CommandError> {
    engine.thread_file_diff(&thread_id, scope, &file)
}

/// Lets `AppHandle` users reach the engine without a `State` parameter.
pub fn engine(app: &AppHandle) -> State<'_, Engine> {
    app.state::<Engine>()
}
