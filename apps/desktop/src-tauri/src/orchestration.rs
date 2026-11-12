use hoshi_core::{
    AppSettings, BankedResetSpend, BrowserInput, ChangesSummary, Command, CommandError, DiffFile,
    DiffScope, Editor, FileDiff, FolderListing, ForgeAccount, ForgeKind, GitActionRequest,
    GitActionResult, GitStatus, HeldMessage, LocalServer, MessageAttachment, ProviderAccountStatus,
    ProviderModels, ProviderVersion, PullRequest, PullRequestAction, PullRequestDetail,
    PullRequestLabel, RemoteRepository, SlashCommand, Snapshot, TerminalAttach, TerminalEvent,
    ThreadItem, TranscriptPage, UsageLimits, WorkspaceEntry,
};
use hoshi_engine::{Engine, Notification, Notify};
use hoshi_providers::{ClaudeCodeAdapter, ProviderAdapter};
use std::sync::Arc;
use tauri::ipc::{Channel, InvokeBody, Request};
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
/// The commands a thread's running turn waits for in the background.
const BACKGROUND_TASKS_CHANNEL: &str = "hoshi://background-tasks";
const HELD_MESSAGE_CHANNEL: &str = "hoshi://held-message";
/// The provider plan's limits, as a running turn reported them.
const USAGE_LIMITS_CHANNEL: &str = "hoshi://usage-limits";
/// The provider's CLI against its latest release, and how its update goes.
const PROVIDER_VERSION_CHANNEL: &str = "hoshi://provider-version";
/// The step a running commit, push or pull request action reached.
const GIT_ACTION_CHANNEL: &str = "hoshi://git-action";
/// What a thread's browser page painted, while the panel shows it.
const BROWSER_FRAME_CHANNEL: &str = "hoshi://browser-frame";
/// Where a thread's browser page is, and what the agent just did on it.
const BROWSER_PAGE_CHANNEL: &str = "hoshi://browser-page";
const WORKTREE_SETUP_CHANNEL: &str = "hoshi://worktree-setup";
/// The servers of a thread a view watches, when they change.
const SERVERS_CHANNEL: &str = "hoshi://servers";

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
            Notification::BackgroundTasks(update) => handle.emit(BACKGROUND_TASKS_CHANNEL, update),
            Notification::UsageLimits(limits) => handle.emit(USAGE_LIMITS_CHANNEL, limits),
            Notification::ProviderVersion(version) => {
                handle.emit(PROVIDER_VERSION_CHANNEL, version)
            }
            Notification::GitActionProgress(progress) => handle.emit(GIT_ACTION_CHANNEL, progress),
            Notification::BrowserFrame(frame) => handle.emit(BROWSER_FRAME_CHANNEL, frame),
            Notification::BrowserPage(page) => handle.emit(BROWSER_PAGE_CHANNEL, page),
            Notification::HeldMessage(held) => handle.emit(HELD_MESSAGE_CHANNEL, held),
            Notification::WorktreeSetup(run) => handle.emit(WORKTREE_SETUP_CHANNEL, run),
            Notification::Servers(update) => handle.emit(SERVERS_CHANNEL, update),
        };
        if let Err(error) = result {
            eprintln!("hoshi: failed to emit to the UI: {error}");
        }
    });
    let engine = Engine::open(data_dir, notify)?;
    let settings = crate::settings::load(&crate::settings::settings_path(app.handle())?);
    engine.set_adapter(adapter_for(&settings));
    engine.set_thread_titles(settings.thread_titles.clone());
    engine.set_browser(settings.browser.clone());
    engine.set_subagent_effort(settings.providers.claude_code.subagent_effort);
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

