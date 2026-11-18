use crate::sections::{Logins, PullRequests};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui_kit::*;
use hoshi_core::{
    AccessMode, AppSettings, Command, CommandError, Effort, Event, FavoriteModel, ItemContent,
    MessageAttachment, ModelInfo, ProviderModels, ReadModel, StreamDelta, StreamKind, ThreadItem,
    ThreadStatus, ThreadWorkspace, ToolDetail, TranscriptPage,
};
use hoshi_engine::{Engine, Notification, Notify, ProvidersStatus};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// Engine notifications arrive on its threads; the UI reads them from this channel.
pub fn channel() -> (Notify, UnboundedReceiver<Notification>) {
    let (sender, receiver): (UnboundedSender<Notification>, _) = unbounded();
    let notify: Notify = Arc::new(move |notification| {
        let _ = sender.unbounded_send(notification);
    });
    (notify, receiver)
}

/// What the next turn runs with.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelChoice {
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub fast_mode: bool,
    pub ultracode: bool,
}

impl Default for ModelChoice {
    fn default() -> Self {
        Self {
            provider: hoshi_core::default_provider(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
        }
    }
}

/// What a new thread starts with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DraftChoices {
    pub model: ModelChoice,
    pub access_mode: AccessMode,
    pub plan_mode: bool,
    /// In a new worktree instead of the project's folder.
    pub worktree: bool,
}

/// A thread not started yet: the composer's first message creates it.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    /// `None` for a chat.
    pub project_id: Option<String>,
}

/// How many transcripts stay loaded: the open one and those open just before it.
const KEPT_TRANSCRIPTS: usize = 4;

/// A failed read of the usage limits is tried again this soon, not at the next check.
const LIMITS_RETRY: std::time::Duration = std::time::Duration::from_secs(30);

/// The providers whose usage limits the health check reads: only while the thread's card shows
/// them, and only the enabled ones that report limits and are known to be signed in.
fn limits_to_refresh(
    settings: &hoshi_core::AppSettings,
    accounts: &HashMap<String, hoshi_core::ProviderAccountStatus>,
) -> Vec<&'static str> {
    if !settings.usage_limits_in_details {
        return Vec::new();
    }
    crate::providers::KNOWN_PROVIDERS
        .iter()
        .filter(|provider| matches!(provider.usage, crate::providers::UsageReport::Limits))
        .filter(|provider| provider.enabled(&settings.providers))
        .filter(|provider| {
            accounts.get(provider.id) == Some(&hoshi_core::ProviderAccountStatus::Ready)
        })
        .map(|provider| provider.id)
        .collect()
}

/// What one pass of the health-check loop does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HealthPass {
    check: bool,
    read_limits: bool,
}

/// With the check off (`interval_secs` 0) nothing runs; otherwise the limits are read on the
/// first pass, with each check, and when a retry is due.
fn health_pass(
    interval_secs: u32,
    since_check: std::time::Duration,
    retry_due: bool,
    first: bool,
) -> HealthPass {
    let on = interval_secs > 0;
    let check = on && since_check.as_secs() >= u64::from(interval_secs);
    HealthPass {
        check,
        read_limits: on && (check || retry_due || first),
    }
}

fn limits_retry(failed: bool, retry: bool, now: std::time::Instant) -> Option<std::time::Instant> {
    (failed && !retry).then(|| now + LIMITS_RETRY)
}

pub struct Workspace {
    pub engine: Arc<Engine>,
    /// When a failed read of the usage limits is tried once more.
    limits_retry_at: Option<std::time::Instant>,
    /// A read of the usage limits is running.
    reading_limits: bool,
    pub model: ReadModel,
    pub settings: AppSettings,
    /// Where `settings` is saved; `None` keeps changes in memory.
    pub settings_path: Option<PathBuf>,
    /// Each enabled provider's models, the default provider first: as known without running
    /// a CLI until the CLIs list them.
    pub models: Vec<ProviderModels>,
    pub open: Option<Entity<ThreadState>>,
    /// The transcripts of the threads open last, the latest first, kept up to date so going
    /// back to one is instant.
    recent: Vec<Entity<ThreadState>>,
    /// The turn the user stopped and when, by thread id, until it ends.
    stopping: HashMap<String, (String, std::time::Instant)>,
    /// The transcripts of the subagents being shown, which take the thread's events too.
    subagents: Vec<WeakEntity<ThreadState>>,
    /// The new thread being written, while no thread is open.
    pub draft: Option<Draft>,
    /// The draft's choices, which its first message creates the thread with.
    pub choices: DraftChoices,
    /// The choices last made, kept for new threads whose settings don't fix them.
    pub last_used: DraftChoices,
    /// Each thread's pull request, as the forge last told.
    pub pull_requests: PullRequests,
    pub logins: Logins,
    /// Threads whose turn ended while another was on screen, until they are opened.
    pub unseen: HashSet<String>,
    /// The enabled providers' CLIs, accounts and plan limits, as the engine last told.
    pub providers: ProvidersStatus,
    /// The servers of each watched thread, as the engine last told.
    pub servers: HashMap<String, Vec<hoshi_core::LocalServer>>,
    /// The step each running git action reached.
    pub git_phases: HashMap<String, hoshi_core::GitActionPhase>,
    /// How many commands each thread's agent runs in the background.
    pub background_tasks: HashMap<String, usize>,
    /// The commands each thread's agent left running, by thread id.
    pub background: HashMap<String, hoshi_core::ThreadBackgroundTasks>,
    /// Delegated tasks whose answer hasn't gone back yet, by thread id.
    pub pending_tasks: HashSet<String>,
    /// Delegated tasks whose card, waiting on the user, is on screen.
    pub tasks_on_screen: HashSet<String>,
    /// The last detail read of each thread's pull request; `None` when that read failed.
    pub pull_request_details: HashMap<String, Option<hoshi_core::PullRequestDetail>>,
    /// The branch each project's root folder is on, by project id.
    pub project_branches: HashMap<String, Option<String>>,
    /// What the last sweep of merged worktrees did.
    pub merged_cleanup: Option<hoshi_engine::MergedCleanup>,
    pub browsers: Entity<crate::browser::Browsers>,
    /// Unsent composer drafts by [`crate::drafts::key`].
    pub drafts: HashMap<String, crate::drafts::Draft>,
    /// The latest worktree setup of each thread since the app started.
    pub worktree_setups: HashMap<String, hoshi_core::WorktreeSetupRun>,
    /// How full each thread's context window was after its last model call.
    pub context_usages: HashMap<String, hoshi_core::ContextUsage>,
    /// The message each thread holds until its conversation is compacted.
    pub held_messages: HashMap<String, hoshi_core::HeldMessage>,
    /// Each thread's queued messages and whether it can be resumed, as the engine last told;
    /// threads with neither are left out.
    pub queues: HashMap<String, hoshi_core::ThreadQueue>,
    drafts_home: Option<PathBuf>,
    drafts_saving: Option<Task<()>>,
}

