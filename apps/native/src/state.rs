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
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

mod thread_state;

pub use thread_state::*;

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
    /// The id its first message creates the thread with.
    pub thread_id: String,
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
    /// The thread on screen that stays out of Done until the user leaves it: it had a turn
    /// they hadn't seen, or a turn ended or its pull request finished while it was open.
    held: Option<String>,
    /// The turn end each visit on its way to the engine marks seen, by thread id.
    visiting: HashMap<String, Option<String>>,
    /// What [`Self::unseen`] last found, until the threads or the visits change.
    unseen: RefCell<Option<Rc<HashSet<String>>>>,
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
    /// The thread id each new thread's place will create, by [`crate::drafts::key`].
    draft_threads: HashMap<String, String>,
    /// Draft threads sent but not created in the model yet, with their project.
    sending_drafts: HashMap<String, Option<String>>,
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
    /// The last save of `last_used`; each save waits for the one before it.
    choices_saving: Option<futures::future::Shared<Task<()>>>,
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
            held: None,
            visiting: HashMap::new(),
            unseen: RefCell::default(),
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
            draft_threads: HashMap::new(),
            sending_drafts: HashMap::new(),
            worktree_setups,
            context_usages,
            held_messages,
            queues: HashMap::new(),
            drafts_home: None,
            drafts_saving: None,
            choices_saving: None,
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
                let open_project = self
                    .open_thread_id(cx)
                    .and_then(|id| self.model.thread(&id))
                    .map(|thread| thread.project_id.clone());
                self.model.apply(&envelope.event);
                self.unseen.take();
                let model = &self.model;
                self.visiting.retain(|thread_id, ended| {
                    model.thread(thread_id).is_some_and(|thread| {
                        thread.has_unseen_turn() && thread.last_turn_ended_at == *ended
                    })
                });
                if let Event::ThreadCreated { thread } = &envelope.event {
                    self.sending_drafts.remove(&thread.id);
                }
                if let Event::ProjectRemoved { .. } = &envelope.event {
                    let model = &self.model;
                    self.draft_threads.retain(|key, _| {
                        crate::drafts::new_thread_project(key)
                            .flatten()
                            .is_none_or(|project_id| model.project(project_id).is_some())
                    });
                }
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
                if let Event::TurnCompleted { thread_id, .. } = &envelope.event
                    && open.as_deref() == Some(thread_id.as_str())
                {
                    self.held = Some(thread_id.clone());
                    self.visit(thread_id, cx);
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
                let finished = |pull_request: Option<&hoshi_core::PullRequest>| {
                    pull_request.is_some_and(|pull_request| pull_request.state.is_finished())
                };
                if finished(update.pull_request.as_ref())
                    && !finished(self.pull_requests.get(&update.thread_id))
                    && self.open_thread_id(cx).as_deref() == Some(update.thread_id.as_str())
                {
                    self.held = Some(update.thread_id.clone());
                }
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
        self.held = self
            .model
            .thread(thread_id)
            .filter(|thread| thread.has_unseen_turn())
            .map(|thread| thread.id.clone());
        self.visit(thread_id, cx);
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
        cx.notify();
    }

    fn visit(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        if let Some(thread) = self
            .model
            .thread(thread_id)
            .filter(|thread| thread.has_unseen_turn())
        {
            self.visiting
                .insert(thread.id.clone(), thread.last_turn_ended_at.clone());
            self.unseen.take();
        }
        let (engine, thread_id) = (self.engine.clone(), thread_id.to_owned());
        cx.background_spawn(async move {
            engine.dispatch(Command::ThreadVisit { thread_id }).ok();
        })
        .detach();
    }

    /// The listed threads whose latest turn ended after the user last opened them.
    pub fn unseen(&self) -> Rc<HashSet<String>> {
        if let Some(unseen) = &*self.unseen.borrow() {
            return unseen.clone();
        }
        let model = &self.model;
        let unseen: Rc<HashSet<String>> = Rc::new(
            model
                .threads()
                .filter(|thread| {
                    !thread.archived
                        && thread
                            .parent_thread_id
                            .as_deref()
                            .is_none_or(|parent| model.thread(parent).is_none())
                        && thread.has_unseen_turn()
                        && self.visiting.get(&thread.id) != Some(&thread.last_turn_ended_at)
                })
                .map(|thread| thread.id.clone())
                .collect(),
        );
        *self.unseen.borrow_mut() = Some(unseen.clone());
        unseen
    }

    /// The open thread, while it stays out of Done until the user leaves it.
    pub fn held(&self, cx: &App) -> Option<String> {
        self.held
            .clone()
            .filter(|held| self.open_thread_id(cx).as_deref() == Some(held.as_str()))
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
        self.held = None;
        let left = self.open.take();
        self.keep_recent(left, cx);
        self.stream_held(cx);
        self.choices = self.new_thread_choices(project_id.as_deref());
        let thread_id = self
            .draft_threads
            .entry(crate::drafts::key(None, project_id.as_deref()))
            .or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        self.draft = Some(Draft {
            project_id,
            thread_id,
        });
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
        let thread_id = draft.thread_id.clone();
        if self.sending_drafts.contains_key(&thread_id) || self.model.thread(&thread_id).is_some() {
            return Task::ready(Err(CommandError::ThreadBusy { thread_id }));
        }
        let place = crate::drafts::key(None, draft.project_id.as_deref());
        self.draft_threads.remove(&place);
        self.sending_drafts
            .insert(thread_id.clone(), draft.project_id.clone());
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
            let created = {
                let engine = engine.clone();
                cx.background_spawn(async move { engine.dispatch(create) })
                    .await
            };
            if let Err(error) = created {
                this.update(cx, |this, cx| {
                    let project_id = this.sending_drafts.remove(&thread_id).flatten();
                    let place_exists = project_id
                        .as_deref()
                        .is_none_or(|project_id| this.model.project(project_id).is_some());
                    if place_exists && this.model.thread(&thread_id).is_none() {
                        this.draft_threads.entry(place).or_insert(thread_id);
                    }
                    cx.notify();
                })
                .ok();
                return Err(error);
            }
            let sent = cx
                .background_spawn(async move { engine.dispatch(send) })
                .await;
            this.update(cx, |this, cx| this.open_thread(&thread_id, cx))
                .ok();
            sent
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

    /// Shows the thread's context window in its card or under the composer, and saves it.
    pub fn set_context_meter(
        &mut self,
        place: hoshi_core::ContextMeterPlace,
        cx: &mut Context<Self>,
    ) {
        if self.settings.context_meter == place {
            return;
        }
        self.settings.context_meter = place;
        self.save_settings(cx);
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

    /// Shows the app in `language` from now on, and saves it.
    pub fn set_language(&mut self, language: hoshi_core::Language, cx: &mut Context<Self>) {
        if self.settings.general.language == language {
            return;
        }
        self.settings.general.language = language;
        crate::locale::show(language, cx);
        self.save_settings(cx);
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
        let mut threads_changed = false;
        for key in keys {
            if self.draft_threads.remove(key).is_none() {
                continue;
            }
            threads_changed = true;
            if let Some(draft) = self.draft.as_mut()
                && crate::drafts::key(None, draft.project_id.as_deref()) == *key
            {
                draft.thread_id = uuid::Uuid::new_v4().to_string();
                self.draft_threads
                    .insert(key.clone(), draft.thread_id.clone());
            }
        }
        if threads_changed {
            cx.notify();
        }
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
        let previous = self.choices_saving.take();
        let saving = futures::FutureExt::shared(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            if let Err(error) = crate::choices::save(&home, &choices) {
                eprintln!("hoshi: could not save the new thread choices: {error}");
            }
        }));
        cx.background_spawn(saving.clone()).detach();
        self.choices_saving = Some(saving);
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

    /// Whether the thread's turn ended but its agent awaits commands it left running.
    pub fn awaits_background(&self, thread_id: &str) -> bool {
        self.background
            .get(thread_id)
            .is_some_and(|background| background.waiting)
    }

    /// The threads as the sidebar sorts them: one whose agent awaits its background commands
    /// still works.
    pub fn listed_threads(&self) -> Vec<hoshi_core::Thread> {
        self.model
            .threads()
            .map(|thread| {
                let mut thread = thread.clone();
                if thread.status == ThreadStatus::Idle && self.awaits_background(&thread.id) {
                    thread.status = ThreadStatus::Working;
                }
                thread
            })
            .collect()
    }

    pub fn open_thread_id(&self, cx: &App) -> Option<String> {
        self.open
            .as_ref()
            .map(|open| open.read(cx).thread_id.clone())
    }

    /// The open thread, or the one the draft on screen will create.
    pub fn shown_thread_id(&self, cx: &App) -> Option<String> {
        self.open_thread_id(cx)
            .or_else(|| self.draft.as_ref().map(|draft| draft.thread_id.clone()))
    }

    /// The project a draft thread is for, `Some(None)` for a chat; `None` for any other id.
    pub fn draft_project(&self, thread_id: &str) -> Option<Option<String>> {
        if self.model.thread(thread_id).is_some() {
            return None;
        }
        if let Some(project_id) = self.sending_drafts.get(thread_id) {
            return Some(project_id.clone());
        }
        self.draft_threads
            .iter()
            .find(|(_, id)| *id == thread_id)
            .and_then(|(key, _)| crate::drafts::new_thread_project(key))
            .map(|project_id| project_id.map(str::to_owned))
    }

    /// Whether `thread_id` is a thread, or one a draft will create.
    pub fn thread_or_draft(&self, thread_id: &str) -> bool {
        self.model.thread(thread_id).is_some()
            || self.sending_drafts.contains_key(thread_id)
            || self.draft_threads.values().any(|id| id == thread_id)
    }

    pub fn set_tasks_on_screen(&mut self, tasks: HashSet<String>, cx: &mut Context<Self>) {
        if self.tasks_on_screen != tasks {
            self.tasks_on_screen = tasks;
            cx.notify();
        }
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
    fn a_draft_sends_once_and_a_failed_send_keeps_the_draft_started_since(cx: &mut TestAppContext) {
        use futures::FutureExt as _;
        let workspace = workspace(cx, 0);
        let start = |workspace: &Entity<Workspace>, cx: &mut TestAppContext| {
            workspace.update(cx, |workspace, cx| {
                workspace.start_draft(Some("gone".into()), cx);
                workspace.shown_thread_id(cx).unwrap()
            })
        };
        let first = start(&workspace, cx);
        let (sending, again) = workspace.update(cx, |workspace, cx| {
            let sending = workspace.send_draft("hello".into(), Vec::new(), cx);
            let again = workspace.send_draft("hello".into(), Vec::new(), cx);
            (sending, again)
        });
        assert!(matches!(
            again.now_or_never(),
            Some(Err(hoshi_core::CommandError::ThreadBusy { .. }))
        ));
        let started_since = start(&workspace, cx);
        assert_ne!(started_since, first);

        let result = std::rc::Rc::new(std::cell::RefCell::new(None));
        cx.update(|cx| {
            let result = result.clone();
            cx.spawn(async move |_| *result.borrow_mut() = Some(sending.await))
                .detach()
        });
        cx.run_until_parked();
        assert!(matches!(
            result.borrow_mut().take(),
            Some(Err(hoshi_core::CommandError::ProjectNotFound { .. }))
        ));
        assert_eq!(start(&workspace, cx), started_since);
        assert!(workspace.read_with(cx, |workspace, _| !workspace.thread_or_draft(&first)));

        workspace
            .update(cx, |workspace, cx| {
                workspace.send_draft("hello".into(), Vec::new(), cx)
            })
            .detach();
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |workspace, _| {
            !workspace.thread_or_draft(&started_since)
        }));
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