/// Cancels a delegated task: it stops if it runs, and its answer never goes back.
#[tauri::command(async)]
pub fn cancel_task(engine: State<'_, Engine>, thread_id: String) {
    engine.cancel_task(&thread_id);
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

/// Spends the next banked reset and reads the limits again, which takes a few seconds.
#[tauri::command(async)]
pub fn spend_banked_reset(engine: State<'_, Engine>) -> BankedResetSpend {
    engine.spend_banked_reset()
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

/// Runs the CLI, which takes a few seconds.
#[tauri::command(async)]
pub fn get_provider_account_status(engine: State<'_, Engine>) -> ProviderAccountStatus {
    engine.provider_account_status()
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

/// Saves a file the user is about to send. Its bytes are the request's body; its name and
/// type come percent-encoded in the `x-name` and `x-mime-type` headers.
#[tauri::command(async)]
pub fn save_file(
    engine: State<'_, Engine>,
    request: Request<'_>,
) -> Result<MessageAttachment, CommandError> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(CommandError::Internal {
            detail: "the file's bytes are missing".into(),
        });
    };
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(percent_decoded)
            .unwrap_or_default()
    };
    let mime_type = header("x-mime-type");
    let mime_type = if mime_type.is_empty() {
        "application/octet-stream".to_owned()
    } else {
        mime_type
    };
    engine.save_file(&header("x-name"), &mime_type, bytes)
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// An image or file removed from the composer before it was sent.
#[tauri::command(async)]
pub fn discard_attachment(engine: State<'_, Engine>, attachment: MessageAttachment) {
    engine.discard_attachment(&attachment);
}

/// Where a sent image or file is saved, to open it; `None` when it is gone.
#[tauri::command(async)]
pub fn get_attachment_path(
    engine: State<'_, Engine>,
    attachment: MessageAttachment,
) -> Option<String> {
    engine.attachment_path(&attachment)
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
pub fn get_local_image(engine: State<'_, Engine>, path: String) -> Option<String> {
    engine.local_image(&path)
}

#[tauri::command(async)]
pub fn open_local_file(
    engine: State<'_, Engine>,
    path: String,
    editor: Option<Editor>,
) -> Result<(), CommandError> {
    engine.open_local_file(&path, editor)
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

/// The thread's servers now; changes arrive on `hoshi://servers` until `unwatch_servers`.
#[tauri::command(async)]
pub fn watch_servers(engine: State<'_, Engine>, thread_id: String) -> Vec<LocalServer> {
    engine.watch_servers(&thread_id)
}

#[tauri::command(async)]
pub fn unwatch_servers(engine: State<'_, Engine>, thread_id: String) {
    engine.unwatch_servers(&thread_id);
}

/// Takes up to a few seconds while the server shuts down.
#[tauri::command(async)]
pub fn stop_server(
    engine: State<'_, Engine>,
    thread_id: String,
    pid: u32,
    port: u16,
) -> Result<(), CommandError> {
    engine.stop_server(&thread_id, pid, port)
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

/// Compacts the thread's conversation first, then sends the message.
#[tauri::command(async)]
pub fn compact_then_send(
    engine: State<'_, Engine>,
    thread_id: String,
    message_id: String,
    text: String,
    attachments: Vec<MessageAttachment>,
) -> Result<(), CommandError> {
    engine.compact_then_send(&thread_id, &message_id, text, attachments)
}

/// Stops compacting the thread, so the message it holds goes at once.
#[tauri::command(async)]
pub fn send_held_message_now(
    engine: State<'_, Engine>,
    thread_id: String,
) -> Result<(), CommandError> {
    engine.send_held_message_now(&thread_id)
}

/// Takes back the message the thread holds until it is compacted; `discard` drops it.
#[tauri::command(async)]
pub fn take_held_message(
    engine: State<'_, Engine>,
    thread_id: String,
    discard: bool,
) -> Option<HeldMessage> {
    engine.take_held_message(&thread_id, discard)
}

#[tauri::command(async)]
pub fn browser_view(
    engine: State<'_, Engine>,
    thread_id: String,
    viewing: bool,
) -> Result<(), CommandError> {
    engine.browser_view(&thread_id, viewing)
}

#[tauri::command(async)]
pub fn browser_open(
    engine: State<'_, Engine>,
    thread_id: String,
    url: String,
) -> Result<(), CommandError> {
    engine.browser_open(&thread_id, &url)
}

/// Back (`-1`), forward (`1`) or reload (`0`).
#[tauri::command(async)]
pub fn browser_go(
    engine: State<'_, Engine>,
    thread_id: String,
    delta: i32,
) -> Result<(), CommandError> {
    engine.browser_go(&thread_id, delta)
}

#[tauri::command(async)]
pub fn browser_resize(
    engine: State<'_, Engine>,
    thread_id: String,
    width: u32,
    height: u32,
) -> Result<(), CommandError> {
    engine.browser_resize(&thread_id, width, height)
}

#[tauri::command(async)]
pub fn browser_fit(
    engine: State<'_, Engine>,
    thread_id: String,
    width: u32,
    height: u32,
) -> Result<(), CommandError> {
    engine.browser_fit(&thread_id, width, height)
}

#[tauri::command(async)]
pub fn browser_input(
    engine: State<'_, Engine>,
    thread_id: String,
    input: BrowserInput,
) -> Result<(), CommandError> {
    engine.browser_input(&thread_id, &input)
}

#[tauri::command(async)]
pub fn browser_close(engine: State<'_, Engine>, thread_id: String) {
    engine.browser_close(&thread_id);
}

#[tauri::command]
pub fn worktree_setups(engine: State<'_, Engine>) -> Vec<hoshi_core::WorktreeSetupRun> {
    engine.worktree_setups()
}

#[tauri::command(async)]
pub fn rerun_worktree_setup(
    engine: State<'_, Engine>,
    thread_id: String,
) -> Result<(), CommandError> {
    engine.rerun_worktree_setup(&thread_id)
}

#[tauri::command(async)]
pub fn skip_worktree_setup(
    engine: State<'_, Engine>,
    thread_id: String,
) -> Result<(), CommandError> {
    engine.skip_worktree_setup(&thread_id)
}

/// The user's answer to an agent's secret request; `None` declines. The value only reaches
/// a file the agent's commands read.
#[tauri::command(async)]
pub fn answer_secret_request(
    engine: State<'_, Engine>,
    thread_id: String,
    request_id: String,
    secret: Option<String>,
) -> Result<(), CommandError> {
    engine.answer_secret_request(&thread_id, &request_id, secret.as_deref())
}

#[tauri::command(async)]
pub fn get_git_status(
    engine: State<'_, Engine>,
    thread_id: String,
) -> Result<GitStatus, CommandError> {
    engine.git_status(&thread_id)
}

#[tauri::command(async)]
pub fn run_git_action(
    engine: State<'_, Engine>,
    thread_id: String,
    request: GitActionRequest,
) -> Result<GitActionResult, CommandError> {
    engine.run_git_action(&thread_id, &request)
}

#[tauri::command(async)]
pub fn rewind_thread(
    engine: State<'_, Engine>,
    thread_id: String,
    message_id: String,
    revert_files: bool,
) -> Result<(), CommandError> {
    engine.rewind_thread(&thread_id, &message_id, revert_files)
}

#[tauri::command(async)]
pub fn get_turn_diff_files(
    engine: State<'_, Engine>,
    thread_id: String,
    turn_id: String,
) -> Result<Vec<DiffFile>, CommandError> {
    engine.turn_diff_files(&thread_id, &turn_id)
}

#[tauri::command(async)]
pub fn get_turn_file_diff(
    engine: State<'_, Engine>,
    thread_id: String,
    turn_id: String,
    file: DiffFile,
) -> Result<FileDiff, CommandError> {
    engine.turn_file_diff(&thread_id, &turn_id, &file)
}

/// Lets `AppHandle` users reach the engine without a `State` parameter.
pub fn engine(app: &AppHandle) -> State<'_, Engine> {
    app.state::<Engine>()
}