impl Workspace {
    /// The models `provider` offers, once read.
    pub fn catalog(&self, provider: &str) -> Option<&ProviderModels> {
        self.models
            .iter()
            .find(|catalog| catalog.provider == provider)
    }

    /// `provider`'s plan limits as last read or reported; `None` until read once.
    pub fn usage_limits(&self, provider: &str) -> Option<&hoshi_core::UsageLimits> {
        self.providers.usage_limits.get(provider)
    }

    /// The default provider's CLI as last checked; `None` until checked or while disabled.
    pub fn provider_version(&self) -> Option<&hoshi_core::ProviderVersion> {
        self.providers.versions.get(hoshi_core::DEFAULT_PROVIDER)
    }

    /// The model a choice runs on: the one it names, or its provider's default.
    pub fn chosen_model(&self, provider: &str, slug: Option<&str>) -> Option<&ModelInfo> {
        let catalog = self.catalog(provider)?;
        let slug = slug.or(catalog.default_model.as_deref())?;
        catalog.models.iter().find(|model| model.slug == slug)
    }

    pub fn new(
        engine: Arc<Engine>,
        mut notifications: UnboundedReceiver<Notification>,
        cx: &mut Context<Self>,
    ) -> Self {
        let snapshot = engine.snapshot();
        let model = ReadModel::from_snapshot(&snapshot);
        let pull_requests = snapshot
            .pull_requests
            .into_iter()
            .filter_map(|update| Some((update.thread_id, update.pull_request?)))
            .collect();
        let project_branches = snapshot
            .project_branches
            .into_iter()
            .map(|update| (update.project_id, update.branch))
            .collect();
        let pending_tasks = snapshot.pending_tasks.into_iter().collect();
        let context_usages = snapshot
            .context_usages
            .into_iter()
            .map(|update| (update.thread_id, update.usage))
            .collect();
        let held_messages = snapshot
            .held_messages
            .into_iter()
            .map(|held| (held.thread_id.clone(), held))
            .collect();
        let worktree_setups = engine
            .worktree_setups()
            .into_iter()
            .map(|run| (run.thread_id.clone(), run))
            .collect();
        let browsers = cx.new(|cx| crate::browser::Browsers::new(engine.clone(), cx));
        cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| this.reload_logins(cx)).ok();
        })
        .detach();
        let catalog = engine.clone();
        cx.spawn(async move |this, cx| {
            let models = cx
                .background_spawn(async move { catalog.provider_models() })
                .await;
            this.update(cx, |this, cx| {
                this.models = models;
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let mut checked = std::time::Instant::now();
            let mut first = true;
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(5))
                    .await;
                let Ok((interval, retry_due)) = this.update(cx, |this, _| {
                    (
                        this.settings.providers.health_check_interval_secs,
                        this.limits_retry_at
                            .is_some_and(|at| at <= std::time::Instant::now()),
                    )
                }) else {
                    break;
                };
                let pass = health_pass(interval, checked.elapsed(), retry_due, first);
                first = false;
                if pass.check {
                    checked = std::time::Instant::now();
                    let Ok(engine) = this.update(cx, |this, _| this.engine.clone()) else {
                        break;
                    };
                    let (status, models) = cx
                        .background_spawn(async move {
                            (engine.check_providers(), engine.provider_models())
                        })
                        .await;
                    if this
                        .update(cx, |this, cx| {
                            this.set_providers(status);
                            this.models = models;
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                if pass.read_limits {
                    let retry = retry_due && !pass.check;
                    let Ok(reading) = this.update(cx, |this, cx| this.read_usage_limits(retry, cx))
                    else {
                        break;
                    };
                    reading.await;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while let Some(notification) = notifications.next().await {
                if this
                    .update(cx, |this, cx| this.receive(notification, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let providers = engine.providers_status();
        let models = engine.known_provider_models();
        let merged_cleanup = engine.merged_cleanup();
        Self {
            limits_retry_at: None,
            reading_limits: false,
            engine,
            model,
            settings: AppSettings::default(),
            settings_path: None,
            models,
            open: None,
            recent: Vec::new(),
            stopping: HashMap::new(),
            subagents: Vec::new(),
            draft: None,
            choices: DraftChoices::default(),
            last_used: DraftChoices::default(),
            pull_requests,
            logins: Logins::new(),
            unseen: HashSet::new(),
            providers,
            servers: HashMap::new(),
            git_phases: HashMap::new(),
            background_tasks: HashMap::new(),
            background: HashMap::new(),
            pending_tasks,
            tasks_on_screen: HashSet::new(),
            pull_request_details: HashMap::new(),
            project_branches,
            merged_cleanup,
            browsers,
            drafts: HashMap::new(),
            worktree_setups,
            context_usages,
            held_messages,
            queues: HashMap::new(),
            drafts_home: None,
            drafts_saving: None,
        }
    }

    /// Reads the providers' models again, as when a CLI was just installed.
    pub fn reload_models(&mut self, cx: &mut Context<Self>) {
        let engine = self.engine.clone();
        cx.spawn(async move |this, cx| {
            let models = cx
                .background_spawn(async move { engine.provider_models() })
                .await;
            this.update(cx, |this, cx| {
                this.models = models;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn receive(&mut self, notification: Notification, cx: &mut Context<Self>) {
        match notification {
            Notification::Event(envelope) => {
                let working: Vec<String> = self
                    .model
                    .threads()
                    .filter(|thread| thread.status == ThreadStatus::Working)
                    .map(|thread| thread.id.clone())
                    .collect();
                let open_project = self
                    .open_thread_id(cx)
                    .and_then(|id| self.model.thread(&id))
                    .map(|thread| thread.project_id.clone());
                self.model.apply(&envelope.event);
                crate::fork::follow_created(self, &envelope.event, cx);
                crate::details::track_pending_tasks(&mut self.pending_tasks, &envelope.event);
                if let Event::ThreadDeleted { thread_id } = &envelope.event {
                    self.browsers
                        .update(cx, |browsers, cx| browsers.forget(thread_id, cx));
                    self.recent
                        .retain(|recent| recent.read(cx).thread_id != *thread_id);
                    self.stream_held(cx);
                }
                let model = &self.model;
                self.stopping.retain(|thread_id, (turn_id, _)| {
                    model
                        .thread(thread_id)
                        .and_then(|thread| thread.active_turn_id.as_deref())
                        == Some(turn_id.as_str())
                });
                let open = self.open_thread_id(cx);
                for id in working {
                    let finished = self
                        .model
                        .thread(&id)
                        .is_some_and(|thread| thread.status == ThreadStatus::Idle);
                    if finished && open.as_deref() != Some(id.as_str()) {
                        self.unseen.insert(id);
                    }
                }
                for held in self.open.iter().chain(&self.recent) {
                    held.update(cx, |thread, cx| thread.apply(&envelope.event, cx));
                }
                self.subagents
                    .retain(|subagent| subagent.upgrade().is_some());
                for subagent in self.subagents.clone() {
                    subagent
                        .update(cx, |thread, cx| thread.apply(&envelope.event, cx))
                        .ok();
                }
                if let (Some(open), Some(project_id)) = (&open, open_project)
                    && self.model.thread(open).is_none()
                {
                    self.start_draft(project_id, cx);
                }
                cx.notify();
            }
            Notification::Stream(deltas) => {
                for held in self.open.iter().chain(&self.recent) {
                    held.update(cx, |thread, cx| thread.stream(deltas.clone(), cx));
                }
            }
            Notification::PullRequest(update) => {
                match update.pull_request {
                    Some(pull_request) => {
                        self.pull_requests.insert(update.thread_id, pull_request);
                    }
                    None => {
                        self.pull_requests.remove(&update.thread_id);
                    }
                }
                cx.notify();
            }
            Notification::ProviderVersion(_) | Notification::UsageLimits(_) => {}
            Notification::Providers(status) => {
                let installed = status.accounts.iter().any(|(provider, account)| {
                    *account != hoshi_core::ProviderAccountStatus::NotInstalled
                        && self.providers.accounts.get(provider)
                            == Some(&hoshi_core::ProviderAccountStatus::NotInstalled)
                });
                if self.set_providers(status) && installed {
                    self.reload_models(cx);
                }
                cx.notify();
            }
            Notification::PullRequestDetail(update) => {
                let kept = self
                    .pull_request_details
                    .entry(update.thread_id)
                    .or_default();
                if update.detail.is_some() || kept.is_none() {
                    *kept = update.detail;
                }
                cx.notify();
            }
            Notification::Servers(update) => {
                self.servers.insert(update.thread_id, update.servers);
                cx.notify();
            }
            Notification::GitActionProgress(progress) => {
                self.git_phases.insert(progress.thread_id, progress.phase);
                cx.notify();
            }
            Notification::ProjectBranch(update) => {
                self.project_branches
                    .insert(update.project_id, update.branch);
                cx.notify();
            }
            Notification::MergedCleanup(cleanup) => {
                self.merged_cleanup = Some(cleanup);
                cx.notify();
            }
            Notification::ContextUsage(update) => {
                self.context_usages.insert(update.thread_id, update.usage);
                cx.notify();
            }
            Notification::HeldMessage(update) => {
                match update.message {
                    Some(held) => self.held_messages.insert(update.thread_id, held),
                    None => self.held_messages.remove(&update.thread_id),
                };
                cx.notify();
            }
            Notification::Queue(queue) => {
                self.set_queue(queue);
                cx.notify();
            }
            Notification::BrowserFrame(frame) => {
                self.browsers
                    .update(cx, |browsers, cx| browsers.receive_frame(frame, cx));
            }
            Notification::BrowserPage(page) => {
                self.browsers
                    .update(cx, |browsers, cx| browsers.receive_page(page, cx));
            }
            Notification::BackgroundTasks(update) => {
                if update.tasks.is_empty() {
                    self.background.remove(&update.thread_id);
                } else {
                    self.background
                        .insert(update.thread_id.clone(), update.clone());
                }
                if update.tasks.is_empty() {
                    self.background_tasks.remove(&update.thread_id);
                } else {
                    self.background_tasks
                        .insert(update.thread_id, update.tasks.len());
                }
                cx.notify();
            }
            Notification::WorktreeSetup(run) => {
                self.worktree_setups.insert(run.thread_id.clone(), run);
                cx.notify();
            }
        }
    }

    pub fn open_thread(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        if self
            .open
            .as_ref()
            .is_some_and(|open| open.read(cx).thread_id == thread_id)
        {
            return;
        }
        let (engine, visited) = (self.engine.clone(), thread_id.to_owned());
        cx.background_spawn(async move {
            engine
                .dispatch(Command::ThreadVisit { thread_id: visited })
                .ok();
        })
        .detach();
        self.set_queue(self.engine.thread_queue(thread_id));
        let kept = self
            .recent
            .iter()
            .position(|recent| recent.read(cx).thread_id == thread_id)
            .map(|index| self.recent.remove(index));
        let thread = kept.unwrap_or_else(|| {
            let cwd = self.model.thread_cwd(thread_id).map(str::to_owned);
            let lineage = crate::fork::lineage(&self.model, thread_id);
            cx.new(|cx| ThreadState {
                lineage,
                ..ThreadState::load(self.engine.clone(), thread_id, cwd, cx)
            })
        });
        let left = self.open.replace(thread);
        self.keep_recent(left, cx);
        self.stream_held(cx);
        self.draft = None;
        self.unseen.remove(thread_id);
        cx.notify();
    }

    fn set_queue(&mut self, queue: hoshi_core::ThreadQueue) {
        if queue.messages.is_empty() && !queue.can_resume {
            self.queues.remove(&queue.thread_id);
        } else {
            self.queues.insert(queue.thread_id.clone(), queue);
        }
    }

    /// Loads a subagent's transcript, by its id, and keeps it up to date while it is shown.
    pub fn open_subagent(
        &mut self,
        transcript_id: &str,
        cwd: Option<String>,
        cx: &mut Context<Self>,
    ) -> Entity<ThreadState> {
        let engine = self.engine.clone();
        let thread = cx.new(|cx| ThreadState::load(engine, transcript_id, cwd, cx));
        self.subagents.push(thread.downgrade());
        thread
    }

    /// Opens a new thread in `project_id`, or a new chat with `None`, to be created by its
    /// first message.
    pub fn start_draft(&mut self, project_id: Option<String>, cx: &mut Context<Self>) {
        let left = self.open.take();
        self.keep_recent(left, cx);
        self.stream_held(cx);
        self.choices = self.new_thread_choices(project_id.as_deref());
        self.draft = Some(Draft { project_id });
        cx.notify();
    }

    /// What a new thread in `project_id`, or a chat with `None`, starts with.
    pub fn new_thread_choices(&self, project_id: Option<&str>) -> DraftChoices {
        let place = match project_id {
            None => crate::choices::NewThreadPlace::Chat,
            Some(project_id)
                if self
                    .model
                    .threads()
                    .any(|thread| thread.project_id.as_deref() == Some(project_id)) =>
            {
                crate::choices::NewThreadPlace::Project
            }
            Some(_) => crate::choices::NewThreadPlace::EmptyProject,
        };
        crate::choices::for_new_thread(&self.last_used, &self.settings.new_threads, place)
    }

    /// Creates the draft's thread with `text` as its first message, then opens it. The thread
    /// names itself from the message.
    pub fn send_draft(
        &mut self,
        text: String,
        attachments: Vec<MessageAttachment>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), CommandError>> {
        let Some(draft) = self.draft.clone() else {
            return Task::ready(Ok(()));
        };
        let thread_id = uuid::Uuid::new_v4().to_string();
        let choices = &self.choices;
        let workspace = if draft.project_id.is_some() && choices.worktree {
            ThreadWorkspace::NewWorktree
        } else {
            ThreadWorkspace::Local
        };
        let create = Command::ThreadCreate {
            thread_id: thread_id.clone(),
            project_id: draft.project_id,
            title: String::new(),
            workspace,
            provider: choices.model.provider.clone(),
            model: choices.model.model.clone(),
            effort: choices.model.effort,
            fast_mode: choices.model.fast_mode,
            ultracode: choices.model.ultracode,
            access_mode: choices.access_mode,
            plan_mode: choices.plan_mode,
        };
        let send = Command::MessageSend {
            thread_id: thread_id.clone(),
            message_id: uuid::Uuid::new_v4().to_string(),
            text,
            attachments,
        };
        let engine = self.engine.clone();
        cx.spawn(async move |this, cx| {
            cx.background_spawn(async move {
                engine.dispatch(create)?;
                engine.dispatch(send)
            })
            .await?;
            this.update(cx, |this, cx| this.open_thread(&thread_id, cx))
                .ok();
            Ok(())
        })
    }

    /// Starts a chat and sends it `text`: the `HOSHI_NATIVE_SEND` hook.
    pub fn new_chat_with(&mut self, text: String, cx: &mut Context<Self>) {
        self.start_draft(None, cx);
        self.send_draft(text, Vec::new(), cx).detach_and_log_err(cx);
    }

    /// Stars `model`, or unstars it, and saves the settings.
    pub fn toggle_favorite(&mut self, favorite: FavoriteModel, cx: &mut Context<Self>) {
        let favorites = &mut self.settings.favorite_models;
        match favorites.iter().position(|kept| *kept == favorite) {
            Some(index) => {
                favorites.remove(index);
            }
            None => favorites.push(favorite),
        }
        self.save_settings(cx);
    }

    /// Takes a providers status unless a newer one already arrived; whether it was newer.
    pub fn set_providers(&mut self, status: ProvidersStatus) -> bool {
        if status.revision < self.providers.revision {
            return false;
        }
        self.providers = status;
        true
    }

    /// Shows the usage limits in the thread's card, or hides them, and saves the settings.
    pub fn set_usage_limits_in_details(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.settings.usage_limits_in_details == on {
            return;
        }
        self.settings.usage_limits_in_details = on;
        self.save_settings(cx);
        if on && self.settings.providers.health_check_interval_secs > 0 {
            self.read_usage_limits(false, cx).detach();
        }
    }

    /// Reads the usage limits the thread's card shows again; the engine hands them over as it
    /// reads them. A failed read is tried once more soon, unless it was that try.
    fn read_usage_limits(&mut self, retry: bool, cx: &mut Context<Self>) -> Task<()> {
        let providers = limits_to_refresh(&self.settings, &self.providers.accounts);
        if providers.is_empty()
            || self.reading_limits
            || self.settings.providers.health_check_interval_secs == 0
        {
            return Task::ready(());
        }
        self.limits_retry_at = None;
        self.reading_limits = true;
        let engine = self.engine.clone();
        cx.spawn(async move |this, cx| {
            let failed = cx
                .background_spawn(async move {
                    let reporting = engine.usage_limit_providers();
                    providers
                        .into_iter()
                        .filter(|provider| reporting.iter().any(|id| id == provider))
                        .filter(|provider| engine.usage_limits_of(provider, false).is_none())
                        .count()
                        > 0
                })
                .await;
            this.update(cx, |this, _| {
                this.reading_limits = false;
                this.limits_retry_at = limits_retry(failed, retry, std::time::Instant::now());
            })
            .ok();
        })
    }

    /// Saves the settings and hands the engine what changed.
    fn save_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.settings_path.clone() {
            let snapshot = hoshi_app::settings::Snapshot::of(&self.settings);
            let (engine, previous) = (self.engine.clone(), self.settings.clone());
            cx.background_spawn(async move {
                hoshi_app::apply_settings(&engine, &path, &previous, &snapshot)
            })
            .detach_and_log_err(cx);
        }
        cx.notify();
    }

    /// Reads who the user is signed in as on each forge, again.
    pub fn reload_logins(&self, cx: &mut Context<Self>) {
        let forges = self.engine.clone();
        cx.spawn(async move |this, cx| {
            let accounts = cx
                .background_spawn(async move { forges.forge_accounts() })
                .await;
            this.update(cx, |this, cx| {
                this.logins = accounts
                    .into_iter()
                    .filter_map(|account| Some((account.forge, account.login?)))
                    .collect();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Reads the unsent drafts and the new-thread choices kept in `home`.
    pub fn load_drafts(&mut self, home: &std::path::Path) {
        self.drafts = crate::drafts::load(home);
        self.last_used = crate::choices::load(home);
        self.choices = self.last_used.clone();
        self.drafts_home = Some(home.to_path_buf());
    }

    /// The draft key of what the composer writes to now.
    pub fn draft_key(&self, cx: &App) -> Option<String> {
        match self.open_thread_id(cx) {
            Some(thread_id) => Some(crate::drafts::key(Some(&thread_id), None)),
            None => self
                .draft
                .as_ref()
                .map(|draft| crate::drafts::key(None, draft.project_id.as_deref())),
        }
    }

    /// Keeps `draft` as the draft of `key`, or forgets it when it holds nothing. Lists of drafts
    /// redraw only when one appears or goes.
    pub fn set_draft(&mut self, key: &str, draft: crate::drafts::Draft, cx: &mut Context<Self>) {
        let had = self.drafts.contains_key(key);
        if draft.is_empty() {
            if self.drafts.remove(key).is_none() {
                return;
            }
        } else if self.drafts.get(key) == Some(&draft) {
            return;
        } else {
            self.drafts.insert(key.to_owned(), draft);
        }
        self.save_drafts(cx);
        if had != self.drafts.contains_key(key) {
            cx.notify();
        }
    }

    /// Drops these drafts whole, with the images and files the engine saved for them.
    pub fn forget_drafts(&mut self, keys: &[String], cx: &mut Context<Self>) {
        let before = self.drafts.len();
        let mut saved = Vec::new();
        self.drafts.retain(|key, draft| {
            let forgotten = keys.contains(key);
            if forgotten {
                saved.extend(draft.attachments.iter().map(|kept| kept.attachment.clone()));
            }
            !forgotten
        });
        if !saved.is_empty() {
            let engine = self.engine.clone();
            cx.background_spawn(async move {
                for attachment in &saved {
                    engine.discard_attachment(attachment);
                }
            })
            .detach();
        }
        if self.drafts.len() != before {
            self.save_drafts(cx);
            cx.notify();
        }
    }

    /// Changes the draft's choices, and keeps the change as the last used.
    pub fn set_choices(&mut self, change: impl Fn(&mut DraftChoices), cx: &mut Context<Self>) {
        change(&mut self.choices);
        let before = self.last_used.clone();
        change(&mut self.last_used);
        if self.last_used == before {
            return;
        }
        let Some(home) = self.drafts_home.clone() else {
            return;
        };
        let choices = self.last_used.clone();
        cx.background_spawn(async move { crate::choices::save(&home, &choices) })
            .detach_and_log_err(cx);
    }

    fn save_drafts(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.drafts_home.clone() else {
            return;
        };
        let drafts = self.drafts.clone();
        self.drafts_saving = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            cx.background_spawn(async move { crate::drafts::save(&home, &drafts) })
                .await
                .ok();
        }));
    }

    /// Keeps a transcript that left the screen among the recent ones, dropping the oldest.
    fn keep_recent(&mut self, left: Option<Entity<ThreadState>>, cx: &App) {
        let left = left.filter(|left| self.model.thread(&left.read(cx).thread_id).is_some());
        if let Some(left) = left {
            self.recent.insert(0, left);
            self.recent.truncate(KEPT_TRANSCRIPTS - 1);
        }
    }

    /// Streams the text of every transcript held, the open one and the recent ones.
    fn stream_held(&self, cx: &App) {
        let ids = self
            .open
            .iter()
            .chain(&self.recent)
            .map(|held| held.read(cx).thread_id.clone())
            .collect();
        self.engine.stream_threads(ids);
    }

    /// Whether `thread` is the open transcript or one of the recent ones, kept up to date.
    pub fn holds(&self, thread: &Entity<ThreadState>) -> bool {
        self.open
            .iter()
            .chain(&self.recent)
            .any(|held| held == thread)
    }

    /// When the user stopped the thread's running turn, which has yet to end.
    pub fn stopping_since(&self, thread_id: &str) -> Option<std::time::Instant> {
        let (turn_id, asked) = self.stopping.get(thread_id)?;
        let running = self
            .model
            .thread(thread_id)
            .and_then(|thread| thread.active_turn_id.as_deref());
        (running == Some(turn_id.as_str())).then_some(*asked)
    }

    /// The user pressed Stop on the thread's running turn.
    pub fn mark_stopping(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        let turn_id = self
            .model
            .thread(thread_id)
            .and_then(|thread| thread.active_turn_id.clone());
        if let Some(turn_id) = turn_id {
            self.stopping
                .insert(thread_id.to_owned(), (turn_id, std::time::Instant::now()));
            cx.notify();
        }
    }

    /// The stop failed, so the turn runs on as before.
    pub fn unmark_stopping(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        if self.stopping.remove(thread_id).is_some() {
            cx.notify();
        }
    }

    pub fn open_thread_id(&self, cx: &App) -> Option<String> {
        self.open
            .as_ref()
            .map(|open| open.read(cx).thread_id.clone())
    }

    pub fn set_tasks_on_screen(&mut self, tasks: HashSet<String>, cx: &mut Context<Self>) {
        if self.tasks_on_screen != tasks {
            self.tasks_on_screen = tasks;
            cx.notify();
        }
    }
}

/// The transcript of the thread on screen.
pub struct ThreadState {
    pub thread_id: String,
    pub items: Vec<ThreadItem>,
    pub has_older: bool,
    pub loading: bool,
    pub loading_older: bool,
    /// Text still being written, in the order it started.
    pub streaming: Vec<Streaming>,
    /// Where the thread's agent runs, which paths in its calls are shown relative to.
    pub cwd: Option<String>,
    /// Items whole, read when a row that left something out opens.
    pub whole: HashMap<String, ItemContent>,
    reading: HashSet<String>,
    /// `None` in tests that build a transcript by hand.
    pub engine: Option<Arc<Engine>>,
    /// How many of the first items the thread inherited from the threads it was forked from.
    pub inherited: usize,
    /// Each fork up the chain with the thread it came from, this thread first: their
    /// inherited history shows above its own items.
    pub lineage: Vec<(String, String)>,
    /// How many items each fork of `lineage` inherited, as far as read.
    segments: Vec<usize>,
    /// The fork of `lineage` whose inherited history reads next, and the item it reads before.
    next_history: usize,
    history_before: Option<String>,
}

pub struct Streaming {
    pub item_id: String,
    pub kind: StreamKind,
    pub text: String,
}

impl ThreadState {
    pub fn new(thread_id: &str, items: Vec<ThreadItem>) -> Self {
        Self {
            thread_id: thread_id.to_owned(),
            items,
            has_older: false,
            loading: false,
            loading_older: false,
            streaming: Vec::new(),
            cwd: None,
            whole: HashMap::new(),
            reading: HashSet::new(),
            engine: None,
            inherited: 0,
            lineage: Vec::new(),
            segments: Vec::new(),
            next_history: 0,
            history_before: None,
        }
    }

    fn load(
        engine: Arc<Engine>,
        thread_id: &str,
        cwd: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = thread_id.to_owned();
        let reader = engine.clone();
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_spawn(async move { reader.transcript(&id, None) })
                .await;
            this.update(cx, |thread, cx| {
                thread.loading = false;
                match page {
                    Ok(page) => {
                        thread.items = page.items;
                        thread.has_older = page.has_older;
                    }
                    Err(error) => eprintln!("hoshi: transcript failed: {error}"),
                }
                thread.load_history(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            loading: true,
            cwd,
            engine: Some(engine),
            ..Self::new(thread_id, Vec::new())
        }
    }

    /// The item as shown: whole once read, else as its page left it.
    pub fn content<'a>(&'a self, item: &'a ThreadItem) -> &'a ItemContent {
        self.whole.get(&item.id).unwrap_or(&item.content)
    }

    /// Reads the item whole in the background when its page left part of it out.
    pub fn read_whole(&mut self, item_id: &str, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let Some(at) = self.items.iter().position(|item| item.id == item_id) else {
            return;
        };
        let incomplete = left_out(&self.items[at].content);
        if !incomplete || self.whole.contains_key(item_id) || !self.reading.insert(item_id.into()) {
            return;
        }
        let thread_id = if at < self.inherited {
            self.items[at].thread_id.clone()
        } else {
            self.thread_id.clone()
        };
        let id = item_id.to_owned();
        cx.spawn(async move |this, cx| {
            let read = id.clone();
            let item = cx
                .background_spawn(async move { engine.thread_item(&thread_id, &read) })
                .await;
            this.update(cx, |thread, cx| {
                thread.reading.remove(&id);
                match item {
                    Ok(Some(item)) => {
                        thread.whole.insert(id, item.content);
                        cx.notify();
                    }
                    Ok(None) => {}
                    Err(error) => eprintln!("hoshi: could not read a tool call: {error}"),
                }
            })
            .ok();
        })
        .detach();
    }

    /// What a running call printed so far.
    pub fn live_output(&self, item_id: &str) -> Option<&str> {
        self.streaming
            .iter()
            .find(|streaming| {
                streaming.kind == StreamKind::ToolOutput && streaming.item_id == item_id
            })
            .map(|streaming| streaming.text.as_str())
    }

    /// Puts the page before the first item shown above it.
    pub fn load_older(&mut self, cx: &mut Context<Self>) {
        if !self.has_older {
            self.load_history(cx);
            return;
        }
        let (Some(engine), Some(first)) = (self.engine.clone(), self.items.first()) else {
            return;
        };
        if self.loading || self.loading_older {
            return;
        }
        self.loading_older = true;
        let (id, before) = (self.thread_id.clone(), first.id.clone());
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_spawn(async move { engine.transcript(&id, Some(&before)) })
                .await;
            this.update(cx, |thread, cx| {
                thread.loading_older = false;
                match page {
                    Ok(page) => {
                        thread.has_older = page.has_older;
                        let mut items = page.items;
                        items.append(&mut thread.items);
                        thread.items = items;
                    }
                    Err(error) => eprintln!("hoshi: older transcript failed: {error}"),
                }
                thread.load_history(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Sets the forks the thread's inherited history comes from, once it is known to be one.
    pub fn set_lineage(&mut self, lineage: Vec<(String, String)>, cx: &mut Context<Self>) {
        if self.lineage == lineage || self.inherited > 0 {
            return;
        }
        self.lineage = lineage;
        self.load_history(cx);
        cx.notify();
    }

    /// Puts the next page of inherited history above, once the thread's own items are all read.
    fn load_history(&mut self, cx: &mut Context<Self>) {
        let Some((fork, _)) = self.lineage.get(self.next_history) else {
            return;
        };
        let Some(engine) = self.engine.clone() else {
            return;
        };
        if self.has_older || self.loading || self.loading_older {
            return;
        }
        self.loading_older = true;
        let (fork, before) = (fork.clone(), self.history_before.clone());
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_spawn(async move { engine.fork_history(&fork, before.as_deref()) })
                .await;
            this.update(cx, |thread, cx| {
                thread.loading_older = false;
                match page {
                    Ok(page) => thread.add_history(page, cx),
                    Err(error) => {
                        eprintln!("hoshi: inherited history failed: {error}");
                        thread.next_history = thread.lineage.len();
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A page of the history the fork `lineage[next_history]` inherited, put above. The next
    /// fork up the chain starts right away once this one's is whole.
    pub(crate) fn add_history(&mut self, page: TranscriptPage, cx: &mut Context<Self>) {
        let at = self.next_history;
        if self.segments.len() <= at {
            self.segments.resize(at + 1, 0);
        }
        self.segments[at] += page.items.len();
        self.inherited += page.items.len();
        let more = page.has_older && !page.items.is_empty();
        self.history_before = page
            .items
            .first()
            .filter(|_| more)
            .map(|item| item.id.clone());
        if !more {
            self.next_history += 1;
        }
        let mut items = page.items;
        items.append(&mut self.items);
        self.items = items;
        if !more {
            self.load_history(cx);
        }
    }

    /// Where a "Forked from" divider goes, as the index of the item it goes before, and the
    /// thread it names: above the thread's own items, and above each fork's inherited history
    /// once read whole. None until the thread's own items are all read.
    pub fn fork_boundaries(&self) -> Vec<(usize, &str)> {
        if self.has_older || self.loading {
            return Vec::new();
        }
        let mut at = self.inherited;
        let mut boundaries = Vec::new();
        for (index, (_, source)) in self.lineage.iter().enumerate() {
            if index > 0 {
                if self.next_history < index {
                    break;
                }
                at -= self.segments.get(index - 1).copied().unwrap_or(0);
            }
            boundaries.push((at, source.as_str()));
        }
        boundaries
    }

    fn apply(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            Event::ItemAdded { item } if item.thread_id == self.thread_id => {
                self.streaming
                    .retain(|streaming| streaming.item_id != item.id);
                match self.items.iter_mut().find(|known| known.id == item.id) {
                    Some(known) => *known = item.clone(),
                    None => self.items.push(item.clone()),
                }
            }
            Event::ToolCallCompleted {
                thread_id,
                item_id,
                output: done,
                is_error: failed,
                detail: done_detail,
            } if *thread_id == self.thread_id => {
                self.streaming
                    .retain(|streaming| &streaming.item_id != item_id);
                if let Some(ItemContent::ToolCall {
                    output,
                    is_error,
                    detail,
                    ..
                }) = self.call_mut(item_id)
                {
                    *output = Some(done.clone());
                    *is_error = *failed;
                    if done_detail.is_some() {
                        detail.clone_from(done_detail);
                    }
                }
            }
            Event::ToolCallDetailSet {
                thread_id,
                item_id,
                detail: set,
            } if *thread_id == self.thread_id => {
                if let Some(ItemContent::ToolCall { detail, .. }) = self.call_mut(item_id) {
                    *detail = Some(set.clone());
                }
            }
            Event::ToolCallInterrupted { thread_id, item_id } if *thread_id == self.thread_id => {
                self.streaming
                    .retain(|streaming| &streaming.item_id != item_id);
                if let Some(ItemContent::ToolCall { interrupted, .. }) = self.call_mut(item_id) {
                    *interrupted = true;
                }
            }
            Event::MessagesDropped {
                thread_id,
                message_ids,
            } if *thread_id == self.thread_id => {
                for item in &mut self.items {
                    if let ItemContent::UserMessage { dropped, .. } = &mut item.content
                        && message_ids.contains(&item.id)
                    {
                        *dropped = true;
                    }
                }
            }
            _ => return,
        }
        cx.notify();
    }

    fn call_mut(&mut self, item_id: &str) -> Option<&mut ItemContent> {
        self.whole.remove(item_id);
        self.items
            .iter_mut()
            .find(|item| item.id == item_id)
            .map(|item| &mut item.content)
    }

    fn stream(&mut self, deltas: Vec<StreamDelta>, cx: &mut Context<Self>) {
        let mut changed = false;
        for delta in deltas {
            if delta.thread_id != self.thread_id {
                continue;
            }
            changed = true;
            let index = match self
                .streaming
                .iter()
                .position(|streaming| streaming.item_id == delta.item_id)
            {
                Some(index) => index,
                None => {
                    self.streaming.push(Streaming {
                        item_id: delta.item_id.clone(),
                        kind: delta.kind,
                        text: String::new(),
                    });
                    self.streaming.len() - 1
                }
            };
            let streaming = &mut self.streaming[index];
            if delta.kind == StreamKind::ToolOutput {
                streaming.text = delta.delta;
            } else {
                streaming.text.push_str(&delta.delta);
            }
        }
        if changed {
            cx.notify();
        }
    }
}

/// Whether a page of the transcript left part of the item out: long output or a diff.
pub fn left_out(content: &ItemContent) -> bool {
    match content {
        ItemContent::ToolCall {
            output_omitted,
            detail,
            ..
        } => *output_omitted || matches!(detail, Some(ToolDetail::FileChange { diff: None, .. })),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::ThreadState;
    use gpui_kit::{AppContext as _, TestAppContext};
    use hoshi_core::{Event, ItemContent, ThreadItem};

    fn message(id: &str) -> ThreadItem {
        ThreadItem {
            id: id.into(),
            thread_id: "t".into(),
            created_at: "2026-11-15T10:00:00Z".into(),
            content: ItemContent::UserMessage {
                text: id.into(),
                attachments: Vec::new(),
                steer: true,
                task_result: None,
                pull_request_update: None,
                resumed: None,
                rewind: None,
                dropped: false,
                model_switch: None,
            },
        }
    }

    #[gpui_kit::test]
    fn messages_the_turn_dropped_are_marked_in_place(cx: &mut TestAppContext) {
        let thread =
            cx.update(|cx| cx.new(|_| ThreadState::new("t", vec![message("m1"), message("m2")])));
        thread.update(cx, |thread, cx| {
            thread.apply(
                &Event::MessagesDropped {
                    thread_id: "other".into(),
                    message_ids: vec!["m1".into()],
                },
                cx,
            );
            thread.apply(
                &Event::MessagesDropped {
                    thread_id: "t".into(),
                    message_ids: vec!["m2".into()],
                },
                cx,
            );
        });
        let dropped: Vec<bool> = thread.read_with(cx, |thread, _| {
            thread
                .items
                .iter()
                .map(|item| matches!(item.content, ItemContent::UserMessage { dropped: true, .. }))
                .collect()
        });
        assert_eq!(dropped, [false, true]);
    }
}

#[cfg(test)]
mod providers_tests {
    use super::{Workspace, channel};
    use gpui_kit::{AppContext as _, TestAppContext};
    use std::sync::Arc;

    #[gpui_kit::test]
    fn a_providers_status_taken_before_the_one_held_is_ignored(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let home =
            std::env::temp_dir().join(format!("hoshi-native-providers-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let (notify, notifications) = channel();
        let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
        let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
        workspace.update(cx, |workspace, _| {
            let older = workspace.engine.providers_status();
            let mut newer = workspace.engine.providers_status();
            newer.checked_at = Some("2026-11-17T12:00:00Z".into());
            assert!(workspace.set_providers(newer.clone()));
            assert!(!workspace.set_providers(older));
            assert_eq!(workspace.providers, newer);
        });
    }
}

#[cfg(test)]
mod queue_tests {
    use super::{Workspace, channel};
    use gpui_kit::{AppContext as _, TestAppContext};
    use hoshi_core::{MessageSender, QueueEntry, ThreadQueue};
    use hoshi_engine::Notification;
    use std::sync::Arc;

    #[gpui_kit::test]
    fn the_engine_s_queue_news_is_kept_until_the_queue_empties(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let home =
            std::env::temp_dir().join(format!("hoshi-native-queue-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let (notify, notifications) = channel();
        let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
        let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
        let queue = |messages: Vec<QueueEntry>, can_resume| ThreadQueue {
            thread_id: "t".into(),
            messages,
            can_resume,
        };
        let entry = QueueEntry {
            message_id: "m1".into(),
            text: "then the docs".into(),
            attachments: Vec::new(),
            held: true,
            sender: MessageSender::User,
        };
        workspace.update(cx, |workspace, cx| {
            workspace.receive(Notification::Queue(queue(vec![entry.clone()], false)), cx)
        });
        let kept = workspace.read_with(cx, |workspace, _| workspace.queues.get("t").cloned());
        assert_eq!(kept, Some(queue(vec![entry], false)));

        workspace.update(cx, |workspace, cx| {
            workspace.receive(Notification::Queue(queue(Vec::new(), true)), cx)
        });
        assert!(workspace.read_with(cx, |workspace, _| workspace.queues.contains_key("t")));
        workspace.update(cx, |workspace, cx| {
            workspace.receive(Notification::Queue(queue(Vec::new(), false)), cx)
        });
        assert!(!workspace.read_with(cx, |workspace, _| workspace.queues.contains_key("t")));
    }
}

#[cfg(test)]
mod live_tests {
    use super::{Workspace, channel};
    use gpui_kit::{AppContext as _, TestAppContext};
    use hoshi_core::ItemContent;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// Spends a few tokens: real Claude Code, in a chat on a scratch home.
    #[gpui_kit::test]
    #[ignore]
    fn a_real_turn_streams_its_text_before_the_answer_lands(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let home = std::env::temp_dir().join(format!("hoshi-native-live-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let (notify, notifications) = channel();
        let engine = Arc::new(hoshi_app::open_engine(&home, notify).unwrap());
        let workspace =
            cx.update(|cx| cx.new(|cx| Workspace::new(engine.clone(), notifications, cx)));
        workspace.update(cx, |workspace, cx| {
            workspace.new_chat_with(
                "Write three short sentences about the sea, in Spanish. No tools.".into(),
                cx,
            )
        });

        let mut streamed = 0usize;
        let deadline = Instant::now() + Duration::from_secs(180);
        let answer = loop {
            cx.run_until_parked();
            let (streaming, answer) = workspace.read_with(cx, |workspace, cx| {
                let Some(thread) = workspace.open.as_ref().map(|open| open.read(cx)) else {
                    return (0, None);
                };
                let streaming = thread
                    .streaming
                    .iter()
                    .map(|streaming| streaming.text.len())
                    .max()
                    .unwrap_or(0);
                let answer = thread.items.iter().find_map(|item| match &item.content {
                    ItemContent::AssistantMessage { text } => Some(text.clone()),
                    _ => None,
                });
                (streaming, answer)
            });
            streamed = streamed.max(streaming);
            if let Some(answer) = answer {
                break answer;
            }
            assert!(Instant::now() < deadline, "no answer in time");
            std::thread::sleep(Duration::from_millis(20));
        };
        engine.close_lingering_agents();
        eprintln!("streamed up to {streamed} bytes; answer: {answer}");
        assert!(streamed > 0, "the answer landed without streaming");
        assert!(!answer.trim().is_empty());
    }
}

#[cfg(test)]
mod recent_tests {
    use super::{KEPT_TRANSCRIPTS, Workspace, channel};
    use gpui_kit::{AppContext as _, Entity, TestAppContext};
    use hoshi_core::{AccessMode, Command, ThreadWorkspace};
    use std::sync::Arc;

    fn workspace(cx: &mut TestAppContext, threads: usize) -> Entity<Workspace> {
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let home =
            std::env::temp_dir().join(format!("hoshi-native-recent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(home.join("project")).unwrap();
        std::fs::create_dir_all(home.join("home")).unwrap();
        let (notify, notifications) = channel();
        let engine = Arc::new(hoshi_engine::Engine::open(&home.join("home"), notify).unwrap());
        engine
            .dispatch(Command::ProjectCreate {
                project_id: "p".into(),
                title: "project".into(),
                workspace_root: home.join("project").to_string_lossy().into_owned(),
            })
            .unwrap();
        for index in 0..threads {
            engine
                .dispatch(Command::ThreadCreate {
                    thread_id: format!("t{index}"),
                    project_id: Some("p".into()),
                    title: format!("thread {index}"),
                    workspace: ThreadWorkspace::Local,
                    provider: hoshi_core::default_provider(),
                    model: None,
                    effort: None,
                    fast_mode: false,
                    ultracode: false,
                    access_mode: AccessMode::default(),
                    plan_mode: false,
                })
                .unwrap();
        }
        cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)))
    }

    #[gpui_kit::test]
    fn a_thread_open_just_before_comes_back_with_the_same_transcript(cx: &mut TestAppContext) {
        let workspace = workspace(cx, KEPT_TRANSCRIPTS + 1);
        let open = |workspace: &Entity<Workspace>, id: &str, cx: &mut TestAppContext| {
            workspace.update(cx, |workspace, cx| workspace.open_thread(id, cx));
            workspace.read_with(cx, |workspace, _| workspace.open.clone().unwrap())
        };
        let first = open(&workspace, "t0", cx);
        open(&workspace, "t1", cx);
        assert_eq!(open(&workspace, "t0", cx), first, "t0 was kept");

        for index in 1..=KEPT_TRANSCRIPTS {
            open(&workspace, &format!("t{index}"), cx);
        }
        assert!(
            !workspace.read_with(cx, |workspace, _| workspace.holds(&first)),
            "the oldest of more than {KEPT_TRANSCRIPTS} transcripts goes"
        );
        assert_ne!(open(&workspace, "t0", cx), first);
    }
}

#[cfg(test)]
mod limits_tests {
    use super::{HealthPass, LIMITS_RETRY, health_pass, limits_retry, limits_to_refresh};
    use hoshi_core::{AppSettings, ProviderAccountStatus};
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    #[test]
    fn the_limits_are_read_at_start_with_each_check_and_on_a_due_retry_while_checks_run() {
        let pass = |check, read_limits| HealthPass { check, read_limits };
        let early = Duration::from_secs(10);
        let late = Duration::from_secs(300);
        assert_eq!(health_pass(300, early, false, true), pass(false, true));
        assert_eq!(health_pass(300, early, false, false), pass(false, false));
        assert_eq!(health_pass(300, early, true, false), pass(false, true));
        assert_eq!(health_pass(300, late, false, false), pass(true, true));
        assert_eq!(health_pass(0, late, true, true), pass(false, false));
    }

    #[test]
    fn a_failed_read_is_tried_once_more_then_waits_for_the_next_check() {
        let now = Instant::now();
        assert_eq!(limits_retry(false, false, now), None);
        assert_eq!(limits_retry(true, false, now), Some(now + LIMITS_RETRY));
        assert_eq!(limits_retry(true, true, now), None);
    }

    #[test]
    fn the_health_check_reads_the_limits_the_card_can_show_and_only_while_it_shows_them() {
        let mut settings = AppSettings::default();
        settings.providers.codex.added = true;
        settings.providers.codex.enabled = true;
        let mut accounts = HashMap::from([
            ("claudeCode".to_owned(), ProviderAccountStatus::Ready),
            ("codex".to_owned(), ProviderAccountStatus::Ready),
        ]);
        assert!(limits_to_refresh(&settings, &accounts).is_empty());

        settings.usage_limits_in_details = true;
        assert!(
            limits_to_refresh(&settings, &HashMap::new()).is_empty(),
            "not before the accounts are known"
        );
        let mut read = limits_to_refresh(&settings, &accounts);
        read.sort_unstable();
        assert_eq!(read, ["claudeCode", "codex"]);

        accounts.insert("codex".to_owned(), ProviderAccountStatus::SignedOut);
        assert_eq!(limits_to_refresh(&settings, &accounts), ["claudeCode"]);
        accounts.insert("codex".to_owned(), ProviderAccountStatus::Unknown);
        assert_eq!(limits_to_refresh(&settings, &accounts), ["claudeCode"]);
        settings.providers.claude_code.enabled = false;
        assert!(limits_to_refresh(&settings, &accounts).is_empty());
    }
}
