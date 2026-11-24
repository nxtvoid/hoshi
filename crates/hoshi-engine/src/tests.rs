use super::*;
use hoshi_core::{
    AccessMode, ApprovalDecision, BackgroundTask, Effort, GitAction, GitActionRequest,
    RequestResponse, RequestStatus, ThreadStatus, ToolDetail,
};
use hoshi_providers::OutputSink;
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug, Clone, PartialEq)]
struct Request {
    cwd: String,
    prompt: String,
    resume: Option<String>,
    resume_at: Option<String>,
    model: Option<String>,
    attachments: Vec<TurnAttachment>,
    instructions: Option<String>,
    subagent_effort: SubagentEffort,
}

/// Records each request and replays a script; a turn whose script does not
/// end it waits for an interrupt.
struct FakeAdapter {
    requests: Mutex<Vec<Request>>,
    script: Vec<ProviderOutput>,
    /// What a title request answers; `None` declines.
    title: Option<String>,
    /// What the user answered the turns' requests, by request id.
    responses: Arc<Mutex<Vec<(String, RequestResponse)>>>,
    /// Messages that joined a running turn, by id; `None` refuses them all.
    steers: Option<Arc<Mutex<Vec<String>>>>,
    /// Ends each waiting turn, in the order they started.
    finishers: Mutex<Vec<Sender<TurnOutcome>>>,
}

struct FakeTurn {
    stop: Sender<TurnOutcome>,
    responses: Arc<Mutex<Vec<(String, RequestResponse)>>>,
    steers: Option<Arc<Mutex<Vec<String>>>>,
}

impl RunningTurn for FakeTurn {
    fn interrupt(&self) {
        let _ = self.stop.send(TurnOutcome::Interrupted);
    }

    fn stop_background_task(&self, _task_id: &str) {
        let _ = self.stop.send(TurnOutcome::Completed);
    }

    fn steer(
        &self,
        message_id: &str,
        _prompt: &str,
        _attachments: &[TurnAttachment],
        _at_once: bool,
    ) -> bool {
        let Some(steers) = &self.steers else {
            return false;
        };
        steers.lock().unwrap().push(message_id.to_owned());
        true
    }

    fn respond(&self, request_id: &str, response: &RequestResponse) {
        self.responses
            .lock()
            .unwrap()
            .push((request_id.to_owned(), response.clone()));
    }
}

impl ProviderAdapter for FakeAdapter {
    fn provider(&self) -> &'static str {
        hoshi_core::DEFAULT_PROVIDER
    }

    fn models(&self) -> hoshi_core::ProviderModels {
        hoshi_core::ProviderModels::default()
    }

    fn generate_title(&self, _request: &TitleRequest) -> Option<hoshi_providers::GeneratedTitle> {
        Some(hoshi_providers::GeneratedTitle {
            title: self.title.clone()?,
            needs_refinement: false,
        })
    }

    fn compacts_idle_threads(&self) -> bool {
        true
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        mut sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        let mut requests = self.requests.lock().unwrap();
        let turn = requests.len();
        requests.push(Request {
            cwd: request.cwd,
            prompt: request.prompt,
            resume: request.resume_session_id,
            resume_at: request.resume_session_at,
            model: request.model,
            attachments: request.attachments,
            instructions: request.instructions,
            subagent_effort: request.subagent_effort,
        });
        let script: Vec<_> = self
            .script
            .iter()
            .cloned()
            .map(|output| match output {
                ProviderOutput::AssistantMessage { item_id, text } => {
                    let item_id = format!("{turn}:{item_id}");
                    ProviderOutput::AssistantMessage { item_id, text }
                }
                other => other,
            })
            .collect();
        let (stop, stopped) = mpsc::channel();
        self.finishers.lock().unwrap().push(stop.clone());
        thread::spawn(move || {
            let ends = script
                .iter()
                .any(|output| matches!(output, ProviderOutput::TurnCompleted { .. }));
            script.into_iter().for_each(&mut sink);
            if !ends {
                let outcome = stopped.recv().unwrap_or(TurnOutcome::Interrupted);
                sink(ProviderOutput::TurnCompleted { outcome });
            }
        });
        Ok(Box::new(FakeTurn {
            stop,
            responses: self.responses.clone(),
            steers: self.steers.clone(),
        }))
    }
}

struct Harness {
    engine: Engine,
    adapter: Arc<FakeAdapter>,
    events: Receiver<Event>,
    root: String,
    data_dir: PathBuf,
}

struct LimitsAdapter {
    asks: Mutex<u8>,
}

impl ProviderAdapter for LimitsAdapter {
    fn provider(&self) -> &'static str {
        hoshi_core::DEFAULT_PROVIDER
    }

    fn models(&self) -> hoshi_core::ProviderModels {
        hoshi_core::ProviderModels::default()
    }

    fn usage_limits(&self) -> Option<UsageLimits> {
        thread::sleep(Duration::from_millis(50));
        let mut asks = self.asks.lock().unwrap();
        *asks += 1;
        Some(UsageLimits {
            windows: vec![hoshi_core::UsageWindow {
                kind: hoshi_core::UsageWindowKind::Session,
                model: None,
                used_percent: *asks,
                window_minutes: 300,
                resets_at: None,
            }],
            banked_resets: None,
            account: None,
            extra_usage: None,
        })
    }

    fn start_turn(
        &self,
        _request: TurnRequest,
        _sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        Err(TurnFailure::ProviderDisabled)
    }
}

/// Banks one reset of `grant_a` and answers each spend with the next of `answers`.
struct ResetAdapter {
    answers: Mutex<Vec<BankedResetSpend>>,
    spends: Mutex<Vec<(String, String)>>,
}

impl ProviderAdapter for ResetAdapter {
    fn provider(&self) -> &'static str {
        hoshi_core::DEFAULT_PROVIDER
    }

    fn models(&self) -> hoshi_core::ProviderModels {
        hoshi_core::ProviderModels::default()
    }

    fn usage_limits(&self) -> Option<UsageLimits> {
        Some(UsageLimits {
            windows: Vec::new(),
            banked_resets: Some(hoshi_core::BankedResets {
                count: 2,
                next_grant_id: Some("grant_a".into()),
                expires_at: Vec::new(),
                grants: ["grant_a", "grant_b"]
                    .map(|id| hoshi_core::BankedGrant {
                        id: id.into(),
                        kind: hoshi_core::ResetKind::Session,
                        resets: 1,
                        expires_at: None,
                    })
                    .into(),
            }),
            account: None,
            extra_usage: None,
        })
    }

    fn spend_banked_reset(&self, grant_id: &str, request_id: &str) -> BankedResetSpend {
        self.spends
            .lock()
            .unwrap()
            .push((grant_id.to_owned(), request_id.to_owned()));
        self.answers.lock().unwrap().remove(0)
    }

    fn start_turn(
        &self,
        _request: TurnRequest,
        _sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        Err(TurnFailure::ProviderDisabled)
    }
}

/// Reports 1.0.0 against a 1.1.0 release until its updater ran, or nothing installed when
/// `missing`. Its update writes `marker`, which the installed version reads.
struct OutdatedAdapter {
    marker: PathBuf,
    missing: bool,
}

impl ProviderAdapter for OutdatedAdapter {
    fn provider(&self) -> &'static str {
        hoshi_core::DEFAULT_PROVIDER
    }

    fn models(&self) -> hoshi_core::ProviderModels {
        hoshi_core::ProviderModels::default()
    }

    fn account_status(&self) -> ProviderAccountStatus {
        if self.marker.is_file() || !self.missing {
            ProviderAccountStatus::Ready
        } else {
            ProviderAccountStatus::NotInstalled
        }
    }

    fn installation(&self) -> Option<Installation> {
        let updated = self.marker.is_file();
        let marker = self.marker.display();
        let (program, args) = if cfg!(windows) {
            (
                "cmd",
                vec!["/C".to_owned(), format!("echo updated> {marker}")],
            )
        } else {
            (
                "sh",
                vec!["-c".to_owned(), format!("echo updated > '{marker}'")],
            )
        };
        let installed = match (updated, self.missing) {
            (true, _) => Some("1.1.0".into()),
            (false, true) => None,
            (false, false) => Some("1.0.0".into()),
        };
        Some(Installation {
            installed,
            latest: Some("1.1.0".into()),
            updater: Some(Updater::Command {
                program: program.into(),
                args,
            }),
        })
    }

    fn start_turn(
        &self,
        _request: TurnRequest,
        _sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        Err(TurnFailure::ProviderDisabled)
    }
}

#[test]
fn an_outdated_cli_is_updated_and_checked_again() {
    let (sender, versions) = mpsc::channel();
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::ProviderVersion(ProviderVersionUpdate {
            version: Some(version),
            ..
        }) = notification
        {
            let _ = sender.send(version);
        }
    });
    let dir = scratch_dir("provider-update");
    let engine = Engine::from_store(Store::open_in_memory().unwrap(), &dir, notify).unwrap();
    engine.set_adapter(Some(Arc::new(OutdatedAdapter {
        marker: dir.join("updated"),
        missing: false,
    })));

    let checked = engine.check_provider_version().unwrap();
    assert!(checked.update_available);
    assert!(checked.update_command.is_some());

    engine.update_provider();
    let finished = versions
        .iter()
        .find(|version| {
            !matches!(
                version.update,
                ProviderUpdate::Idle | ProviderUpdate::Running { .. }
            )
        })
        .unwrap();

    assert_eq!(finished.update, ProviderUpdate::Succeeded);
    assert_eq!(finished.installed.as_deref(), Some("1.1.0"));
    assert!(!finished.update_available);
}

#[test]
fn a_missing_cli_is_installed_and_its_account_checked_again() {
    let (sender, notifications) = mpsc::channel();
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::Providers(status) = notification {
            let _ = sender.send(status);
        }
    });
    let dir = scratch_dir("provider-install");
    let engine = Engine::from_store(Store::open_in_memory().unwrap(), &dir, notify).unwrap();
    engine.set_adapter(Some(Arc::new(OutdatedAdapter {
        marker: dir.join("updated"),
        missing: true,
    })));
    let checked = engine.check_provider_version().unwrap();
    assert_eq!(checked.installed, None);
    assert!(!checked.update_available);

    engine.update_provider();
    let mut revisions = Vec::new();
    let status = loop {
        let status = notifications.recv_timeout(Duration::from_secs(30)).unwrap();
        revisions.push(status.revision);
        if status.accounts.contains_key(DEFAULT_PROVIDER) {
            break status;
        }
    };
    assert!(
        revisions.len() > 1,
        "the update's progress sends the status too"
    );
    assert!(revisions.is_sorted_by(|a, b| a < b), "{revisions:?}");

    let version = &status.versions[DEFAULT_PROVIDER];
    assert_eq!(version.update, ProviderUpdate::Succeeded);
    assert_eq!(version.installed.as_deref(), Some("1.1.0"));
    assert_eq!(
        status.accounts.get(DEFAULT_PROVIDER),
        Some(&ProviderAccountStatus::Ready)
    );
}

/// Reports `version` as installed, with no release to compare it with.
struct InstalledAdapter {
    provider: &'static str,
    version: &'static str,
    account: ProviderAccountStatus,
}

impl ProviderAdapter for InstalledAdapter {
    fn provider(&self) -> &'static str {
        self.provider
    }

    fn models(&self) -> hoshi_core::ProviderModels {
        hoshi_core::ProviderModels::default()
    }

    fn installation(&self) -> Option<Installation> {
        Some(Installation {
            installed: Some(self.version.into()),
            latest: None,
            updater: None,
        })
    }

    fn account_status(&self) -> ProviderAccountStatus {
        self.account
    }

    fn start_turn(
        &self,
        _request: TurnRequest,
        _sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        Err(TurnFailure::ProviderDisabled)
    }
}

fn installed(
    provider: &'static str,
    version: &'static str,
    account: ProviderAccountStatus,
) -> Arc<dyn ProviderAdapter> {
    Arc::new(InstalledAdapter {
        provider,
        version,
        account,
    })
}

fn version_updates() -> (Notify, Receiver<ProviderVersionUpdate>) {
    let (sender, updates) = mpsc::channel();
    let sender = Mutex::new(sender);
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::ProviderVersion(update) = notification {
            let _ = sender.lock().unwrap().send(update);
        }
    });
    (notify, updates)
}

/// Waits for each of `providers` to report a version.
fn wait_for_versions(updates: &Receiver<ProviderVersionUpdate>, providers: &[&str]) {
    let mut waiting: HashSet<&str> = providers.iter().copied().collect();
    while !waiting.is_empty() {
        let update = updates.recv_timeout(Duration::from_secs(10)).unwrap();
        if update.version.is_some() {
            waiting.remove(update.provider.as_str());
        }
    }
}

#[test]
fn each_enabled_provider_keeps_its_own_version_and_account() {
    let (notify, updates) = version_updates();
    let dir = scratch_dir("provider-versions");
    let engine = Engine::from_store(Store::open_in_memory().unwrap(), &dir, notify).unwrap();
    engine.set_adapters(vec![
        installed(DEFAULT_PROVIDER, "2.1.325", ProviderAccountStatus::Ready),
        installed("copilot", "1.0.112", ProviderAccountStatus::NotInstalled),
    ]);
    wait_for_versions(&updates, &[DEFAULT_PROVIDER, "copilot"]);

    let status = engine.check_providers();

    let installed_of = |provider: &str| status.versions[provider].installed.clone();
    assert_eq!(installed_of(DEFAULT_PROVIDER).as_deref(), Some("2.1.325"));
    assert_eq!(installed_of("copilot").as_deref(), Some("1.0.112"));
    assert_eq!(
        status.accounts["copilot"],
        ProviderAccountStatus::NotInstalled
    );
    assert_eq!(
        status.accounts[DEFAULT_PROVIDER],
        ProviderAccountStatus::Ready
    );
    assert!(status.checked_at.is_some());
    assert_eq!(
        engine.provider_version().unwrap().installed.as_deref(),
        Some("2.1.325")
    );
    let again = engine.providers_status();
    assert!(again.revision > status.revision);
    assert_eq!(
        ProvidersStatus {
            revision: status.revision,
            ..again
        },
        status
    );
}

#[test]
fn a_disabled_provider_forgets_its_version_and_says_so() {
    let (notify, updates) = version_updates();
    let dir = scratch_dir("provider-disabled");
    let engine = Engine::from_store(Store::open_in_memory().unwrap(), &dir, notify).unwrap();
    engine.set_adapters(vec![
        installed(DEFAULT_PROVIDER, "2.1.325", ProviderAccountStatus::Ready),
        installed("copilot", "1.0.112", ProviderAccountStatus::Ready),
    ]);
    wait_for_versions(&updates, &[DEFAULT_PROVIDER, "copilot"]);
    engine.check_providers();

    engine.set_adapters(vec![installed(
        DEFAULT_PROVIDER,
        "2.1.325",
        ProviderAccountStatus::Ready,
    )]);

    let forgotten = updates
        .iter()
        .find(|update| update.version.is_none())
        .unwrap();
    assert_eq!(forgotten.provider, "copilot");
    let status = engine.providers_status();
    assert!(!status.versions.contains_key("copilot"));
    assert!(!status.accounts.contains_key("copilot"));
    assert!(status.versions.contains_key(DEFAULT_PROVIDER));
}

#[test]
fn usage_limits_are_read_once_while_fresh_and_kept_across_restarts() {
    let data_dir = scratch_dir("usage-limits");
    let notify: Notify = Arc::new(|_| {});
    let engine = Engine::open(&data_dir, notify.clone()).unwrap();
    let adapter = Arc::new(LimitsAdapter {
        asks: Mutex::new(0),
    });

    engine.set_adapter(Some(adapter.clone()));
    let first = engine.usage_limits(false).unwrap();
    assert_eq!(*adapter.asks.lock().unwrap(), 1);
    assert_eq!(engine.usage_limits(false), Some(first));

    let forced = engine.usage_limits(true).unwrap();
    assert_eq!(*adapter.asks.lock().unwrap(), 2);
    drop(engine);

    let restarted = Engine::open(&data_dir, notify).unwrap();
    assert_eq!(restarted.known_usage_limits(), Some(forced));
}

#[test]
fn settings_deletes_and_quitting_never_wait_for_a_busy_browser() {
    let engine = Engine::open(&scratch_dir("busy-browser"), Arc::new(|_| {})).unwrap();
    engine
        .dispatch(Command::ThreadCreate {
            thread_id: "t".into(),
            project_id: None,
            title: "chat".into(),
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
    let (held, busy) = mpsc::channel();
    let browsing = engine.browsing.clone();
    let slow_page = thread::spawn(move || {
        let _launching = browsing.hold_launch();
        held.send(()).unwrap();
        thread::sleep(Duration::from_secs(5));
    });
    busy.recv().unwrap();

    let started = Instant::now();
    engine.set_browser(BrowserSettings {
        agent_access: false,
        ..BrowserSettings::default()
    });
    engine
        .dispatch(Command::ThreadDelete {
            thread_id: "t".into(),
        })
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    engine.close_browser();
    assert!(started.elapsed() < BROWSER_CLOSE_WAIT + Duration::from_secs(1));
    assert!(!hold(&engine.browser_settings).agent_access);
    slow_page.join().unwrap();
}

#[test]
fn an_unanswered_spend_is_retried_with_the_same_request() {
    let data_dir = scratch_dir("banked-resets");
    let engine = Engine::open(&data_dir, Arc::new(|_| {})).unwrap();
    let unconfirmed = BankedResetSpend::Failed {
        reason: BankedResetFailure::Unconfirmed,
    };
    let adapter = Arc::new(ResetAdapter {
        answers: Mutex::new(vec![
            unconfirmed,
            BankedResetSpend::Reset,
            BankedResetSpend::NothingToReset,
        ]),
        spends: Mutex::default(),
    });

    assert_eq!(engine.spend_banked_reset(), BankedResetSpend::NoneLeft);
    engine.set_adapter(Some(adapter.clone()));
    engine.usage_limits(true).unwrap();
    assert_eq!(engine.spend_banked_reset(), unconfirmed);
    assert_eq!(engine.spend_banked_reset(), BankedResetSpend::Reset);
    assert_eq!(
        engine.spend_banked_reset(),
        BankedResetSpend::NothingToReset
    );

    let spends = adapter.spends.lock().unwrap();
    assert!(spends.iter().all(|(grant, _)| grant == "grant_a"));
    assert_eq!(spends[0].1, spends[1].1);
    assert_ne!(spends[1].1, spends[2].1);
}

#[test]
fn a_turn_s_defaults_are_named_by_the_catalog() {
    let mut catalog = catalog_of(DEFAULT_PROVIDER, &["opus", "haiku"]);
    catalog.models[0].efforts = vec![Effort::Medium, Effort::High];
    catalog.models[0].default_effort = Some(Effort::Medium);
    let choice = |model: Option<&str>, effort| ModelChoice {
        provider: DEFAULT_PROVIDER.into(),
        model: model.map(str::to_owned),
        effort,
        fast_mode: false,
        ultracode: false,
    };
    let named_as = |mut choice: ModelChoice, recorded| {
        name_choice(&mut choice, &catalog, recorded);
        (choice.model, choice.effort)
    };
    let named = |choice| named_as(choice, false);
    assert_eq!(
        taken_effort(&catalog, Some("opus"), Some(Effort::Max)),
        Some(Effort::Medium),
        "a turn is sent the effort its history records"
    );
    assert_eq!(
        taken_effort(&catalog, None, Some(Effort::High)),
        Some(Effort::High)
    );
    assert_eq!(
        taken_effort(&catalog, Some("gone"), Some(Effort::Max)),
        Some(Effort::Max)
    );
    assert_eq!(taken_effort(&catalog, Some("opus"), None), None);

    assert_eq!(
        named(choice(None, None)),
        (Some("opus".into()), Some(Effort::Medium))
    );
    assert_eq!(
        named(choice(None, Some(Effort::High))),
        (Some("opus".into()), Some(Effort::High))
    );
    assert_eq!(
        named(choice(Some("opus"), Some(Effort::Max))),
        (Some("opus".into()), Some(Effort::Medium)),
        "an effort the model doesn't take is its default"
    );
    assert_eq!(
        named_as(choice(Some("opus"), Some(Effort::Max)), true),
        (Some("opus".into()), Some(Effort::Max)),
        "what a turn ran on stays as it was, though the model no longer takes it"
    );
    assert_eq!(
        named_as(choice(None, None), true),
        (Some("opus".into()), Some(Effort::Medium))
    );
    assert_eq!(
        named(choice(Some("haiku"), Some(Effort::High))),
        (Some("haiku".into()), None)
    );
    assert_eq!(
        named(choice(Some("gone"), Some(Effort::High))),
        (Some("gone".into()), Some(Effort::High)),
        "a model the catalog doesn't list stays as it was"
    );
}

#[test]
fn a_spend_draws_from_the_grant_the_user_picked_when_it_is_listed() {
    let data_dir = scratch_dir("banked-grant");
    let engine = Engine::open(&data_dir, Arc::new(|_| {})).unwrap();
    let adapter = Arc::new(ResetAdapter {
        answers: Mutex::new(vec![BankedResetSpend::Reset]),
        spends: Mutex::default(),
    });
    engine.set_adapter(Some(adapter.clone()));
    engine.usage_limits(true).unwrap();

    assert_eq!(
        engine.spend_banked_grant(DEFAULT_PROVIDER, Some("unknown")),
        BankedResetSpend::NoneLeft
    );
    assert_eq!(
        engine.spend_banked_grant(DEFAULT_PROVIDER, Some("grant_b")),
        BankedResetSpend::Reset
    );
    let spends = adapter.spends.lock().unwrap();
    assert_eq!(spends.len(), 1);
    assert_eq!(spends[0].0, "grant_b");
}

#[test]
fn secret_files_past_their_day_go_when_the_engine_starts() {
    let data_dir = scratch_dir("old-secrets");
    let secrets = data_dir.join("secrets");
    std::fs::create_dir_all(&secrets).unwrap();
    let (old, fresh) = (secrets.join("old"), secrets.join("fresh"));
    std::fs::write(&old, "token-1").unwrap();
    std::fs::write(&fresh, "token-2").unwrap();
    let file = std::fs::File::options().write(true).open(&old).unwrap();
    file.set_modified(std::time::SystemTime::now() - Duration::from_secs(25 * 60 * 60))
        .unwrap();
    drop(file);

    let _engine = Engine::from_store(
        Store::open_in_memory().unwrap(),
        &data_dir,
        Arc::new(|_| {}),
    )
    .unwrap();

    assert!(!old.exists());
    assert!(fresh.exists());
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hoshi-engine-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

impl Harness {
    /// An engine with project `p` on `root` and a local thread `t`.
    fn new(name: &str, root: PathBuf, store: Store, script: Vec<ProviderOutput>) -> Self {
        let (sender, events) = mpsc::channel();
        let notify: Notify = Arc::new(move |notification| {
            if let Notification::Event(envelope) = notification {
                let _ = sender.send(envelope.event);
            }
        });
        let data_dir = scratch_dir(&format!("{name}-data"));
        let engine = Engine::from_store(store, &data_dir, notify).unwrap();
        let adapter = Arc::new(FakeAdapter {
            requests: Mutex::new(Vec::new()),
            script,
            title: None,
            responses: Arc::default(),
            steers: None,
            finishers: Mutex::default(),
        });
        engine.set_adapter(Some(adapter.clone()));
        let root = dunce::canonicalize(root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        engine
            .dispatch(Command::ProjectCreate {
                project_id: "p".into(),
                title: "Repo".into(),
                workspace_root: root.clone(),
            })
            .unwrap();
        let harness = Self {
            engine,
            adapter,
            events,
            root,
            data_dir,
        };
        harness.create_thread("t", ThreadWorkspace::Local).unwrap();
        harness
    }

    fn in_memory(name: &str, script: Vec<ProviderOutput>) -> Self {
        Self::new(
            name,
            scratch_dir(name),
            Store::open_in_memory().unwrap(),
            script,
        )
    }

    fn create_thread(&self, id: &str, workspace: ThreadWorkspace) -> Result<(), CommandError> {
        self.create_thread_in(id, Some("p"), workspace)
    }

    fn create_thread_in(
        &self,
        id: &str,
        project_id: Option<&str>,
        workspace: ThreadWorkspace,
    ) -> Result<(), CommandError> {
        self.engine.dispatch(Command::ThreadCreate {
            thread_id: id.into(),
            project_id: project_id.map(str::to_owned),
            title: String::new(),
            workspace,
            provider: hoshi_core::default_provider(),
            model: Some("haiku".into()),
            effort: Some(Effort::Low),
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::AcceptEdits,
            plan_mode: false,
        })
    }

    fn send(&self, thread_id: &str, message_id: &str, text: &str) {
        self.engine
            .dispatch(Command::MessageSend {
                thread_id: thread_id.into(),
                message_id: message_id.into(),
                text: text.into(),
                attachments: Vec::new(),
            })
            .unwrap();
    }

    fn wait_for<T>(&self, find: impl FnMut(Event) -> Option<T>) -> T {
        self.events.iter().find_map(find).unwrap()
    }

    fn wait_for_turn_end(&self) -> TurnOutcome {
        self.wait_for(|event| match event {
            Event::TurnCompleted { outcome, .. } => Some(outcome),
            _ => None,
        })
    }

    fn thread(&self, thread_id: &str) -> hoshi_core::Thread {
        self.engine
            .snapshot()
            .threads
            .into_iter()
            .find(|thread| thread.id == thread_id)
            .unwrap()
    }
}

#[test]
fn a_turn_s_background_commands_are_known_until_it_ends_and_can_be_stopped() {
    let harness = Harness::in_memory("background-commands", Vec::new());
    harness.send("t", "m1", "run it in the background");
    let turn_id = harness.wait_for(|event| match event {
        Event::TurnStarted { turn_id, .. } => Some(turn_id),
        _ => None,
    });
    let task = BackgroundTask {
        task_id: "b1".into(),
        tool_call_id: Some("toolu_1".into()),
        description: "Start the dev server".into(),
        started_at: 1,
    };
    lock(&harness.engine.state).receive(
        "t",
        &turn_id,
        ProviderOutput::BackgroundTasks {
            tasks: vec![task.clone()],
            waiting: true,
        },
    );
    assert_eq!(
        harness.engine.snapshot().background_tasks,
        [ThreadBackgroundTasks {
            thread_id: "t".into(),
            tasks: vec![task],
            waiting: true,
        }]
    );

    harness
        .engine
        .dispatch(Command::BackgroundTaskStop {
            thread_id: "t".into(),
            task_id: "b1".into(),
        })
        .unwrap();

    assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
    assert!(harness.engine.snapshot().background_tasks.is_empty());
}

fn completed_turn() -> Vec<ProviderOutput> {
    vec![
        ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        },
        ProviderOutput::AssistantMessage {
            item_id: "a".into(),
            text: "done".into(),
        },
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    ]
}

#[test]
fn a_turn_runs_in_the_project_root_and_the_next_one_resumes_its_session() {
    let harness = Harness::in_memory("resume", completed_turn());

    harness.send("t", "m1", "first");
    assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
    harness.send("t", "m2", "second");
    harness.wait_for_turn_end();

    let request = |prompt: &str, resume: Option<&str>| Request {
        cwd: harness.root.clone(),
        prompt: prompt.into(),
        resume: resume.map(Into::into),
        resume_at: None,
        model: Some("haiku".into()),
        attachments: Vec::new(),
        instructions: Some(delegation::instructions(0, true, false, true)),
        subagent_effort: SubagentEffort::AgentDecides,
    };
    assert_eq!(
        *harness.adapter.requests.lock().unwrap(),
        [request("first", None), request("second", Some("s1"))]
    );
    assert_eq!(harness.thread("t").status, ThreadStatus::Idle);
    assert_eq!(harness.engine.transcript("t", None).unwrap().items.len(), 4);
}

#[test]
fn a_chat_runs_in_its_own_folder_which_goes_away_with_it() {
    let harness = Harness::in_memory("chat", completed_turn());
    harness
        .create_thread_in("chat-1", None, ThreadWorkspace::Local)
        .unwrap();

    harness.send("chat-1", "m1", "hello");
    assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);

    let folder = harness.data_dir.join("chats").join("chat-1");
    let request = harness.adapter.requests.lock().unwrap()[0].clone();
    assert_eq!(PathBuf::from(&request.cwd), folder);
    assert!(folder.is_dir());
    let instructions = request.instructions.unwrap();
    assert!(instructions.contains("delegate_task"));
    assert!(!instructions.contains("watch_pull_request"));
    {
        let mut state = lock(&harness.engine.state);
        assert!(!state.offers_tool("chat-1", "watch_pull_request"));
        let isolated = state.call_tool(
            "chat-1",
            "delegate_task",
            &serde_json::json!({ "task": "look", "isolation": "worktree" }),
        );
        assert!(isolated.is_err());
    }
    assert_eq!(
        harness.create_thread_in("chat-2", None, ThreadWorkspace::NewWorktree),
        Err(CommandError::ChatNotLocal {
            thread_id: "chat-2".into()
        })
    );
    assert_eq!(
        harness.engine.search_paths(None, None, "").unwrap(),
        Vec::new()
    );

    harness
        .engine
        .dispatch(Command::ThreadDelete {
            thread_id: "chat-1".into(),
        })
        .unwrap();
    assert!(!folder.exists());
}

#[test]
fn a_sent_image_reaches_the_provider_and_goes_away_with_its_thread() {
    let harness = Harness::in_memory("image", completed_turn());
    let image = harness
        .engine
        .save_image("shot.png", "image/png", "cG5n")
        .unwrap();

    harness
        .engine
        .dispatch(Command::MessageSend {
            thread_id: "t".into(),
            message_id: "m1".into(),
            text: "look".into(),
            attachments: vec![image.clone()],
        })
        .unwrap();
    harness.wait_for_turn_end();
    let sent = harness.adapter.requests.lock().unwrap()[0].attachments[0].clone();
    let path = sent.path.expect("the image's file");

    assert_eq!(sent.attachment, image);
    assert!(path.exists());
    harness
        .engine
        .dispatch(Command::ThreadDelete {
            thread_id: "t".into(),
        })
        .unwrap();
    assert!(!path.exists());
}

#[test]
fn removing_a_project_forgets_what_the_engine_kept_for_its_threads() {
    let harness = Harness::in_memory("project-removed", completed_turn());
    {
        let mut state = lock(&harness.engine.state);
        let message = || QueuedMessage {
            id: "m1".into(),
            text: "later".into(),
            attachments: Vec::new(),
            from_user: true,
            held: false,
            to: None,
        };
        state
            .queued
            .entry("t".into())
            .or_default()
            .push_back(message());
        state.after_compaction.insert("t".into(), message());
        state.watches.insert("t".into(), Default::default());
        state.pending_wakes.insert(
            "t".into(),
            vec![Wake {
                update: hoshi_core::PullRequestUpdate::default(),
                text: "news".into(),
            }],
        );
    }

    harness
        .engine
        .dispatch(Command::ProjectRemove {
            project_id: "p".into(),
        })
        .unwrap();

    let state = lock(&harness.engine.state);
    assert!(state.model.thread("t").is_none());
    assert!(state.queued.is_empty());
    assert!(state.after_compaction.is_empty());
    assert!(state.watches.is_empty());
    assert!(state.pending_wakes.is_empty());
}

fn titled_harness(name: &str) -> Harness {
    let harness = Harness::in_memory(name, completed_turn());
    harness.engine.set_adapter(Some(Arc::new(FakeAdapter {
        requests: Mutex::new(Vec::new()),
        script: completed_turn(),
        title: Some("Empty TSX Snippet".into()),
        responses: Arc::default(),
        steers: None,
        finishers: Mutex::default(),
    })));
    harness
}

#[test]
fn a_first_message_gets_a_generated_title() {
    let harness = titled_harness("generated-title");

    harness.send("t", "m1", "```tsx\n```");

    let title = harness.wait_for(|event| match event {
        Event::ThreadTitleSet { title, .. } if !title.is_empty() => Some(title),
        _ => None,
    });
    assert_eq!(title, "Empty TSX Snippet");
}

/// A fake adapter answering as another provider.
struct OtherProvider(Arc<FakeAdapter>);

const OTHER_PROVIDER: &str = "copilot";

impl ProviderAdapter for OtherProvider {
    fn provider(&self) -> &'static str {
        OTHER_PROVIDER
    }

    fn models(&self) -> hoshi_core::ProviderModels {
        self.0.models()
    }

    fn generate_title(&self, request: &TitleRequest) -> Option<hoshi_providers::GeneratedTitle> {
        self.0.generate_title(request)
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        self.0.start_turn(request, sink)
    }
}

fn fake(title: Option<&str>) -> Arc<FakeAdapter> {
    Arc::new(FakeAdapter {
        requests: Mutex::new(Vec::new()),
        script: completed_turn(),
        title: title.map(str::to_owned),
        responses: Arc::default(),
        steers: None,
        finishers: Mutex::default(),
    })
}

#[test]
fn a_thread_runs_on_the_provider_it_chose() {
    let harness = Harness::in_memory("other-provider", completed_turn());
    let (default, other) = (fake(None), fake(None));
    harness.engine.set_adapters(vec![
        default.clone(),
        Arc::new(OtherProvider(other.clone())),
    ]);
    harness
        .engine
        .dispatch(Command::ThreadModelSet {
            thread_id: "t".into(),
            provider: OTHER_PROVIDER.into(),
            model: Some("gpt-6.1-sol".into()),
            effort: None,
            fast_mode: false,
            ultracode: false,
        })
        .unwrap();

    harness.send("t", "m1", "hi");
    harness.wait_for_turn_end();

    assert!(default.requests.lock().unwrap().is_empty());
    let requests = other.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model.as_deref(), Some("gpt-6.1-sol"));
}

#[test]
fn limits_a_turn_reports_are_its_threads_provider_s() {
    let harness = Harness::in_memory("limits-by-provider", completed_turn());
    let window = |kind, used_percent| hoshi_core::UsageWindow {
        kind,
        model: None,
        used_percent,
        window_minutes: 300,
        resets_at: None,
    };
    let reported = |windows| {
        ProviderOutput::UsageLimits(UsageLimits {
            windows,
            banked_resets: None,
            account: None,
            extra_usage: None,
        })
    };
    let mut script = completed_turn();
    script.insert(
        1,
        reported(vec![
            window(hoshi_core::UsageWindowKind::Session, 70),
            window(hoshi_core::UsageWindowKind::Weekly, 40),
        ]),
    );
    script.insert(
        2,
        reported(vec![window(hoshi_core::UsageWindowKind::Session, 100)]),
    );
    let other = Arc::new(FakeAdapter {
        script,
        ..Arc::into_inner(fake(None)).unwrap()
    });
    harness
        .engine
        .set_adapters(vec![fake(None), Arc::new(OtherProvider(other))]);
    harness
        .engine
        .dispatch(Command::ThreadModelSet {
            thread_id: "t".into(),
            provider: OTHER_PROVIDER.into(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
        })
        .unwrap();

    harness.send("t", "m1", "hi");
    harness.wait_for_turn_end();

    assert_eq!(harness.engine.known_usage_limits(), None);
    let limits = harness
        .engine
        .known_usage_limits_of(OTHER_PROVIDER)
        .unwrap();
    let used: Vec<u8> = limits.windows.iter().map(|w| w.used_percent).collect();
    assert_eq!(used, [100, 40]);
    let status = harness.engine.providers_status();
    assert_eq!(status.usage_limits.get(OTHER_PROVIDER), Some(&limits));
    assert!(!status.usage_limits.contains_key(DEFAULT_PROVIDER));
}

#[test]
fn each_provider_is_asked_for_its_own_limits() {
    let data_dir = scratch_dir("limits-each-provider");
    let engine = Engine::open(&data_dir, Arc::new(|_| {})).unwrap();
    let (claude, other) = (
        Arc::new(LimitsAdapter {
            asks: Mutex::new(0),
        }),
        Arc::new(LimitsAdapter {
            asks: Mutex::new(9),
        }),
    );
    engine.set_adapters(vec![claude.clone(), Arc::new(OtherLimits(other.clone()))]);

    assert_eq!(
        engine.usage_limits(false).unwrap().windows[0].used_percent,
        1
    );
    let others = engine.usage_limits_of(OTHER_PROVIDER, false).unwrap();
    assert_eq!(others.windows[0].used_percent, 10);
    assert_eq!(engine.usage_limits_of("codex", false), None);
    assert_eq!(*claude.asks.lock().unwrap(), 1);
    assert_eq!(*other.asks.lock().unwrap(), 10);
    assert_eq!(engine.usage_limit_providers(), Vec::<String>::new());
    drop(engine);

    let restarted = Engine::open(&data_dir, Arc::new(|_| {})).unwrap();
    assert_eq!(
        restarted.known_usage_limits_of(OTHER_PROVIDER),
        Some(others)
    );
}

/// Limits answered as another provider.
struct OtherLimits(Arc<LimitsAdapter>);

impl ProviderAdapter for OtherLimits {
    fn provider(&self) -> &'static str {
        OTHER_PROVIDER
    }

    fn models(&self) -> hoshi_core::ProviderModels {
        hoshi_core::ProviderModels::default()
    }

    fn usage_limits(&self) -> Option<UsageLimits> {
        self.0.usage_limits()
    }

    fn start_turn(
        &self,
        request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        self.0.start_turn(request, sink)
    }
}

#[test]
fn a_thread_on_a_disabled_provider_fails_its_turn() {
    let harness = Harness::in_memory("disabled-other", completed_turn());
    harness
        .engine
        .dispatch(Command::ThreadModelSet {
            thread_id: "t".into(),
            provider: OTHER_PROVIDER.into(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
        })
        .unwrap();

    harness.send("t", "m1", "hi");

    assert_eq!(
        harness.wait_for_turn_end(),
        TurnOutcome::Failed {
            reason: TurnFailure::ProviderDisabled
        }
    );
}

#[test]
fn titles_come_from_the_provider_the_settings_name() {
    let harness = Harness::in_memory("other-titles", completed_turn());
    harness.engine.set_adapters(vec![
        fake(Some("From Claude")),
        Arc::new(OtherProvider(fake(Some("From Copilot")))),
    ]);
    harness.engine.set_thread_titles(ThreadTitleSettings {
        provider: Some(OTHER_PROVIDER.into()),
        ..ThreadTitleSettings::default()
    });

    harness.send("t", "m1", "fix the login bug");

    let title = harness.wait_for(|event| match event {
        Event::ThreadTitleSet { title, .. } if title != "fix the login bug" => Some(title),
        _ => None,
    });
    assert_eq!(title, "From Copilot");
}

#[test]
fn titles_can_be_turned_off_but_still_regenerated_on_request() {
    let harness = titled_harness("titles-off");
    harness.engine.set_thread_titles(ThreadTitleSettings {
        generate: false,
        ..ThreadTitleSettings::default()
    });
    harness.send("t", "m1", "fix the login bug");
    harness.wait_for_turn_end();
    assert_eq!(harness.thread("t").title, "fix the login bug");

    harness.engine.regenerate_title("t").unwrap();

    let title = harness.wait_for(|event| match event {
        Event::ThreadTitleSet { title, .. } => Some(title),
        _ => None,
    });
    assert_eq!(title, "Empty TSX Snippet");
}

#[test]
fn interrupting_stops_the_turn() {
    let harness = Harness::in_memory("interrupt", Vec::new());
    harness.send("t", "m1", "long task");

    harness
        .engine
        .dispatch(Command::TurnInterrupt {
            thread_id: "t".into(),
        })
        .unwrap();

    assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Interrupted);
    assert_eq!(harness.thread("t").status, ThreadStatus::Idle);
}

#[test]
fn a_disabled_provider_fails_the_turn() {
    let harness = Harness::in_memory("disabled", completed_turn());
    harness.engine.set_adapter(None);

    harness.send("t", "m1", "hi");

    assert_eq!(
        harness.wait_for_turn_end(),
        TurnOutcome::Failed {
            reason: TurnFailure::ProviderDisabled
        }
    );
    assert_eq!(harness.thread("t").status, ThreadStatus::Error);
}

#[test]
fn empty_folders_of_chats_never_started_go_at_start() {
    let data_dir = scratch_dir("chat-folders-data");
    let chats = data_dir.join("chats");
    std::fs::create_dir_all(chats.join("never-sent")).unwrap();
    std::fs::create_dir_all(chats.join("kept")).unwrap();
    std::fs::write(chats.join("kept").join("notes.txt"), "keep").unwrap();
    let notify: Notify = Arc::new(|_| {});
    let _engine = Engine::from_store(Store::open_in_memory().unwrap(), &data_dir, notify).unwrap();
    assert!(!chats.join("never-sent").exists());
    assert!(chats.join("kept").join("notes.txt").exists());
}

#[test]
fn a_drafts_terminal_lives_through_other_commands_and_goes_with_its_thread() {
    let harness = Harness::in_memory("draft-terminal", Vec::new());
    let sink = || -> TerminalSink { Box::new(|_| {}) };
    let attach = harness
        .engine
        .open_draft_terminal("d1", Some("p"), "draft-term", (80, 24), sink())
        .unwrap();
    assert_eq!(attach.cwd, harness.root);
    let chat = harness
        .engine
        .open_draft_terminal("d2", None, "chat-term", (80, 24), sink())
        .unwrap();
    assert_eq!(
        PathBuf::from(chat.cwd),
        harness.data_dir.join("chats").join("d2")
    );

    harness
        .engine
        .dispatch(Command::ThreadVisit {
            thread_id: "t".into(),
        })
        .unwrap();
    assert!(harness.engine.write_terminal("draft-term", "").is_ok());
    assert!(harness.engine.write_terminal("chat-term", "").is_ok());

    harness.create_thread("d1", ThreadWorkspace::Local).unwrap();
    assert!(harness.engine.write_terminal("draft-term", "").is_ok());
    harness
        .engine
        .dispatch(Command::ThreadDelete {
            thread_id: "d1".into(),
        })
        .unwrap();
    assert!(harness.engine.write_terminal("draft-term", "").is_err());
    assert!(harness.engine.write_terminal("chat-term", "").is_ok());
    harness.engine.close_terminal("chat-term");
}

#[test]
fn a_chat_whose_folder_cannot_be_made_fails_its_turn() {
    let harness = Harness::in_memory("chat-no-folder", completed_turn());
    std::fs::write(harness.data_dir.join("chats"), "not a folder").unwrap();
    harness
        .create_thread_in("chat-1", None, ThreadWorkspace::Local)
        .unwrap();

    harness.send("chat-1", "m1", "hello");

    assert!(matches!(
        harness.wait_for_turn_end(),
        TurnOutcome::Failed {
            reason: TurnFailure::SpawnFailed { .. }
        }
    ));
    assert_eq!(harness.thread("chat-1").status, ThreadStatus::Error);
    assert!(harness.adapter.requests.lock().unwrap().is_empty());
}

#[test]
fn a_turn_left_running_by_a_crash_is_closed_on_startup() {
    let dir = scratch_dir("crash");
    let database = dir.join("state.sqlite");
    let harness = Harness::new("crash", dir, Store::open(&database).unwrap(), Vec::new());
    harness.send("t", "m1", "long task");
    drop(harness);

    let engine = Engine::from_store(
        Store::open(&database).unwrap(),
        &scratch_dir("crash-reopened"),
        Arc::new(|_| {}),
    )
    .unwrap();

    let thread = &engine.snapshot().threads[0];
    assert_eq!(thread.status, ThreadStatus::Idle);
    assert_eq!(thread.active_turn_id, None);
}

/// The engine on `database` again, as after a restart, with a fresh fake provider.
fn reopen(name: &str, database: &Path) -> (Engine, Arc<FakeAdapter>, Receiver<Event>) {
    let (sender, events) = mpsc::channel();
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::Event(envelope) = notification {
            let _ = sender.send(envelope.event);
        }
    });
    let engine = Engine::from_store(
        Store::open(database).unwrap(),
        &scratch_dir(&format!("{name}-reopened")),
        notify,
    )
    .unwrap();
    let adapter = adapter(None);
    engine.set_adapter(Some(adapter.clone()));
    (engine, adapter, events)
}

#[test]
fn the_last_merged_cleanup_survives_a_restart() {
    let dir = scratch_dir("merged-cleanup-restart");
    let database = dir.join("state.sqlite");
    let harness = Harness::new(
        "merged-cleanup-restart",
        dir,
        Store::open(&database).unwrap(),
        Vec::new(),
    );
    let cleanup = harness.engine.delete_merged_worktrees();
    drop(harness);

    let (engine, ..) = reopen("merged-cleanup-restart", &database);
    assert_eq!(engine.merged_cleanup(), Some(cleanup));
}

/// Once the engine is done starting it.
fn wait_for_turn_start(engine: &Engine, events: &Receiver<Event>) -> String {
    let turn_id = events
        .iter()
        .find_map(|event| match event {
            Event::TurnStarted { turn_id, .. } => Some(turn_id),
            _ => None,
        })
        .unwrap();
    engine.snapshot();
    turn_id
}

/// Why hoshi sent the message that opened turn `turn_id` of thread "t" by itself.
fn resumption(engine: &Engine, turn_id: &str) -> Option<hoshi_core::Resumption> {
    match engine.thread_item("t", turn_id).unwrap()?.content {
        ItemContent::UserMessage { resumed, .. } => resumed,
        _ => None,
    }
}

fn session_started(harness: &Harness) {
    harness.wait_for(|event| matches!(event, Event::SessionAttached { .. }).then_some(()));
}

#[test]
fn a_turn_a_restart_cut_short_goes_on_once_told_of_the_subagent_it_lost() {
    let dir = scratch_dir("restart-continues");
    let database = dir.join("state.sqlite");
    let script = vec![
        ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        },
        ProviderOutput::ToolCallStarted {
            item_id: "toolu_a".into(),
            name: "Agent".into(),
            input: serde_json::json!({ "description": "Count the files" }).to_string(),
        },
    ];
    let harness = Harness::new(
        "restart-continues",
        dir,
        Store::open(&database).unwrap(),
        script,
    );
    harness.send("t", "m1", "count them");
    harness.wait_for(|event| match event {
        Event::ItemAdded { item } if item.id == "toolu_a" => Some(()),
        _ => None,
    });
    drop(harness);

    let (engine, adapter, events) = reopen("restart-continues", &database);
    assert_eq!(engine.snapshot().threads[0].active_turn_id, None);
    engine.continue_cut_threads();
    wait_for_turn_start(&engine, &events);
    engine.continue_cut_threads();

    let requests = adapter.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].resume.as_deref(), Some("s1"));
    assert_eq!(
        requests[0].prompt,
        format!(
            "{}\n- subagent: Count the files (id toolu_a)\n\n{}",
            recovery::CUT_WORK_NOTE,
            recovery::CONTINUE_PROMPT
        )
    );
}

#[test]
fn a_project_s_own_setting_decides_whether_its_threads_go_on_after_a_restart() {
    let cases = [
        (false, Some(true), true),
        (true, Some(false), false),
        (false, None, false),
    ];
    for (case, (global, own, continues)) in cases.into_iter().enumerate() {
        let name = format!("restart-project-{case}");
        let dir = scratch_dir(&name);
        let database = dir.join("state.sqlite");
        let script = vec![ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        }];
        let harness = Harness::new(&name, dir, Store::open(&database).unwrap(), script);
        harness
            .engine
            .dispatch(Command::ProjectSettingsSet {
                project_id: "p".into(),
                settings: hoshi_core::ProjectSettings {
                    continue_after_restart: own,
                },
            })
            .unwrap();
        harness.send("t", "m1", "work");
        session_started(&harness);
        drop(harness);

        let (engine, adapter, events) = reopen(&name, &database);
        engine.set_continue_after_restart(global);
        let effective = engine.continue_after_restart_in(Some("p"));
        assert_eq!(effective.value, continues, "case {case}");
        let source = match own {
            Some(_) => hoshi_core::SettingSource::Project,
            None => hoshi_core::SettingSource::Global,
        };
        assert_eq!(effective.source, source, "case {case}");
        engine.continue_cut_threads();
        if continues {
            assert!(next_turn_start(&events).is_some(), "case {case}");
            engine.snapshot();
        }
        assert_eq!(
            adapter.requests.lock().unwrap().len(),
            usize::from(continues),
            "case {case}"
        );
        if !continues {
            assert!(engine.thread_queue("t").can_resume, "case {case}");
        }
    }
}

#[test]
fn a_restart_s_cut_commands_reach_the_next_message_when_threads_do_not_go_on() {
    let dir = scratch_dir("restart-note");
    let database = dir.join("state.sqlite");
    let script = vec![ProviderOutput::SessionStarted {
        session_id: "s1".into(),
    }];
    let harness = Harness::new("restart-note", dir, Store::open(&database).unwrap(), script);
    harness.send("t", "m1", "start the dev server");
    session_started(&harness);
    lock(&harness.engine.state).receive(
        "t",
        "m1",
        ProviderOutput::BackgroundTasks {
            tasks: vec![dev_server()],
            waiting: false,
        },
    );
    drop(harness);

    let (engine, adapter, events) = reopen("restart-note", &database);
    engine.set_continue_after_restart(false);
    engine.continue_cut_threads();
    assert!(adapter.requests.lock().unwrap().is_empty());
    let send = |message_id: &str, text: &str| {
        engine
            .dispatch(Command::MessageSend {
                thread_id: "t".into(),
                message_id: message_id.into(),
                text: text.into(),
                attachments: Vec::new(),
            })
            .unwrap();
        wait_for_turn_start(&engine, &events);
    };
    send("m2", "is it up?");
    finish(&adapter, 0, TurnOutcome::Completed);
    send("m3", "and now?");

    let prompts = prompts(&adapter);
    assert_eq!(
        prompts[0],
        format!(
            "{}\n- shell: Start the dev server (id b1)\n\nUser message:\nis it up?",
            recovery::CUT_WORK_NOTE
        )
    );
    assert_eq!(prompts[1], "and now?");
}

#[test]
fn messages_queued_before_a_restart_wait_for_the_user_then_follow() {
    let dir = scratch_dir("restart-queue");
    let database = dir.join("state.sqlite");
    let harness = Harness::new(
        "restart-queue",
        dir,
        Store::open(&database).unwrap(),
        Vec::new(),
    );
    harness.engine.set_adapter(Some(adapter(None)));
    harness.send("t", "m1", "long task");
    harness.send("t", "m2", "also this");
    drop(harness);

    let (engine, adapter, events) = reopen("restart-queue", &database);
    engine.continue_cut_threads();
    let held: Vec<_> = engine
        .held_queue("t")
        .into_iter()
        .map(|message| message.message_id)
        .collect();
    assert_eq!(held, ["m2"]);
    assert!(adapter.requests.lock().unwrap().is_empty());

    engine
        .dispatch(Command::MessageSend {
            thread_id: "t".into(),
            message_id: "m3".into(),
            text: "and this".into(),
            attachments: Vec::new(),
        })
        .unwrap();
    assert_eq!(wait_for_turn_start(&engine, &events), "m3");
    finish(&adapter, 0, TurnOutcome::Completed);
    assert_eq!(wait_for_turn_start(&engine, &events), "m2");

    assert_eq!(prompts(&adapter), ["and this", "also this"]);
    assert!(engine.held_queue("t").is_empty());
}

#[test]
fn a_message_held_for_a_compaction_a_restart_cut_short_still_goes() {
    let dir = scratch_dir("restart-compaction");
    let database = dir.join("state.sqlite");
    let harness = Harness::new(
        "restart-compaction",
        dir,
        Store::open(&database).unwrap(),
        Vec::new(),
    );
    harness.engine.set_adapter(Some(adapter(None)));
    harness
        .engine
        .compact_then_send("t", "m1", "carry on".into(), Vec::new())
        .unwrap();
    drop(harness);

    let (engine, adapter, events) = reopen("restart-compaction", &database);
    engine.continue_cut_threads();
    let held = engine.snapshot().held_messages;
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].message_id, "m1");

    engine.send_held_message_now("t").unwrap();

    assert_eq!(wait_for_turn_start(&engine, &events), "m1");
    assert_eq!(prompts(&adapter), ["carry on"]);
    assert!(engine.snapshot().held_messages.is_empty());
}

fn usage_limited(resets_at: &str) -> TurnOutcome {
    TurnOutcome::Failed {
        reason: TurnFailure::UsageLimited {
            detail: "limit reached".into(),
            resets_at: Some(resets_at.into()),
        },
    }
}

#[test]
fn a_usage_limit_holds_the_queue_and_the_thread_goes_on_once_it_resets() {
    let dir = scratch_dir("usage-limit");
    let database = dir.join("state.sqlite");
    let harness = Harness::new(
        "usage-limit",
        dir,
        Store::open(&database).unwrap(),
        Vec::new(),
    );
    let first = adapter(None);
    harness.engine.set_adapter(Some(first.clone()));
    harness.send("t", "m1", "long task");
    lock(&harness.engine.state).receive(
        "t",
        "m1",
        ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        },
    );
    harness.send("t", "m2", "also this");
    let reset = Utc::now() + chrono::Duration::hours(1);
    let reset_ms = reset.timestamp_millis() as u64;
    finish(&first, 0, usage_limited(&reset.to_rfc3339()));
    harness.wait_for_turn_end();
    assert_eq!(first.requests.lock().unwrap().len(), 1);
    // The turn's end is told before its usage limit is saved, under the same lock.
    drop(lock(&harness.engine.state));
    drop(harness);

    let (engine, adapter, events) = reopen("usage-limit", &database);
    let held: Vec<_> = engine
        .held_queue("t")
        .into_iter()
        .map(|message| message.message_id)
        .collect();
    assert_eq!(held, ["m2"]);
    lock(&engine.state).resume_limited_threads(reset_ms);
    assert!(adapter.requests.lock().unwrap().is_empty());
    engine.set_resume_after_usage_limit(true);
    lock(&engine.state).resume_limited_threads(reset_ms - 1);
    assert!(adapter.requests.lock().unwrap().is_empty());
    assert!(engine.usage_limit_resume("t").is_some());

    lock(&engine.state).resume_limited_threads(reset_ms);
    wait_for_turn_start(&engine, &events);
    lock(&engine.state).resume_limited_threads(reset_ms);
    finish(&adapter, 0, TurnOutcome::Completed);
    events
        .iter()
        .find(|event| matches!(event, Event::TurnCompleted { .. }))
        .unwrap();
    assert_eq!(engine.held_queue("t").len(), 1);

    engine.resume_queue("t");
    assert_eq!(wait_for_turn_start(&engine, &events), "m2");
    assert_eq!(
        prompts(&adapter),
        [recovery::LIMIT_RESET_PROMPT, "also this"]
    );
    assert_eq!(
        adapter.requests.lock().unwrap()[0].resume.as_deref(),
        Some("s1")
    );
}

fn snooze(engine: &Engine, until: chrono::DateTime<Utc>) -> Result<(), CommandError> {
    engine.dispatch(Command::ThreadSnooze {
        thread_id: "t".into(),
        until: until.to_rfc3339(),
    })
}

#[test]
fn a_running_turn_can_be_snoozed_but_not_a_message_the_user_queued() {
    let harness = Harness::in_memory("snooze-queue", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    let later = Utc::now() + chrono::Duration::hours(1);
    harness.send("t", "m1", "long task");
    snooze(&harness.engine, later).unwrap();
    assert!(harness.thread("t").snoozed_until.is_some());

    lock(&harness.engine.state)
        .queued
        .entry("t".into())
        .or_default()
        .push_back(QueuedMessage {
            id: "news".into(),
            text: "the pull request changed".into(),
            attachments: Vec::new(),
            from_user: false,
            held: false,
            to: None,
        });
    snooze(&harness.engine, later).unwrap();

    harness.send("t", "m2", "also this");
    assert_eq!(harness.thread("t").snoozed_until, None);
    assert_eq!(
        snooze(&harness.engine, later),
        Err(CommandError::MessageQueued {
            thread_id: "t".into()
        })
    );
}

#[test]
fn a_snoozed_thread_does_not_go_on_after_a_restart() {
    let dir = scratch_dir("snooze-restart");
    let database = dir.join("state.sqlite");
    let script = vec![ProviderOutput::SessionStarted {
        session_id: "s1".into(),
    }];
    let harness = Harness::new(
        "snooze-restart",
        dir,
        Store::open(&database).unwrap(),
        script,
    );
    harness.send("t", "m1", "long task");
    session_started(&harness);
    snooze(&harness.engine, Utc::now() + chrono::Duration::hours(1)).unwrap();
    drop(harness);

    let (engine, adapter, _events) = reopen("snooze-restart", &database);
    engine.continue_cut_threads();

    assert!(adapter.requests.lock().unwrap().is_empty());
}

#[test]
fn a_snoozed_thread_does_not_compact_while_idle() {
    let harness = Harness::in_memory("snooze-compact", large_cached_turn());
    harness
        .engine
        .set_idle_compaction(IdleCompaction::BeforeCacheExpires);
    harness.send("t", "m1", "work");
    harness.wait_for_turn_end();
    let at = lock(&harness.engine.state).idle_compactions["t"];
    snooze(&harness.engine, Utc::now() + chrono::Duration::hours(1)).unwrap();

    lock(&harness.engine.state).compact_idle_threads(at);

    assert_eq!(prompts(&harness.adapter), ["work"]);
}

#[test]
fn a_usage_limit_snoozes_until_its_reset_and_waking_keeps_the_resumption() {
    let harness = Harness::in_memory("snooze-limit", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "long task");
    lock(&harness.engine.state).receive(
        "t",
        "m1",
        ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        },
    );
    let reset = Utc::now() + chrono::Duration::hours(1);
    let reset_ms = reset.timestamp_millis() as u64;
    finish(&adapter, 0, usage_limited(&reset.to_rfc3339()));
    harness.wait_for_turn_end();
    assert_eq!(harness.thread("t").snoozed_until, None);

    harness.engine.set_snooze_until_limit_resets(true);
    harness.send("t", "m2", "go on");
    finish(&adapter, 1, usage_limited(&reset.to_rfc3339()));
    harness.wait_for_turn_end();
    let thread = harness.thread("t");
    let until = chrono::DateTime::parse_from_rfc3339(thread.snoozed_until.as_deref().unwrap());
    assert_eq!(until.unwrap().timestamp_millis(), reset.timestamp_millis());
    assert!(harness.engine.usage_limit_snoozed("t"));

    harness.engine.set_resume_after_usage_limit(true);
    lock(&harness.engine.state).resume_limited_threads(reset_ms - 1);
    harness
        .engine
        .dispatch(Command::ThreadWake {
            thread_id: "t".into(),
        })
        .unwrap();
    assert_eq!(harness.thread("t").snoozed_until, None);
    assert!(harness.thread("t").last_snooze_wake_at.is_some());
    assert!(!harness.engine.usage_limit_snoozed("t"));
    assert!(harness.engine.usage_limit_resume("t").is_some());

    snooze(&harness.engine, reset + chrono::Duration::hours(1)).unwrap();
    lock(&harness.engine.state).resume_limited_threads(reset_ms);
    assert_eq!(adapter.requests.lock().unwrap().len(), 2);

    lock(&harness.engine.state).resume_limited_threads(reset_ms + 2 * 60 * 60 * 1000);
    let turn_id = harness.wait_for(|event| match event {
        Event::TurnStarted { turn_id, .. } if turn_id != "m2" => Some(turn_id),
        _ => None,
    });
    assert_eq!(prompts(&adapter)[2], recovery::LIMIT_RESET_PROMPT);
    assert_eq!(
        resumption(&harness.engine, &turn_id),
        Some(hoshi_core::Resumption::UsageLimitReset)
    );
}

#[test]
fn a_task_s_answer_waits_while_its_parent_is_held_by_a_failure() {
    let harness = Harness::in_memory("held-answer", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    delegate(&harness, serde_json::json!({ "task": "count the files" }));
    finish(&adapter, 0, usage_limited("2020-01-01T00:00:00Z"));
    harness.wait_for_turn_end();

    child_turn_ends(&harness, &adapter, 1);

    assert_eq!(adapter.requests.lock().unwrap().len(), 2);
    assert_eq!(harness.thread("t").active_turn_id, None);
    assert_eq!(harness.engine.held_queue("t").len(), 1);
    harness.send("t", "m2", "go on");
    finish(&adapter, 2, TurnOutcome::Completed);
    harness.wait_for(|event| match event {
        Event::TurnStarted { turn_id, .. } if turn_id != "m2" => Some(()),
        _ => None,
    });
    harness.thread("t");
    assert!(prompts(&adapter)[3].starts_with("Delegated task"));
}

#[test]
fn a_turn_that_fails_to_resume_its_session_keeps_it_for_a_retry() {
    let harness = Harness::in_memory("resume-failed", completed_turn());
    harness.send("t", "m1", "first");
    harness.wait_for_turn_end();
    let failing = Arc::new(FakeAdapter {
        requests: Mutex::new(Vec::new()),
        script: vec![
            ProviderOutput::SessionStarted {
                session_id: "s2".into(),
            },
            turn_failed(TurnFailure::ResumeFailed {
                detail: "no conversation found".into(),
            }),
        ],
        title: None,
        responses: Arc::default(),
        steers: None,
        finishers: Mutex::default(),
    });
    harness.engine.set_adapter(Some(failing.clone()));

    harness.send("t", "m2", "second");
    harness.wait_for_turn_end();

    assert_eq!(
        failing.requests.lock().unwrap()[0].resume.as_deref(),
        Some("s1")
    );
    assert_eq!(harness.thread("t").session_id.as_deref(), Some("s1"));
}

fn approval_requested() -> Vec<ProviderOutput> {
    vec![ProviderOutput::RequestOpened {
        request_id: "r1".into(),
        content: ItemContent::Approval {
            tool_call_id: None,
            tool_name: "Bash".into(),
            input: "{}".into(),
            reason: None,
            can_accept_for_session: true,
            status: RequestStatus::Pending,
            decision: None,
            feedback: None,
        },
    }]
}

#[test]
fn an_answer_reaches_the_turn_that_asked() {
    let harness = Harness::in_memory("answer", approval_requested());
    harness.send("t", "m1", "run the tests");
    harness.wait_for(|event| match event {
        Event::ThreadStatusSet { status, .. } => Some(status),
        _ => None,
    });
    assert_eq!(harness.thread("t").status, ThreadStatus::WaitingForInput);

    let response = RequestResponse::Decision {
        decision: ApprovalDecision::AcceptForSession,
        feedback: None,
    };
    harness
        .engine
        .dispatch(Command::RequestRespond {
            thread_id: "t".into(),
            request_id: "r1".into(),
            response: response.clone(),
        })
        .unwrap();

    assert_eq!(
        *harness.adapter.responses.lock().unwrap(),
        vec![("r1".to_owned(), response)]
    );
    assert_eq!(harness.thread("t").status, ThreadStatus::Working);
    assert!(harness.engine.snapshot().pending_requests.is_empty());
}

#[test]
fn a_request_left_open_by_a_crash_is_cancelled_on_startup() {
    let dir = scratch_dir("crash-request");
    let database = dir.join("state.sqlite");
    let harness = Harness::new(
        "crash-request",
        dir,
        Store::open(&database).unwrap(),
        approval_requested(),
    );
    harness.send("t", "m1", "run the tests");
    harness.wait_for(|event| match event {
        Event::ThreadStatusSet { .. } => Some(()),
        _ => None,
    });
    drop(harness);

    let engine = Engine::from_store(
        Store::open(&database).unwrap(),
        &scratch_dir("crash-request-reopened"),
        Arc::new(|_| {}),
    )
    .unwrap();

    assert!(engine.snapshot().pending_requests.is_empty());
    let items = engine.transcript("t", None).unwrap().items;
    let request = items.iter().find(|item| item.id == "r1").unwrap();
    assert_eq!(
        request.content.request_status(),
        Some(RequestStatus::Cancelled)
    );
    assert_eq!(engine.snapshot().threads[0].status, ThreadStatus::Idle);
}

#[test]
fn a_question_left_open_by_a_crash_is_answered_in_a_message() {
    let dir = scratch_dir("crash-question");
    let database = dir.join("state.sqlite");
    let asked = vec![ProviderOutput::RequestOpened {
        request_id: "q1".into(),
        content: ItemContent::Questions {
            questions: vec![hoshi_core::Question {
                header: "Color".into(),
                question: "Which color?".into(),
                options: Vec::new(),
                multi_select: false,
            }],
            status: RequestStatus::Pending,
            answers: None,
        },
    }];
    let harness = Harness::new(
        "crash-question",
        dir,
        Store::open(&database).unwrap(),
        asked,
    );
    harness.send("t", "m1", "paint it");
    harness.wait_for(|event| match event {
        Event::ThreadStatusSet { .. } => Some(()),
        _ => None,
    });
    drop(harness);

    let (sender, events) = mpsc::channel();
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::Event(envelope) = notification {
            let _ = sender.send(envelope.event);
        }
    });
    let data_dir = scratch_dir("crash-question-reopened");
    let engine = Engine::from_store(Store::open(&database).unwrap(), &data_dir, notify).unwrap();
    let again = adapter(None);
    engine.set_adapter(Some(again.clone()));
    assert_eq!(engine.snapshot().pending_requests.len(), 1);
    assert_eq!(engine.snapshot().threads[0].status, ThreadStatus::Idle);

    engine
        .dispatch(Command::RequestRespond {
            thread_id: "t".into(),
            request_id: "q1".into(),
            response: RequestResponse::Answers {
                answers: vec!["Blue".into()],
            },
        })
        .unwrap();
    events
        .iter()
        .find(|event| matches!(event, Event::TurnStarted { .. }))
        .unwrap();

    assert!(engine.snapshot().pending_requests.is_empty());
    assert_eq!(
        again.requests.lock().unwrap()[0].prompt,
        "Which color?
Blue"
    );
}

#[test]
fn a_whole_transcript_leaves_diffs_to_their_item() {
    let change = ToolDetail::FileChange {
        path: "/repo/a.txt".into(),
        additions: 1,
        deletions: 0,
        diff: Some("@@ -0,0 +1,1 @@\n+a".into()),
    };
    let script = vec![
        ProviderOutput::ToolCallStarted {
            item_id: "edit".into(),
            name: "Write".into(),
            input: "{}".into(),
        },
        ProviderOutput::ToolCallCompleted {
            item_id: "edit".into(),
            output: "written".into(),
            is_error: false,
            detail: Some(change.clone()),
        },
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    ];
    let harness = Harness::in_memory("diffs", script);
    harness.send("t", "m1", "write a");
    harness.wait_for_turn_end();

    let detail_of = |item: &ThreadItem| match &item.content {
        ItemContent::ToolCall { detail, .. } => detail.clone(),
        _ => None,
    };
    let items = harness.engine.transcript("t", None).unwrap().items;
    let listed = items.iter().find(|item| item.id == "edit").unwrap();
    assert_eq!(detail_of(listed), Some(change.summary()));
    let whole = harness.engine.thread_item("t", "edit").unwrap().unwrap();
    assert_eq!(detail_of(&whole), Some(change));
}

fn adapter(steers: Option<Arc<Mutex<Vec<String>>>>) -> Arc<FakeAdapter> {
    Arc::new(FakeAdapter {
        requests: Mutex::new(Vec::new()),
        script: Vec::new(),
        title: None,
        responses: Arc::default(),
        steers,
        finishers: Mutex::default(),
    })
}

fn finish(adapter: &FakeAdapter, turn: usize, outcome: TurnOutcome) {
    let finisher = adapter.finishers.lock().unwrap()[turn].clone();
    finisher.send(outcome).unwrap();
}

#[test]
fn a_message_to_a_working_thread_joins_its_turn() {
    let harness = Harness::in_memory("steer", Vec::new());
    let steers = Arc::<Mutex<Vec<String>>>::default();
    let adapter = adapter(Some(steers.clone()));
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "long task");
    harness.send("t", "m2", "also this");

    assert_eq!(*steers.lock().unwrap(), ["m2"]);
    assert_eq!(adapter.requests.lock().unwrap().len(), 1);
    finish(&adapter, 0, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    assert_eq!(adapter.requests.lock().unwrap().len(), 1);
}

#[test]
fn a_message_too_late_for_its_turn_starts_the_next_one() {
    let harness = Harness::in_memory("late-message", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "long task");
    harness.send("t", "m2", "also this");
    assert_eq!(adapter.requests.lock().unwrap().len(), 1);

    finish(&adapter, 0, TurnOutcome::Completed);
    harness.wait_for(|event| match event {
        Event::TurnStarted { turn_id, .. } if turn_id == "m2" => Some(()),
        _ => None,
    });

    assert_eq!(harness.thread("t").active_turn_id.as_deref(), Some("m2"));
    let requests = adapter.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].prompt, "also this");
}

fn stop(harness: &Harness) {
    harness
        .engine
        .dispatch(Command::TurnInterrupt {
            thread_id: "t".into(),
        })
        .unwrap();
    assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Interrupted);
    // Waits for the engine to finish reacting.
    harness.thread("t");
}

fn queue_of(harness: &Harness) -> Vec<(String, bool, hoshi_core::MessageSender)> {
    harness
        .engine
        .thread_queue("t")
        .messages
        .into_iter()
        .map(|entry| (entry.message_id, entry.held, entry.sender))
        .collect()
}

/// The next turn to start, within ten seconds.
fn turn_started(harness: &Harness) -> String {
    let turn_id = next_turn_start(&harness.events).expect("a turn to start");
    harness.thread("t");
    turn_id
}

fn next_turn_start(events: &Receiver<Event>) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        if let Event::TurnStarted { turn_id, .. } = events.recv_timeout(left).ok()? {
            return Some(turn_id);
        }
    }
}

#[test]
fn a_message_queued_before_a_stop_waits_for_the_user_s_next_one() {
    let harness = Harness::in_memory("stopped-queue", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "long task");
    harness.send("t", "m2", "also this");

    stop(&harness);

    assert_eq!(harness.thread("t").active_turn_id, None);
    assert_eq!(adapter.requests.lock().unwrap().len(), 1);
    assert_eq!(
        queue_of(&harness),
        [("m2".to_owned(), true, hoshi_core::MessageSender::User)]
    );
    assert!(harness.engine.thread_queue("t").can_resume);

    harness.send("t", "m3", "and this");
    assert_eq!(turn_started(&harness), "m3");
    assert!(!queue_of(&harness)[0].1);
    finish(&adapter, 1, TurnOutcome::Completed);
    assert_eq!(turn_started(&harness), "m2");
    assert_eq!(prompts(&adapter), ["long task", "and this", "also this"]);
}

#[test]
fn a_message_sent_while_a_turn_stops_goes_first_and_the_held_ones_follow() {
    let harness = Harness::in_memory("stopping-queue", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "long task");
    harness.send("t", "m2", "also this");
    assert!(lock(&harness.engine.state).hold_for_stop("t"));
    assert!(queue_of(&harness)[0].1);

    harness.send("t", "m3", "and this");
    assert_eq!(
        queue_of(&harness)
            .into_iter()
            .map(|(id, held, _)| (id, held))
            .collect::<Vec<_>>(),
        [("m3".to_owned(), false), ("m2".to_owned(), false)]
    );
    finish(&adapter, 0, TurnOutcome::Interrupted);
    harness.wait_for_turn_end();
    assert_eq!(turn_started(&harness), "m3");
    finish(&adapter, 1, TurnOutcome::Completed);
    assert_eq!(turn_started(&harness), "m2");
}

#[test]
fn resuming_a_stopped_thread_tells_its_agent_to_continue_then_sends_its_queue() {
    let harness = Harness::in_memory("resume-stopped", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "long task");
    harness.send("t", "m2", "also this");
    stop(&harness);

    harness.engine.resume_thread("t").unwrap();

    turn_started(&harness);
    assert!(!harness.engine.thread_queue("t").can_resume);
    finish(&adapter, 1, TurnOutcome::Completed);
    assert_eq!(turn_started(&harness), "m2");
    assert_eq!(
        prompts(&adapter),
        ["long task", recovery::LIMIT_RESET_PROMPT, "also this"]
    );
}

#[test]
fn resuming_a_thread_that_only_holds_its_queue_just_sends_it() {
    let harness = Harness::in_memory("resume-held", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "long task");
    harness.send("t", "m2", "also this");
    let failure = TurnOutcome::Failed {
        reason: TurnFailure::ProcessExited {
            detail: "exit 1".into(),
        },
    };
    finish(&adapter, 0, failure);
    harness.wait_for_turn_end();
    harness.thread("t");
    assert!(harness.engine.thread_queue("t").can_resume);

    harness.engine.resume_thread("t").unwrap();

    assert_eq!(turn_started(&harness), "m2");
    assert_eq!(prompts(&adapter), ["long task", "also this"]);
}

#[test]
fn a_queued_message_is_sent_now_only_while_the_thread_is_stopped() {
    let harness = Harness::in_memory("send-now", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "long task");
    harness.send("t", "m2", "also this");
    harness.send("t", "m3", "and this");
    assert_eq!(
        harness.engine.send_queued_now("t", "m3"),
        Err(CommandError::ThreadBusy {
            thread_id: "t".into()
        })
    );
    stop(&harness);
    assert_eq!(
        harness.engine.send_queued_now("t", "m9"),
        Err(CommandError::QueuedMessageNotFound {
            thread_id: "t".into(),
            message_id: "m9".into(),
        })
    );

    harness.engine.send_queued_now("t", "m3").unwrap();

    assert_eq!(turn_started(&harness), "m3");
    assert_eq!(
        queue_of(&harness),
        [("m2".to_owned(), false, hoshi_core::MessageSender::User)]
    );
    finish(&adapter, 1, TurnOutcome::Completed);
    assert_eq!(turn_started(&harness), "m2");
}

#[test]
fn removed_queued_messages_never_go_and_show_unread() {
    let harness = Harness::in_memory("remove-queued", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "long task");
    harness.send("t", "m2", "also this");
    harness.send("t", "m3", "and this");
    harness.send("t", "m4", "and that");
    stop(&harness);

    harness.engine.remove_queued_message("t", "m3").unwrap();
    assert_eq!(
        queue_of(&harness)
            .into_iter()
            .map(|(id, ..)| id)
            .collect::<Vec<_>>(),
        ["m2", "m4"]
    );
    harness.engine.clear_queue("t").unwrap();

    assert!(queue_of(&harness).is_empty());
    let items = harness.engine.transcript("t", None).unwrap().items;
    let dropped: Vec<&str> = items
        .iter()
        .filter(|item| matches!(item.content, ItemContent::UserMessage { dropped: true, .. }))
        .map(|item| item.id.as_str())
        .collect();
    assert_eq!(dropped, ["m2", "m3", "m4"]);
    harness.send("t", "m5", "something else");
    turn_started(&harness);
    finish(&adapter, 1, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    harness.thread("t");
    assert_eq!(prompts(&adapter), ["long task", "something else"]);
}

#[test]
fn hoshi_s_news_starts_a_stopped_thread_whose_user_messages_stay_held() {
    let harness = Harness::in_memory("stopped-news", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    let first = delegate(&harness, serde_json::json!({ "task": "count the files" }));
    let second = delegate(&harness, serde_json::json!({ "task": "count the lines" }));
    let first = first["taskId"].as_str().unwrap().to_owned();
    let second = second["taskId"].as_str().unwrap().to_owned();
    harness.send("t", "m2", "also this");
    child_turn_ends(&harness, &adapter, 1);
    let answer = queue_of(&harness)[1].clone();
    assert_eq!(answer.2, hoshi_core::MessageSender::Hoshi);

    stop(&harness);
    assert!(queue_of(&harness).iter().all(|(_, held, _)| *held));

    child_turn_ends(&harness, &adapter, 2);
    let news = turn_started(&harness);
    assert!(prompts(&adapter)[3].contains(&second));
    assert_eq!(queue_of(&harness).len(), 2);
    finish(&adapter, 3, TurnOutcome::Completed);

    assert_eq!(turn_started(&harness), answer.0);
    assert_ne!(news, answer.0);
    assert!(prompts(&adapter)[4].contains(&first));
    assert_eq!(
        queue_of(&harness),
        [("m2".to_owned(), true, hoshi_core::MessageSender::User)]
    );
    finish(&adapter, 4, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    harness.thread("t");
    assert_eq!(adapter.requests.lock().unwrap().len(), 5);
    assert!(queue_of(&harness)[0].1);
}

#[test]
fn a_turn_the_user_stopped_never_lets_hoshi_s_held_news_go() {
    let harness = Harness::in_memory("stopped-completes", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    delegate(&harness, serde_json::json!({ "task": "count the files" }));
    child_turn_ends(&harness, &adapter, 1);
    assert_eq!(queue_of(&harness).len(), 1);
    {
        let mut state = lock(&harness.engine.state);
        assert!(state.hold_for_stop("t"));
        state.receive(
            "t",
            "m1",
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed,
            },
        );
    }

    assert_eq!(harness.thread("t").active_turn_id, None);
    assert!(queue_of(&harness)[0].1);
    assert_eq!(adapter.requests.lock().unwrap().len(), 2);
}

#[test]
fn stopping_a_usage_limited_thread_turns_its_resumption_off() {
    let harness = Harness::in_memory("stop-limited", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.engine.set_resume_after_usage_limit(true);
    harness.send("t", "m1", "long task");
    lock(&harness.engine.state).receive(
        "t",
        "m1",
        ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        },
    );
    let reset = Utc::now() + chrono::Duration::hours(1);
    let reset_ms = reset.timestamp_millis() as u64;
    finish(&adapter, 0, usage_limited(&reset.to_rfc3339()));
    harness.wait_for_turn_end();
    lock(&harness.engine.state).resume_limited_threads(reset_ms - 1);
    assert!(harness.engine.usage_limit_resume("t").is_some());

    harness
        .engine
        .dispatch(Command::TurnInterrupt {
            thread_id: "t".into(),
        })
        .unwrap();

    assert_eq!(harness.engine.usage_limit_resume("t"), None);
    lock(&harness.engine.state).resume_limited_threads(reset_ms);
    assert_eq!(adapter.requests.lock().unwrap().len(), 1);
    assert!(harness.engine.thread_queue("t").can_resume);
    harness.engine.resume_thread("t").unwrap();
    turn_started(&harness);
    assert_eq!(prompts(&adapter)[1], recovery::LIMIT_RESET_PROMPT);
}

#[test]
fn the_user_chooses_whether_a_limited_thread_resumes_whatever_the_setting() {
    let harness = Harness::in_memory("limit-choice", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "long task");
    lock(&harness.engine.state).receive(
        "t",
        "m1",
        ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        },
    );
    let reset = Utc::now() + chrono::Duration::hours(1);
    let reset_ms = reset.timestamp_millis() as u64;
    finish(&adapter, 0, usage_limited(&reset.to_rfc3339()));
    harness.wait_for_turn_end();
    assert_eq!(harness.engine.usage_limit_resume("t"), None);

    assert!(harness.engine.resume_at_usage_limit_reset("t"));
    assert!(harness.engine.usage_limit_resume("t").is_some());
    harness.engine.cancel_usage_limit_resume("t");
    assert_eq!(harness.engine.usage_limit_resume("t"), None);
    harness.engine.set_resume_after_usage_limit(true);
    lock(&harness.engine.state).resume_limited_threads(reset_ms);
    assert_eq!(adapter.requests.lock().unwrap().len(), 1);

    harness.engine.set_resume_after_usage_limit(false);
    assert!(harness.engine.resume_at_usage_limit_reset("t"));
    lock(&harness.engine.state).resume_limited_threads(reset_ms);
    turn_started(&harness);
    assert_eq!(prompts(&adapter)[1], recovery::LIMIT_RESET_PROMPT);
    assert!(!harness.engine.resume_at_usage_limit_reset("t"));
}

#[test]
fn pull_request_news_kept_through_a_stop_waits_for_the_user() {
    let harness = Harness::in_memory("stopped-wake", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "long task");
    lock(&harness.engine.state).pending_wakes.insert(
        "t".into(),
        vec![Wake {
            update: hoshi_core::PullRequestUpdate::default(),
            text: "a check failed".into(),
        }],
    );

    stop(&harness);

    assert_eq!(adapter.requests.lock().unwrap().len(), 1);
    assert!(!lock(&harness.engine.state).pending_wakes.is_empty());
    harness.send("t", "m2", "go on");
    assert_eq!(turn_started(&harness), "m2");
    finish(&adapter, 1, TurnOutcome::Completed);
    turn_started(&harness);
    assert_eq!(prompts(&adapter)[2], "a check failed");
}

#[test]
fn stopping_an_idle_thread_that_waits_for_nothing_is_refused() {
    let harness = Harness::in_memory("stop-idle", Vec::new());
    assert_eq!(
        harness.engine.dispatch(Command::TurnInterrupt {
            thread_id: "t".into(),
        }),
        Err(CommandError::ThreadNotWorking {
            thread_id: "t".into()
        })
    );
}

fn delegate(harness: &Harness, arguments: serde_json::Value) -> serde_json::Value {
    lock(&harness.engine.state)
        .call_tool("t", "delegate_task", &arguments)
        .unwrap()
}

fn child_turn_ends(harness: &Harness, adapter: &FakeAdapter, turn: usize) {
    finish(adapter, turn, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    // Waits for the engine to finish reacting.
    harness.thread("t");
}

#[test]
fn a_delegated_task_runs_in_a_child_and_answers_into_the_running_turn() {
    let harness = Harness::in_memory("delegate", Vec::new());
    let steers = Arc::<Mutex<Vec<String>>>::default();
    let adapter = adapter(Some(steers.clone()));
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");

    let task = delegate(
        &harness,
        serde_json::json!({ "task": "count the files", "title": "Count" }),
    );

    let child = task["taskId"].as_str().unwrap().to_owned();
    let thread = harness.thread(&child);
    assert_eq!(thread.parent_thread_id.as_deref(), Some("t"));
    assert_eq!(thread.title, "Count");
    assert_eq!(
        adapter.requests.lock().unwrap()[1].prompt,
        "count the files"
    );
    assert!(task["next"].is_string());

    child_turn_ends(&harness, &adapter, 1);

    assert_eq!(steers.lock().unwrap().len(), 1);
    let items = harness.engine.transcript("t", None).unwrap().items;
    assert!(items.iter().any(|item| matches!(
        &item.content,
        ItemContent::UserMessage { task_result: Some(from), text, .. }
            if *from == child && text.contains("[completed]")
    )));
    assert!(harness.thread(&child).task_answered);
    assert!(lock(&harness.engine.state).pending_delegations().is_empty());
}

#[test]
fn a_delegated_task_wakes_its_parent_once_the_parent_rested() {
    let harness = Harness::in_memory("delegate-wake", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    delegate(&harness, serde_json::json!({ "task": "count the files" }));
    finish(&adapter, 0, TurnOutcome::Completed);
    harness.wait_for_turn_end();

    child_turn_ends(&harness, &adapter, 1);

    let requests = adapter.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].prompt.starts_with("Delegated task"));
    assert!(harness.thread("t").active_turn_id.is_some());
}

#[test]
fn a_task_that_wakes_for_a_command_it_left_running_answers_again() {
    let harness = Harness::in_memory("delegate-woken", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    let task = delegate(&harness, serde_json::json!({ "task": "wait a minute" }));
    let child = task["taskId"].as_str().unwrap().to_owned();
    finish(&adapter, 0, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    lock(&harness.engine.state).lingering.insert(
        child.clone(),
        Lingering {
            process: 99,
            running: Box::new(KeptTurn(Arc::default())),
            background: None,
            owes_answer: true,
            holds_answer: false,
            ended: false,
            heard_at: SystemTime::now(),
        },
    );

    child_turn_ends(&harness, &adapter, 1);
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    let first = adapter.requests.lock().unwrap()[2].prompt.clone();
    assert!(first.starts_with("Delegated task"));
    assert!(first.contains(delegation::ANSWERS_AGAIN));
    assert!(harness.thread(&child).task_answered);
    finish(&adapter, 2, TurnOutcome::Completed);
    harness.wait_for_turn_end();

    lock(&harness.engine.state).receive_from(&child, 99, ProviderOutput::Woke);
    assert!(!harness.thread(&child).task_answered);
    assert!(harness.engine.snapshot().pending_tasks.contains(&child));
    let state = &harness.engine.state;
    lock(state).receive_from(
        &child,
        99,
        ProviderOutput::AssistantMessage {
            item_id: "woken".into(),
            text: "The minute is up.".into(),
        },
    );
    lock(state).receive_from(
        &child,
        99,
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    );
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    let requests = adapter.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[3].prompt.contains("The minute is up."));
    assert!(harness.thread(&child).task_answered);
}

/// Thread `t` delegated a task whose turn ended while its agent awaits a command in process
/// 99, holding back its answer.
fn task_awaits_a_command(name: &str) -> (Harness, Arc<FakeAdapter>, String) {
    let harness = Harness::in_memory(name, Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    let task = delegate(&harness, serde_json::json!({ "task": "run the tests" }));
    let child = task["taskId"].as_str().unwrap().to_owned();
    finish(&adapter, 0, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    lock(&harness.engine.state).lingering.insert(
        child.clone(),
        Lingering {
            process: 99,
            running: Box::new(KeptTurn(Arc::default())),
            background: Some(ThreadBackgroundTasks {
                thread_id: child.clone(),
                tasks: vec![BackgroundTask {
                    task_id: "b1".into(),
                    tool_call_id: None,
                    description: "Run the tests".into(),
                    started_at: 0,
                }],
                waiting: true,
            }),
            owes_answer: true,
            holds_answer: false,
            ended: false,
            heard_at: SystemTime::now(),
        },
    );
    child_turn_ends(&harness, &adapter, 1);
    (harness, adapter, child)
}

#[test]
fn a_task_that_awaits_its_commands_answers_once_it_wakes_for_them() {
    let (harness, adapter, child) = task_awaits_a_command("delegate-awaits");
    assert_eq!(adapter.requests.lock().unwrap().len(), 2);
    assert!(!harness.thread(&child).task_answered);
    assert!(harness.engine.snapshot().pending_tasks.contains(&child));
    let status = lock(&harness.engine.state)
        .call_tool("t", "task_status", &serde_json::json!({ "taskId": child }))
        .unwrap();
    assert_eq!(status["status"], "running");
    assert!(status.get("answer").is_none(), "{status}");

    let state = &harness.engine.state;
    lock(state).receive_from(&child, 99, ProviderOutput::Woke);
    lock(state).receive_from(
        &child,
        99,
        ProviderOutput::AssistantMessage {
            item_id: "woken".into(),
            text: "The tests pass.".into(),
        },
    );
    lock(state).receive_from(
        &child,
        99,
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    );
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    let requests = adapter.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].prompt.contains("The tests pass."));
    assert!(harness.thread(&child).task_answered);
}

#[test]
fn a_task_whose_process_ends_before_it_wakes_answers_as_stopped() {
    let (harness, adapter, child) = task_awaits_a_command("delegate-awaits-ends");
    let state = &harness.engine.state;
    let server = BackgroundTask {
        task_id: "s1".into(),
        tool_call_id: None,
        description: "Start the dev server".into(),
        started_at: 0,
    };
    lock(state).receive_from(
        &child,
        99,
        ProviderOutput::BackgroundTasks {
            tasks: vec![server],
            waiting: false,
        },
    );
    lock(state).receive_from(&child, 99, ProviderOutput::ProcessEnded);

    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    let requests = adapter.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[2].prompt.contains("[stopped]"),
        "{}",
        requests[2].prompt
    );
    assert!(harness.thread(&child).task_answered);
}

/// Thread `t` delegated a task whose turn said `answer` and ended leaving a command
/// running in process 99, its answer owed to `t` when `owes_answer`.
fn task_left_a_command_running(
    name: &str,
    answer: &str,
    owes_answer: bool,
) -> (Harness, Arc<FakeAdapter>, String) {
    let harness = Harness::in_memory(name, Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    let task = delegate(&harness, serde_json::json!({ "task": "wait a minute" }));
    let child = task["taskId"].as_str().unwrap().to_owned();
    finish(&adapter, 0, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    let process = lock(&harness.engine.state).turns[&child].process;
    lock(&harness.engine.state).receive_from(
        &child,
        process,
        ProviderOutput::AssistantMessage {
            item_id: "first".into(),
            text: answer.into(),
        },
    );
    lock(&harness.engine.state).lingering.insert(
        child.clone(),
        Lingering {
            process: 99,
            running: Box::new(KeptTurn(Arc::default())),
            background: None,
            owes_answer,
            holds_answer: false,
            ended: false,
            heard_at: SystemTime::now(),
        },
    );
    child_turn_ends(&harness, &adapter, 1);
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    finish(&adapter, 2, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    (harness, adapter, child)
}

fn woken_turn_ends(harness: &Harness, child: &str) {
    let state = &harness.engine.state;
    lock(state).receive_from(child, 99, ProviderOutput::Woke);
    lock(state).receive_from(
        child,
        99,
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    );
    harness.wait_for_turn_end();
    harness.thread("t");
}

#[test]
fn a_task_that_wakes_without_saying_anything_new_leaves_its_parent_alone() {
    let (harness, adapter, child) =
        task_left_a_command_running("delegate-woken-silent", "Started the wait.", true);

    woken_turn_ends(&harness, &child);

    assert_eq!(adapter.requests.lock().unwrap().len(), 3);
    assert!(harness.thread(&child).task_answered);
    assert!(lock(&harness.engine.state).delegations.is_empty());
}

#[test]
fn a_task_woken_by_what_the_user_left_running_doesnt_answer_its_parent() {
    let (harness, adapter, child) =
        task_left_a_command_running("delegate-woken-user", "Started the tests.", false);

    lock(&harness.engine.state).receive_from(&child, 99, ProviderOutput::Woke);

    assert!(harness.thread(&child).task_answered);
    assert!(lock(&harness.engine.state).delegations.is_empty());
    assert_eq!(adapter.requests.lock().unwrap().len(), 3);
}

fn says(harness: &Harness, thread_id: &str, item_id: &str, text: &str) {
    let process = lock(&harness.engine.state).turns[thread_id].process;
    lock(&harness.engine.state).receive_from(
        thread_id,
        process,
        ProviderOutput::AssistantMessage {
            item_id: item_id.into(),
            text: text.into(),
        },
    );
}

/// Thread `t` delegated a task, which delegated one of its own; `t`'s turn ended.
fn task_with_a_task_of_its_own(name: &str) -> (Harness, Arc<FakeAdapter>, String, String) {
    let harness = Harness::in_memory(name, Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    let task = delegate(&harness, serde_json::json!({ "task": "delegate the wait" }));
    let child = task["taskId"].as_str().unwrap().to_owned();
    finish(&adapter, 0, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    let own = lock(&harness.engine.state)
        .call_tool(
            &child,
            "delegate_task",
            &serde_json::json!({ "task": "wait" }),
        )
        .unwrap();
    let grandchild = own["taskId"].as_str().unwrap().to_owned();
    (harness, adapter, child, grandchild)
}

#[test]
fn a_task_waiting_on_a_task_of_its_own_says_so_and_answers_again() {
    let (harness, adapter, child, grandchild) = task_with_a_task_of_its_own("delegate-nested");
    let turn_started = |event: Event| matches!(event, Event::TurnStarted { .. }).then_some(());

    says(&harness, &child, "first", "Started my task.");
    child_turn_ends(&harness, &adapter, 1);
    harness.wait_for(turn_started);
    let first = adapter.requests.lock().unwrap()[3].prompt.clone();
    assert!(first.contains("Started my task."));
    assert!(first.contains(delegation::ANSWERS_AGAIN));
    assert!(harness.thread(&child).task_answered);
    finish(&adapter, 3, TurnOutcome::Completed);
    harness.wait_for_turn_end();

    says(&harness, &grandchild, "waited", "Waited.");
    child_turn_ends(&harness, &adapter, 2);
    harness.wait_for(turn_started);
    assert!(!harness.thread(&child).task_answered);
    assert!(
        adapter.requests.lock().unwrap()[4]
            .prompt
            .contains("Waited.")
    );

    says(&harness, &child, "second", "My task said: Waited.");
    child_turn_ends(&harness, &adapter, 4);
    harness.wait_for(turn_started);
    let last = adapter.requests.lock().unwrap()[5].prompt.clone();
    assert!(last.contains("My task said: Waited."));
    assert!(last.contains(delegation::ALL_ANSWERED));
    assert!(harness.thread(&child).task_answered);
}

#[test]
fn deleting_a_thread_deletes_the_tasks_it_delegated_however_deep() {
    let (harness, adapter, child, grandchild) =
        task_with_a_task_of_its_own("delegate-nested-delete");
    says(&harness, &child, "first", "Started my task.");
    child_turn_ends(&harness, &adapter, 1);

    harness
        .engine
        .dispatch(Command::ThreadDelete {
            thread_id: "t".into(),
        })
        .unwrap();

    let left: Vec<String> = harness
        .engine
        .snapshot()
        .threads
        .into_iter()
        .map(|thread| thread.id)
        .collect();
    assert!(!left.contains(&"t".to_owned()));
    assert!(!left.contains(&child));
    assert!(!left.contains(&grandchild));
    assert!(lock(&harness.engine.state).delegations.is_empty());
}

#[test]
fn cancelling_a_task_cancels_the_tasks_of_its_own() {
    let (harness, adapter, child, grandchild) =
        task_with_a_task_of_its_own("delegate-nested-cancel");
    says(&harness, &child, "first", "Started my task.");
    child_turn_ends(&harness, &adapter, 1);

    assert!(lock(&harness.engine.state).cancel_task(&child));

    assert!(harness.thread(&child).task_cancelled);
    assert!(harness.thread(&grandchild).task_cancelled);
    assert!(lock(&harness.engine.state).delegations.is_empty());
}

#[test]
fn a_task_a_restart_cut_short_is_awaited_again_only_once_it_runs() {
    let harness = Harness::in_memory("delegate-restored", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    let task = delegate(&harness, serde_json::json!({ "task": "count the files" }));
    let child = task["taskId"].as_str().unwrap().to_owned();
    assert!(harness.engine.snapshot().pending_tasks.contains(&child));

    let restored = lock(&harness.engine.state).pending_delegations();
    lock(&harness.engine.state).delegations = restored;
    assert!(harness.engine.snapshot().pending_tasks.contains(&child));

    lock(&harness.engine.state).turns.remove(&child);
    assert!(harness.engine.snapshot().pending_tasks.is_empty());
    assert!(lock(&harness.engine.state).delegations.contains_key(&child));
}

#[test]
fn cancelling_a_task_that_answered_stops_the_commands_it_left_running() {
    let harness = Harness::in_memory("delegate-cancel-kept", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    let task = delegate(&harness, serde_json::json!({ "task": "wait a minute" }));
    let child = task["taskId"].as_str().unwrap().to_owned();
    let process = Arc::new(KeptProcess::default());
    lock(&harness.engine.state).lingering.insert(
        child.clone(),
        Lingering {
            process: 99,
            running: Box::new(KeptTurn(process.clone())),
            background: None,
            owes_answer: true,
            holds_answer: false,
            ended: false,
            heard_at: SystemTime::now(),
        },
    );
    child_turn_ends(&harness, &adapter, 1);

    assert!(lock(&harness.engine.state).cancel_task(&child));

    assert!(*process.closed.lock().unwrap());
    assert!(!lock(&harness.engine.state).lingering.contains_key(&child));
    lock(&harness.engine.state).receive_from(&child, 99, ProviderOutput::Woke);
    assert!(harness.thread(&child).task_answered);
    assert!(harness.thread(&child).task_cancelled);
}

#[test]
fn a_cancelled_task_never_answers_and_only_its_parent_reaches_it() {
    let harness = Harness::in_memory("delegate-cancel", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    let task = delegate(&harness, serde_json::json!({ "task": "count the files" }));
    let task_id = serde_json::json!({ "taskId": task["taskId"] });

    let stranger = lock(&harness.engine.state).call_tool("p", "task_status", &task_id);
    assert!(stranger.is_err());
    let running = lock(&harness.engine.state)
        .call_tool("t", "task_status", &task_id)
        .unwrap();
    assert_eq!(running["status"], "running");

    let cancelled = lock(&harness.engine.state)
        .call_tool("t", "task_cancel", &task_id)
        .unwrap();
    assert_eq!(cancelled["status"], "cancel_requested");
    harness.wait_for_turn_end();
    finish(&adapter, 0, TurnOutcome::Completed);
    harness.wait_for_turn_end();

    assert_eq!(adapter.requests.lock().unwrap().len(), 2);
    assert_eq!(harness.thread("t").active_turn_id, None);
    let child = task["taskId"].as_str().unwrap();
    assert!(harness.thread(child).task_cancelled);
    assert!(lock(&harness.engine.state).pending_delegations().is_empty());
}

#[test]
fn the_user_cancels_a_task_waiting_on_its_plan_and_it_never_answers() {
    let harness = Harness::in_memory("delegate-plan-cancel", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    let child = planning_task(&harness, &adapter);

    harness.engine.cancel_task(&child);
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    let told = adapter.requests.lock().unwrap()[2].prompt.clone();
    assert!(told.contains("[stopped]") && told.contains(delegation::STOPPED_BY_USER));
    assert!(told.contains(delegation::ALL_ANSWERED));

    harness.send(&child, "m5", "one more thing");
    child_turn_ends(&harness, &adapter, 3);

    assert!(harness.thread(&child).task_cancelled);
    assert!(harness.engine.snapshot().pending_tasks.is_empty());
    assert_eq!(adapter.requests.lock().unwrap().len(), 4);
}

#[test]
fn a_task_delegates_once_more_with_the_model_effort_and_mode_it_was_given() {
    let harness = Harness::in_memory("delegate-nested-settings", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");

    let task = delegate(
        &harness,
        serde_json::json!({ "task": "plan it", "model": "opus", "effort": "high", "plan": true }),
    );
    let child = task["taskId"].as_str().unwrap().to_owned();
    let thread = harness.thread(&child);
    assert_eq!(thread.model.as_deref(), Some("opus"));
    assert_eq!(thread.effort, Some(Effort::High));
    assert!(thread.plan_mode);

    let deeper = lock(&harness.engine.state)
        .call_tool(
            &child,
            "delegate_task",
            &serde_json::json!({ "task": "count" }),
        )
        .unwrap();
    let grandchild = deeper["taskId"].as_str().unwrap().to_owned();
    {
        let state = lock(&harness.engine.state);
        assert!(state.offers_tool("t", "watch_pull_request"));
        assert!(!state.offers_tool(&child, "watch_pull_request"));
        assert!(!state.offers_tool(&child, "unwatch_pull_request"));
    }
    assert_eq!(
        harness.thread(&grandchild).parent_thread_id.as_deref(),
        Some(child.as_str())
    );
    let too_deep = lock(&harness.engine.state).call_tool(
        &grandchild,
        "delegate_task",
        &serde_json::json!({ "task": "more" }),
    );
    assert!(too_deep.is_err());
    let wrong = lock(&harness.engine.state).call_tool(
        "t",
        "delegate_task",
        &serde_json::json!({ "task": "x", "effort": "huge" }),
    );
    assert!(wrong.is_err());

    let requests = adapter.requests.lock().unwrap();
    let told = |turn: usize| requests[turn].instructions.clone();
    assert!(told(0).is_some_and(|told| !told.contains(delegation::TASK_INSTRUCTIONS)));
    assert!(told(1).is_some_and(|told| told.contains(delegation::TASK_INSTRUCTIONS)));
    assert_eq!(told(2), None);
}

#[test]
fn a_task_runs_on_the_enabled_provider_it_names() {
    let harness = Harness::in_memory("delegate-provider", Vec::new());
    let (default, other) = (adapter(None), adapter(None));
    harness.engine.set_adapters(vec![
        default.clone(),
        Arc::new(OtherProvider(other.clone())),
    ]);
    harness.send("t", "m1", "split the work");
    let call = |arguments: serde_json::Value| {
        lock(&harness.engine.state).call_tool("t", "delegate_task", &arguments)
    };

    let unknown = call(serde_json::json!({ "task": "x", "provider": "gemini" })).unwrap_err();
    assert!(unknown.contains("claudeCode, copilot"), "{unknown}");
    let disabled = call(serde_json::json!({ "task": "x", "provider": "codex" })).unwrap_err();
    assert!(
        disabled.contains("codex is not an enabled provider"),
        "{disabled}"
    );
    let unread =
        call(serde_json::json!({ "task": "x", "provider": OTHER_PROVIDER, "model": "gpt" }));
    assert!(unread.unwrap_err().contains("can't be read"));

    let task =
        call(serde_json::json!({ "task": "review it", "provider": OTHER_PROVIDER })).unwrap();
    let child = harness.thread(task["taskId"].as_str().unwrap());
    assert_eq!(child.provider, OTHER_PROVIDER);
    assert_eq!(child.model, None);
    assert_eq!(default.requests.lock().unwrap().len(), 1);
    assert_eq!(other.requests.lock().unwrap()[0].prompt, "review it");
}

#[test]
fn turns_carry_how_the_agent_chooses_its_subagents_effort() {
    let harness = Harness::in_memory("subagent-effort", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "look it up");
    finish(&adapter, 0, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    harness.engine.set_subagent_effort(SubagentEffort::Thread);
    harness.send("t", "m2", "again");

    let requests = adapter.requests.lock().unwrap();
    assert_eq!(requests[0].subagent_effort, SubagentEffort::AgentDecides);
    assert_eq!(requests[1].subagent_effort, SubagentEffort::Thread);
}

/// A task whose plan the user sent back to planning, its turn over, and its parent at rest.
fn planning_task(harness: &Harness, adapter: &FakeAdapter) -> String {
    harness.send("t", "m1", "split the work");
    let task = delegate(
        harness,
        serde_json::json!({ "task": "plan the README", "plan": true }),
    );
    let child = task["taskId"].as_str().unwrap().to_owned();
    finish(adapter, 0, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    let declined = Event::ItemAdded {
        item: ThreadItem {
            id: "plan1".into(),
            thread_id: child.clone(),
            created_at: "2026-11-08T10:00:00Z".into(),
            content: ItemContent::ProposedPlan {
                plan: "1. Sort the sections".into(),
                status: RequestStatus::Resolved,
                decision: Some(ApprovalDecision::Decline),
                feedback: None,
            },
        },
    };
    lock(&harness.engine.state)
        .commit(&[declined], "2026-11-08T10:00:00Z")
        .unwrap();
    child_turn_ends(harness, adapter, 1);
    child
}

#[test]
fn a_task_still_planning_tells_its_status_and_answers_as_is_when_the_user_says() {
    let harness = Harness::in_memory("delegate-plan-as-is", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    let child = planning_task(&harness, &adapter);

    let status = lock(&harness.engine.state)
        .call_tool("t", "task_status", &serde_json::json!({ "taskId": child }))
        .unwrap();
    assert_eq!(status["status"], "waiting_for_user");
    assert!(harness.engine.snapshot().pending_tasks.contains(&child));

    harness.engine.send_task_answer(&child);
    harness.thread("t");

    let requests = adapter.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].prompt.contains("1. Sort the sections"));
    assert!(harness.engine.snapshot().pending_tasks.is_empty());
}

#[test]
fn a_task_whose_plan_the_user_sent_back_answers_only_once_it_ends_otherwise() {
    let harness = Harness::in_memory("delegate-plan", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    let task = delegate(
        &harness,
        serde_json::json!({ "task": "plan the README", "plan": true }),
    );
    let child = task["taskId"].as_str().unwrap().to_owned();
    finish(&adapter, 0, TurnOutcome::Completed);
    harness.wait_for_turn_end();
    let declined = Event::ItemAdded {
        item: ThreadItem {
            id: "plan1".into(),
            thread_id: child.clone(),
            created_at: "2026-11-08T10:00:00Z".into(),
            content: ItemContent::ProposedPlan {
                plan: "1. Sort the sections".into(),
                status: RequestStatus::Resolved,
                decision: Some(ApprovalDecision::Decline),
                feedback: None,
            },
        },
    };
    lock(&harness.engine.state)
        .commit(&[declined], "2026-11-08T10:00:00Z")
        .unwrap();

    child_turn_ends(&harness, &adapter, 1);
    assert_eq!(adapter.requests.lock().unwrap().len(), 2);

    harness.send(&child, "m5", "keep it as it is");
    child_turn_ends(&harness, &adapter, 2);

    let requests = adapter.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[3].prompt.starts_with("Delegated task"));
    assert!(requests[3].prompt.contains("1. Sort the sections"));
}

#[test]
fn a_task_a_restart_cut_short_answers_once_the_user_has_it_finish() {
    let store_dir = scratch_dir("delegate-restart-store");
    let database = store_dir.join("state.sqlite");
    let first = adapter(None);
    let child = {
        let harness = Harness::new(
            "delegate-restart",
            scratch_dir("delegate-restart"),
            Store::open(&database).unwrap(),
            Vec::new(),
        );
        harness.engine.set_adapter(Some(first.clone()));
        harness.send("t", "m1", "split the work");
        let task = delegate(&harness, serde_json::json!({ "task": "count the files" }));
        finish(&first, 0, TurnOutcome::Completed);
        harness.wait_for_turn_end();
        task["taskId"].as_str().unwrap().to_owned()
    };

    let (sender, events) = mpsc::channel();
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::Event(envelope) = notification {
            let _ = sender.send(envelope.event);
        }
    });
    let data_dir = scratch_dir("delegate-restart-again");
    let engine = Engine::from_store(Store::open(&database).unwrap(), &data_dir, notify).unwrap();
    let again = adapter(None);
    engine.set_adapter(Some(again.clone()));
    let harness = Harness {
        engine,
        adapter: again.clone(),
        events,
        root: String::new(),
        data_dir,
    };
    assert_eq!(harness.thread(&child).active_turn_id, None);

    harness.send(&child, "m9", "go on");
    finish(&again, 0, TurnOutcome::Completed);
    harness.wait_for(|event| match event {
        Event::TurnCompleted { turn_id, .. } if turn_id == "m9" => Some(()),
        _ => None,
    });
    harness.thread("t");

    let requests = again.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].prompt.starts_with("Delegated task"));
    drop(first);
}

#[test]
fn rejects_a_folder_that_does_not_exist() {
    let harness = Harness::in_memory("missing", Vec::new());
    let missing = std::env::temp_dir().join("hoshi-definitely-missing-folder");

    let error = harness
        .engine
        .dispatch(Command::ProjectCreate {
            project_id: "q".into(),
            title: "q".into(),
            workspace_root: missing.to_string_lossy().into_owned(),
        })
        .unwrap_err();

    assert!(matches!(error, CommandError::WorkspaceRootNotFound { .. }));
}

#[test]
fn a_new_worktree_runs_its_project_s_setup_before_the_held_message() {
    let repo = git::tests::temp_repository("engine-setup");
    let harness = Harness::new(
        "setup",
        repo,
        Store::open_in_memory().unwrap(),
        completed_turn(),
    );
    let marker = if cfg!(windows) {
        "Set-Content -Path ready.txt -Value ok"
    } else {
        "echo ok > ready.txt"
    };
    harness
        .engine
        .dispatch(Command::ProjectWorktreeSetupSet {
            project_id: "p".into(),
            setup: Some(hoshi_core::WorktreeSetup {
                files: Vec::new(),
                command: format!("  {marker}  "),
                wait: true,
            }),
        })
        .unwrap();
    harness
        .create_thread("s1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    harness.send("s1234567-rest", "m1", "go");

    harness.wait_for_turn_end();
    let worktree = harness.thread("s1234567-rest").worktree_path.unwrap();
    assert!(Path::new(&worktree).join("ready.txt").is_file());
    let runs = harness.engine.worktree_setups();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, hoshi_core::WorktreeSetupStatus::Succeeded);
    assert_eq!(runs[0].held_messages, 0);
    assert_eq!(harness.adapter.requests.lock().unwrap().len(), 1);
}

#[test]
fn a_failed_setup_keeps_the_message_until_the_user_sends_it_anyway() {
    let repo = git::tests::temp_repository("engine-setup-failed");
    let harness = Harness::new(
        "setup-failed",
        repo,
        Store::open_in_memory().unwrap(),
        completed_turn(),
    );
    harness
        .engine
        .dispatch(Command::ProjectWorktreeSetupSet {
            project_id: "p".into(),
            setup: Some(hoshi_core::WorktreeSetup {
                files: Vec::new(),
                command: "exit 3".into(),
                wait: true,
            }),
        })
        .unwrap();
    harness
        .create_thread("f1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    harness.send("f1234567-rest", "m1", "go");
    let deadline = Instant::now() + Duration::from_secs(30);
    let failed = loop {
        let runs = harness.engine.worktree_setups();
        if runs[0].status != hoshi_core::WorktreeSetupStatus::Running {
            break runs[0].clone();
        }
        assert!(Instant::now() < deadline, "the setup never ended");
        thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(failed.status, hoshi_core::WorktreeSetupStatus::Failed);
    assert_eq!(failed.exit_code, Some(3));
    assert_eq!(failed.held_messages, 1);
    assert!(harness.adapter.requests.lock().unwrap().is_empty());

    harness.engine.skip_worktree_setup("f1234567-rest").unwrap();
    harness.wait_for_turn_end();
    assert_eq!(harness.adapter.requests.lock().unwrap().len(), 1);
}

fn wait_for_setup_end(harness: &Harness) -> hoshi_core::WorktreeSetupRun {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let runs = harness.engine.worktree_setups();
        if let Some(run) = runs
            .into_iter()
            .find(|run| run.status != hoshi_core::WorktreeSetupStatus::Running)
        {
            return run;
        }
        assert!(Instant::now() < deadline, "the setup never ended");
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_new_worktree_gets_the_project_s_files_before_its_command_runs() {
    let repo = git::tests::temp_repository("engine-setup-files");
    std::fs::write(repo.join(".env"), "KEY=1").unwrap();
    std::fs::create_dir_all(repo.join("secrets")).unwrap();
    std::fs::write(repo.join("secrets/a.env"), "a").unwrap();
    std::fs::write(repo.join("secrets/b.env"), "b").unwrap();
    std::fs::write(repo.join("secrets/notes.txt"), "n").unwrap();
    let harness = Harness::new(
        "setup-files",
        repo,
        Store::open_in_memory().unwrap(),
        completed_turn(),
    );
    let check = if cfg!(windows) {
        "if (Test-Path secrets/b.env) { Copy-Item .env ready.txt } else { exit 4 }"
    } else {
        "test -f secrets/b.env && cp .env ready.txt"
    };
    harness
        .engine
        .dispatch(Command::ProjectWorktreeSetupSet {
            project_id: "p".into(),
            setup: Some(hoshi_core::WorktreeSetup {
                files: vec![
                    ".env".into(),
                    "secrets/*.env".into(),
                    "local.properties".into(),
                ],
                command: check.into(),
                wait: true,
            }),
        })
        .unwrap();
    harness
        .create_thread("c1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    harness.send("c1234567-rest", "m1", "go");

    harness.wait_for_turn_end();
    let worktree = PathBuf::from(harness.thread("c1234567-rest").worktree_path.unwrap());
    assert_eq!(
        std::fs::read_to_string(worktree.join("ready.txt"))
            .unwrap()
            .trim(),
        "KEY=1"
    );
    assert!(worktree.join("secrets/a.env").is_file());
    assert!(!worktree.join("secrets/notes.txt").exists());
    let run = wait_for_setup_end(&harness);
    assert_eq!(run.status, hoshi_core::WorktreeSetupStatus::Succeeded);
    assert_eq!(run.copied_files, 3);
    assert_eq!(run.missing_files, ["local.properties"]);
    assert!(run.failed_files.is_empty());
    assert!(run.terminal_id.is_some());
}

#[test]
fn a_setup_of_files_only_releases_the_held_message_once_copied() {
    let repo = git::tests::temp_repository("engine-setup-files-only");
    std::fs::write(repo.join(".env"), "KEY=1").unwrap();
    let harness = Harness::new(
        "setup-files-only",
        repo,
        Store::open_in_memory().unwrap(),
        completed_turn(),
    );
    harness
        .engine
        .dispatch(Command::ProjectWorktreeSetupSet {
            project_id: "p".into(),
            setup: Some(hoshi_core::WorktreeSetup {
                files: vec!["missing/*.env".into(), ".env".into()],
                command: " ".into(),
                wait: true,
            }),
        })
        .unwrap();
    harness
        .create_thread("o1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    harness.send("o1234567-rest", "m1", "go");

    harness.wait_for_turn_end();
    let worktree = PathBuf::from(harness.thread("o1234567-rest").worktree_path.unwrap());
    assert!(worktree.join(".env").is_file());
    let run = wait_for_setup_end(&harness);
    assert_eq!(run.status, hoshi_core::WorktreeSetupStatus::Succeeded);
    assert_eq!(run.terminal_id, None);
    assert_eq!(run.exit_code, None);
    assert_eq!(run.copied_files, 1);
    assert_eq!(run.missing_files, ["missing/*.env"]);
    assert_eq!(run.held_messages, 0);
    assert_eq!(harness.adapter.requests.lock().unwrap().len(), 1);
}

#[test]
fn a_setup_refuses_files_outside_the_project() {
    let repo = git::tests::temp_repository("engine-setup-escape");
    let harness = Harness::new(
        "setup-escape",
        repo,
        Store::open_in_memory().unwrap(),
        completed_turn(),
    );

    let error = harness
        .engine
        .dispatch(Command::ProjectWorktreeSetupSet {
            project_id: "p".into(),
            setup: Some(hoshi_core::WorktreeSetup {
                files: vec![".env".into(), "../outside/.env".into()],
                command: String::new(),
                wait: false,
            }),
        })
        .unwrap_err();

    assert_eq!(
        error,
        CommandError::SetupFileInvalid {
            path: "../outside/.env".into()
        }
    );
    assert_eq!(harness.engine.snapshot().projects[0].worktree_setup, None);
}

#[test]
fn a_new_worktree_s_branch_keeps_its_base_and_takes_the_title_s_name() {
    let repo = git::tests::temp_repository("engine-branch-name");
    let harness = Harness::new(
        "branch-name",
        repo,
        Store::open_in_memory().unwrap(),
        completed_turn(),
    );
    harness.engine.set_adapter(Some(Arc::new(FakeAdapter {
        requests: Mutex::new(Vec::new()),
        script: completed_turn(),
        title: Some("Fix the login bug".into()),
        responses: Arc::default(),
        steers: None,
        finishers: Mutex::default(),
    })));
    harness
        .create_thread("n1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    harness
        .create_thread("p1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    let pushed = harness.thread("p1234567-rest");
    git::set_branch_config(&harness.root, "hoshi/p1234567", "remote", "origin").unwrap();
    assert_eq!(
        git::branch_config(&harness.root, "hoshi/n1234567", git::BASE_CONFIG).as_deref(),
        Some("main")
    );

    harness.send("n1234567-rest", "m1", "the login fails");
    let renamed = harness.wait_for(|event| match event {
        Event::ThreadBranchSet { thread_id, branch } if thread_id == "n1234567-rest" => branch,
        _ => None,
    });
    harness.send("p1234567-rest", "m2", "the login fails");
    harness.wait_for(|event| match event {
        Event::ThreadTitleSet { thread_id, title }
            if thread_id == "p1234567-rest" && title == "Fix the login bug" =>
        {
            Some(())
        }
        _ => None,
    });

    assert_eq!(renamed, "hoshi/fix-the-login-bug");
    let worktree = harness.thread("n1234567-rest").worktree_path.unwrap();
    assert_eq!(
        git::branch_config(&worktree, "hoshi/fix-the-login-bug", git::BASE_CONFIG).as_deref(),
        Some("main")
    );
    let pushed_path = pushed.worktree_path.unwrap();
    thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        git::current_branch(&pushed_path).as_deref(),
        Some("hoshi/p1234567")
    );
    for path in [worktree, pushed_path] {
        let _ = git::remove_worktree(&harness.root, &path);
    }
}

#[test]
fn a_message_sent_after_compacting_waits_for_the_compaction_turn() {
    let mut script = completed_turn();
    script.insert(
        1,
        ProviderOutput::ContextUsage(ContextUsage {
            used_tokens: 250_000,
            max_tokens: Some(1_000_000),
            cache_ttl_secs: Some(3600),
            cache_expires_at: None,
        }),
    );
    let harness = Harness::in_memory("compact", script);

    harness
        .engine
        .compact_then_send("t", "m1", "carry on".into(), Vec::new())
        .unwrap();
    harness.wait_for_turn_end();
    harness.wait_for_turn_end();

    let prompts: Vec<_> = harness
        .adapter
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.prompt.clone())
        .collect();
    assert_eq!(prompts, ["/compact", "carry on"]);
    let usage = lock(&harness.engine.state).context_usages["t"];
    let now = Utc::now().timestamp_millis() as u64;
    let expires = usage.cache_expires_at.unwrap();
    assert!(expires > now + 3_500_000 && expires <= now + 3_600_000);
}

fn large_cached_turn() -> Vec<ProviderOutput> {
    let mut script = completed_turn();
    script.insert(
        1,
        ProviderOutput::ContextUsage(ContextUsage {
            used_tokens: 250_000,
            max_tokens: Some(1_000_000),
            cache_ttl_secs: Some(3600),
            cache_expires_at: None,
        }),
    );
    script
}

fn prompts(adapter: &FakeAdapter) -> Vec<String> {
    adapter
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.prompt.clone())
        .collect()
}

#[test]
fn an_idle_thread_compacts_once_shortly_before_its_cache_expires() {
    let harness = Harness::in_memory("idle-compact", large_cached_turn());
    harness
        .engine
        .set_idle_compaction(IdleCompaction::BeforeCacheExpires);
    harness.send("t", "m1", "work");
    harness.wait_for_turn_end();

    let (at, expires) = {
        let state = lock(&harness.engine.state);
        (
            state.idle_compactions["t"],
            state.context_usages["t"].cache_expires_at.unwrap(),
        )
    };
    assert_eq!(at, expires - hoshi_core::IDLE_COMPACTION_LEAD_MS);
    lock(&harness.engine.state).compact_idle_threads(at - 1);
    assert_eq!(prompts(&harness.adapter), ["work"]);

    lock(&harness.engine.state).compact_idle_threads(at);
    harness.wait_for_turn_end();

    assert_eq!(prompts(&harness.adapter), ["work", "/compact"]);
    assert!(lock(&harness.engine.state).idle_compactions.is_empty());
}

#[test]
fn an_idle_thread_waits_for_the_user_once_its_cache_ran_out_or_it_moved_on() {
    let harness = Harness::in_memory("idle-compact-late", large_cached_turn());
    harness
        .engine
        .set_idle_compaction(IdleCompaction::BeforeCacheExpires);
    harness.send("t", "m1", "work");
    harness.wait_for_turn_end();
    let expires = lock(&harness.engine.state).context_usages["t"]
        .cache_expires_at
        .unwrap();

    lock(&harness.engine.state).compact_idle_threads(expires);
    harness.send("t", "m2", "more");
    harness.wait_for_turn_end();
    harness
        .engine
        .set_idle_compaction(IdleCompaction::WhenYouReturn);
    harness.send("t", "m3", "again");
    harness.wait_for_turn_end();

    assert_eq!(prompts(&harness.adapter), ["work", "more", "again"]);
    assert!(lock(&harness.engine.state).idle_compactions.is_empty());
}

#[test]
fn a_message_held_while_compacting_shows_and_can_be_taken_back() {
    let harness = Harness::in_memory("compact-held", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));

    harness
        .engine
        .compact_then_send("t", "m1", "carry on".into(), Vec::new())
        .unwrap();
    let held = harness.engine.snapshot().held_messages;
    let taken = harness.engine.take_held_message("t", false);
    let left = harness.engine.snapshot().held_messages;
    finish(&adapter, 0, TurnOutcome::Completed);
    harness.wait_for_turn_end();

    assert_eq!(held.len(), 1);
    assert_eq!(held[0].text, "carry on");
    assert_eq!(taken.map(|message| message.message_id), Some("m1".into()));
    assert!(left.is_empty());
    let prompts: Vec<_> = adapter
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.prompt.clone())
        .collect();
    assert_eq!(prompts, ["/compact"]);
}

#[test]
fn sending_a_held_message_now_stops_the_compaction_it_waited_for() {
    let harness = Harness::in_memory("compact-skip", Vec::new());
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness
        .engine
        .compact_then_send("t", "m1", "carry on".into(), Vec::new())
        .unwrap();

    harness.engine.send_held_message_now("t").unwrap();
    harness.wait_for_turn_end();
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));

    assert_eq!(harness.thread("t").active_turn_id.as_deref(), Some("m1"));
    assert!(harness.engine.snapshot().held_messages.is_empty());
}

#[test]
fn a_message_after_a_compaction_that_failed_at_once_is_still_sent() {
    let harness = Harness::in_memory("compact-failed", completed_turn());
    harness.engine.set_adapter(None);

    harness
        .engine
        .compact_then_send("t", "m1", "carry on".into(), Vec::new())
        .unwrap();

    let ids: Vec<_> = harness
        .engine
        .transcript("t", None)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert!(ids.contains(&"m1".to_owned()));
    assert!(lock(&harness.engine.state).after_compaction.is_empty());
}

#[test]
fn a_watched_pull_request_wakes_its_thread_on_a_failing_check() {
    let harness = Harness::in_memory("pr-watch", completed_turn());
    let pull_request = hoshi_core::PullRequest {
        forge: ForgeKind::Github,
        number: 7,
        title: "Work".into(),
        url: "https://github.com/o/r/pull/7".into(),
        author: Some("me".into()),
        head_branch: "hoshi/work".into(),
        head_sha: Some("abc1234".into()),
        base_branch: "main".into(),
        state: hoshi_core::PullRequestState::Open,
        checks: None,
        unresolved_comments: 0,
        open_tasks: 0,
        changes_requested: false,
        turn: None,
        conflicts: false,
        additions: 0,
        deletions: 0,
        updated_at: "2026-11-07T12:00:00Z".into(),
    };
    let detail = |state: hoshi_core::ChecksState| hoshi_core::PullRequestDetail {
        supports: hoshi_core::PullRequestSupport::all(),
        forge_id: "id".into(),
        repository: "o/r".into(),
        mergeability: hoshi_core::Mergeability::Mergeable,
        merge_methods: Vec::new(),
        can_edit: true,
        body: String::new(),
        changed_files: 1,
        reviewers: Vec::new(),
        labels: Vec::new(),
        checks: vec![hoshi_core::PullRequestCheck {
            name: "lint".into(),
            state,
            url: None,
        }],
        review_threads: Vec::new(),
        timeline: Vec::new(),
    };
    let call =
        |tool: &str| lock(&harness.engine.state).call_tool("t", tool, &serde_json::json!({}));
    assert!(call("watch_pull_request").is_err());
    lock(&harness.engine.state).pull_requests.record(
        "t",
        ForgeKind::Github,
        Ok(Some(pull_request.clone())),
        false,
        Instant::now(),
        chrono::Utc::now(),
    );

    let watched = call("watch_pull_request").unwrap();
    assert_eq!(watched["number"], 7);
    assert!(harness.thread("t").watching_pull_request);
    let observe = |state: hoshi_core::ChecksState| {
        lock(&harness.engine.state).observe_watch(
            "t",
            ForgeKind::Github,
            &pull_request,
            &detail(state),
            Some("me".into()),
        );
    };
    observe(hoshi_core::ChecksState::Pending);
    observe(hoshi_core::ChecksState::Failing);

    let update = harness.wait_for(|event| match event {
        Event::ItemAdded { item } => match item.content {
            ItemContent::UserMessage {
                pull_request_update: Some(update),
                text,
                ..
            } => Some((update, text)),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(update.0.failed_checks, ["lint"]);
    assert!(update.1.contains("Checks failed on abc1234"));
    harness.wait_for_turn_end();
    let unwatched = call("unwatch_pull_request").unwrap();
    assert_eq!(unwatched["wasWatching"], true);
    assert!(!harness.thread("t").watching_pull_request);
}

#[test]
fn a_comment_left_while_the_app_was_closed_wakes_the_watching_thread() {
    let dir = scratch_dir("pr-watch-restart");
    let database = dir.join("state.sqlite");
    let harness = Harness::new(
        "pr-watch-restart",
        dir,
        Store::open(&database).unwrap(),
        completed_turn(),
    );
    let pull_request = hoshi_core::PullRequest {
        forge: ForgeKind::Github,
        number: 7,
        title: "Work".into(),
        url: "https://github.com/o/r/pull/7".into(),
        author: Some("me".into()),
        head_branch: "hoshi/work".into(),
        head_sha: Some("abc1234".into()),
        base_branch: "main".into(),
        state: hoshi_core::PullRequestState::Open,
        checks: None,
        unresolved_comments: 0,
        open_tasks: 0,
        changes_requested: false,
        turn: None,
        conflicts: false,
        additions: 0,
        deletions: 0,
        updated_at: "2026-11-07T12:00:00Z".into(),
    };
    lock(&harness.engine.state).pull_requests.record(
        "t",
        ForgeKind::Github,
        Ok(Some(pull_request.clone())),
        false,
        Instant::now(),
        chrono::Utc::now(),
    );
    lock(&harness.engine.state)
        .call_tool("t", "watch_pull_request", &serde_json::json!({}))
        .unwrap();
    drop(harness);

    let (sender, events) = mpsc::channel();
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::Event(envelope) = notification {
            let _ = sender.send(envelope.event);
        }
    });
    let engine = Engine::from_store(
        Store::open(&database).unwrap(),
        &scratch_dir("pr-watch-restarted"),
        notify,
    )
    .unwrap();
    let commented_at =
        (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339_opts(SecondsFormat::Secs, true);
    let detail = hoshi_core::PullRequestDetail {
        supports: hoshi_core::PullRequestSupport::all(),
        forge_id: "id".into(),
        repository: "o/r".into(),
        mergeability: hoshi_core::Mergeability::Mergeable,
        merge_methods: Vec::new(),
        can_edit: true,
        body: String::new(),
        changed_files: 1,
        reviewers: Vec::new(),
        labels: Vec::new(),
        checks: Vec::new(),
        review_threads: Vec::new(),
        timeline: vec![hoshi_core::TimelineItem::Comment {
            author: Some("reviewer".into()),
            body: "Please rename this.".into(),
            created_at: commented_at,
            url: "https://github.com/o/r/pull/7#c1".into(),
        }],
    };

    lock(&engine.state).observe_watch(
        "t",
        ForgeKind::Github,
        &pull_request,
        &detail,
        Some("me".into()),
    );

    let update = events
        .try_iter()
        .find_map(|event| match event {
            Event::ItemAdded { item } => match item.content {
                ItemContent::UserMessage {
                    pull_request_update: Some(update),
                    ..
                } => Some(update),
                _ => None,
            },
            _ => None,
        })
        .expect("a wake for the comment");
    assert_eq!(update.comments, 1);
}

#[test]
fn a_check_that_failed_while_the_app_was_closed_wakes_the_watching_thread() {
    let dir = scratch_dir("pr-watch-checks-restart");
    let database = dir.join("state.sqlite");
    let harness = Harness::new(
        "pr-watch-checks-restart",
        dir,
        Store::open(&database).unwrap(),
        completed_turn(),
    );
    let pull_request = hoshi_core::PullRequest {
        forge: ForgeKind::Github,
        number: 7,
        title: "Work".into(),
        url: "https://github.com/o/r/pull/7".into(),
        author: Some("me".into()),
        head_branch: "hoshi/work".into(),
        head_sha: Some("abc1234".into()),
        base_branch: "main".into(),
        state: hoshi_core::PullRequestState::Open,
        checks: None,
        unresolved_comments: 0,
        open_tasks: 0,
        changes_requested: false,
        turn: None,
        conflicts: false,
        additions: 0,
        deletions: 0,
        updated_at: "2026-11-07T12:00:00Z".into(),
    };
    let detail = |state: hoshi_core::ChecksState| hoshi_core::PullRequestDetail {
        supports: hoshi_core::PullRequestSupport::all(),
        forge_id: "id".into(),
        repository: "o/r".into(),
        mergeability: hoshi_core::Mergeability::Mergeable,
        merge_methods: Vec::new(),
        can_edit: true,
        body: String::new(),
        changed_files: 1,
        reviewers: Vec::new(),
        labels: Vec::new(),
        checks: vec![hoshi_core::PullRequestCheck {
            name: "lint".into(),
            state,
            url: None,
        }],
        review_threads: Vec::new(),
        timeline: Vec::new(),
    };
    lock(&harness.engine.state).pull_requests.record(
        "t",
        ForgeKind::Github,
        Ok(Some(pull_request.clone())),
        false,
        Instant::now(),
        chrono::Utc::now(),
    );
    lock(&harness.engine.state)
        .call_tool("t", "watch_pull_request", &serde_json::json!({}))
        .unwrap();
    lock(&harness.engine.state).observe_watch(
        "t",
        ForgeKind::Github,
        &pull_request,
        &detail(hoshi_core::ChecksState::Pending),
        Some("me".into()),
    );
    drop(harness);

    let (sender, events) = mpsc::channel();
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::Event(envelope) = notification {
            let _ = sender.send(envelope.event);
        }
    });
    let engine = Engine::from_store(
        Store::open(&database).unwrap(),
        &scratch_dir("pr-watch-checks-restarted"),
        notify,
    )
    .unwrap();
    lock(&engine.state).observe_watch(
        "t",
        ForgeKind::Github,
        &pull_request,
        &detail(hoshi_core::ChecksState::Failing),
        Some("me".into()),
    );

    let update = events
        .try_iter()
        .find_map(|event| match event {
            Event::ItemAdded { item } => match item.content {
                ItemContent::UserMessage {
                    pull_request_update: Some(update),
                    ..
                } => Some(update),
                _ => None,
            },
            _ => None,
        })
        .expect("a wake for the failed check");
    assert_eq!(update.failed_checks, ["lint"]);
}

#[test]
fn an_agent_finds_messages_waits_for_and_reads_another_thread() {
    let harness = Harness::in_memory("thread-tools", completed_turn());
    harness.create_thread("o", ThreadWorkspace::Local).unwrap();
    let call = |tool: &str, arguments: serde_json::Value| {
        lock(&harness.engine.state).call_tool("t", tool, &arguments)
    };

    let listed = call("thread_list", serde_json::json!({})).unwrap();
    let ids: Vec<_> = listed["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|thread| thread["threadId"].as_str().unwrap().to_owned())
        .collect();
    assert!(ids.contains(&"o".to_owned()));
    assert!(
        call(
            "thread_send",
            serde_json::json!({ "threadId": "t", "message": "hi" })
        )
        .is_err()
    );
    let sent = call(
        "thread_send",
        serde_json::json!({ "threadId": "o", "message": "check the tests" }),
    )
    .unwrap();
    assert_eq!(sent["delivery"], "started");
    let state = Arc::downgrade(&harness.engine.state);
    let waited = thread_tools::wait_for_thread(
        &state,
        "t",
        &serde_json::json!({ "threadId": "o", "timeoutMs": 10_000 }),
    )
    .unwrap();
    let read = call("thread_read", serde_json::json!({ "threadId": "o" })).unwrap();

    assert_eq!(waited.get("timedOut"), None);
    assert_eq!(waited["lastAnswer"], "done");
    assert_eq!(read.get("archived"), None);
    assert_eq!(read.get("earlierMessages"), None);
    let roles: Vec<_> = read["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| {
            (
                message["role"].as_str().unwrap(),
                message["text"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(roles, [("user", "check the tests"), ("assistant", "done")]);
}

#[test]
fn a_secret_the_user_saves_reaches_the_agent_as_a_file_only() {
    let harness = Harness::in_memory("secret", Vec::new());
    harness.send("t", "m1", "deploy it");
    let ask = |label: &'static str| {
        let state = Arc::downgrade(&harness.engine.state);
        thread::spawn(move || {
            let arguments = serde_json::json!({ "label": label, "reason": "To deploy." });
            secret_requests::request_secret(&state, "t", &arguments)
        })
    };
    let opened = |harness: &Harness| {
        harness.wait_for(|event| match event {
            Event::ItemAdded { item } => match item.content {
                ItemContent::SecretRequest {
                    status: RequestStatus::Pending,
                    ..
                } => Some(item.id),
                _ => None,
            },
            _ => None,
        })
    };

    let saving = ask("Deploy token");
    let request_id = opened(&harness);
    assert_eq!(harness.thread("t").status, ThreadStatus::WaitingForInput);
    harness
        .engine
        .answer_secret_request("t", &request_id, Some("s3cret"))
        .unwrap();
    let answer = saving.join().unwrap().unwrap();
    let declining = ask("Other token");
    let declined_id = opened(&harness);
    harness
        .engine
        .answer_secret_request("t", &declined_id, None)
        .unwrap();
    let declined = declining.join().unwrap().unwrap();

    assert_eq!(answer["status"], "saved");
    let file = answer["file"].as_str().unwrap();
    assert_eq!(std::fs::read_to_string(file).unwrap(), "s3cret");
    assert_eq!(declined["status"], "declined");
    let items = harness.engine.transcript("t", None).unwrap().items;
    let recorded = serde_json::to_string(&items).unwrap();
    assert!(!recorded.contains("s3cret"));
    assert!(items.iter().any(|item| matches!(
        item.content,
        ItemContent::SecretRequest {
            saved: Some(true),
            status: RequestStatus::Resolved,
            ..
        }
    )));
    assert_eq!(harness.thread("t").status, ThreadStatus::Working);
}

#[test]
fn a_git_action_commits_on_a_new_branch_and_pushes_it() {
    let origin = git::tests::temp_repository("engine-git-origin");
    let origin_dir = origin.to_string_lossy().into_owned();
    git::stdout(
        &origin_dir,
        &["config", "receive.denyCurrentBranch", "ignore"],
    )
    .unwrap();
    let repo = git::tests::temp_repository("engine-git-action");
    let harness = Harness::new(
        "git-action",
        repo.clone(),
        Store::open_in_memory().unwrap(),
        completed_turn(),
    );
    git::stdout(&harness.root, &["remote", "add", "origin", &origin_dir]).unwrap();
    git::stdout(&harness.root, &["fetch", "origin"]).unwrap();
    std::fs::write(repo.join("login.txt"), "fixed\n").unwrap();
    let request = |message: Option<&str>| GitActionRequest {
        action: GitAction::CommitPush,
        message: message.map(Into::into),
        paths: None,
        new_branch: true,
        draft: false,
    };

    assert_eq!(
        harness
            .engine
            .run_git_action("t", &request(None))
            .unwrap_err(),
        CommandError::CommitMessageUnavailable {
            thread_id: "t".into()
        }
    );
    let before = harness.engine.git_status("t").unwrap();
    assert!(before.has_changes && before.has_remote);
    let message = "fix: stop the login loop\n\n- keep it";
    let result = harness
        .engine
        .run_git_action("t", &request(Some(message)))
        .unwrap();

    assert_eq!(result.subject.as_deref(), Some("fix: stop the login loop"));
    let branch = "hoshi/fix-stop-the-login-loop";
    assert_eq!(result.pushed.as_deref(), Some(branch));
    let after = harness.engine.git_status("t").unwrap();
    assert!(!after.has_changes && after.has_upstream);
    assert_eq!(after.branch.as_deref(), Some(branch));
    assert!(git::branch_tip(&origin_dir, branch).is_some());
    let _ = std::fs::remove_dir_all(&origin);
}

/// Bitbucket as far as opening pull requests goes.
#[derive(Default)]
struct FakeBitbucket {
    opened: Mutex<Vec<hoshi_forge::NewPullRequest>>,
}

impl FakeBitbucket {
    fn pull_request(opened: &hoshi_forge::NewPullRequest) -> PullRequest {
        PullRequest {
            forge: ForgeKind::Bitbucket,
            number: 12,
            title: opened.title.clone(),
            url: "https://bitbucket.org/acme/shop/pull-requests/12".into(),
            author: Some("ana".into()),
            head_branch: opened.head.clone(),
            head_sha: None,
            base_branch: opened.base.clone(),
            state: hoshi_core::PullRequestState::Open,
            checks: None,
            unresolved_comments: 0,
            open_tasks: 0,
            changes_requested: false,
            turn: None,
            conflicts: false,
            additions: 0,
            deletions: 0,
            updated_at: "2026-11-09T12:00:00Z".into(),
        }
    }
}

impl Forge for FakeBitbucket {
    fn kind(&self) -> ForgeKind {
        ForgeKind::Bitbucket
    }
    fn viewer(&self) -> Result<String, ForgeError> {
        Ok("ana".into())
    }
    fn pull_request_for_branch(
        &self,
        _: &RepositoryRef,
        branch: &str,
    ) -> Result<Option<PullRequest>, ForgeError> {
        Ok(self
            .opened
            .lock()
            .unwrap()
            .iter()
            .find(|opened| opened.head == branch)
            .map(Self::pull_request))
    }
    fn pull_request(&self, _: &RepositoryRef, _: u32) -> Result<PullRequest, ForgeError> {
        Err(ForgeError::NotFound)
    }
    fn search_pull_requests(
        &self,
        _: &RepositoryRef,
        _: &str,
    ) -> Result<Vec<PullRequest>, ForgeError> {
        Ok(Vec::new())
    }
    fn pull_request_detail(
        &self,
        _: &RepositoryRef,
        _: u32,
    ) -> Result<PullRequestDetail, ForgeError> {
        Err(ForgeError::NotFound)
    }
    fn pull_request_files(
        &self,
        _: &RepositoryRef,
        _: u32,
    ) -> Result<Vec<hoshi_forge::PullRequestFile>, ForgeError> {
        Ok(Vec::new())
    }
    fn act(
        &self,
        _: &RepositoryRef,
        _: u32,
        _: &str,
        _: &PullRequestAction,
    ) -> Result<(), ForgeError> {
        Err(ForgeError::NotFound)
    }
    fn create_pull_request(
        &self,
        _: &RepositoryRef,
        pull_request: &hoshi_forge::NewPullRequest,
    ) -> Result<PullRequest, ForgeError> {
        self.opened.lock().unwrap().push(pull_request.clone());
        Ok(Self::pull_request(pull_request))
    }
    fn reviewer_candidates(&self, _: &RepositoryRef, _: &str) -> Result<Vec<String>, ForgeError> {
        Ok(Vec::new())
    }
    fn labels(&self, _: &RepositoryRef) -> Result<Vec<PullRequestLabel>, ForgeError> {
        Ok(Vec::new())
    }
    fn repository(&self, _: &RepositoryRef) -> Result<RemoteRepository, ForgeError> {
        Err(ForgeError::NotFound)
    }
}

#[test]
fn an_agent_on_bitbucket_pushes_its_branch_and_opens_its_pull_request() {
    let origin = git::tests::temp_repository("engine-pr-origin");
    let origin_dir = origin.to_string_lossy().into_owned();
    git_in(
        &origin_dir,
        &["config", "receive.denyCurrentBranch", "ignore"],
    );
    let repo = git::tests::temp_repository("engine-pr-tool");
    let harness = Harness::new(
        "pr-tool",
        repo,
        Store::open_in_memory().unwrap(),
        completed_turn(),
    );
    git_in(&harness.root, &["remote", "add", "origin", &origin_dir]);
    git_in(&harness.root, &["fetch", "origin"]);
    git_in(&harness.root, &["switch", "-c", "fix/login"]);
    git_in(
        &harness.root,
        &["commit", "--allow-empty", "-m", "fix: login"],
    );
    let state = Arc::downgrade(&harness.engine.state);
    let open = |arguments: serde_json::Value| {
        pull_request_tool::create_pull_request(&state, "t", &arguments)
    };
    let arguments = serde_json::json!({ "title": "Fix the login", "description": "Done." });

    assert!(
        open(arguments.clone())
            .unwrap_err()
            .contains("isn't on Bitbucket")
    );
    let forge = Arc::new(FakeBitbucket::default());
    harness.engine.set_forges(vec![forge.clone()]);
    lock(&harness.engine.state).repositories.insert(
        "p".into(),
        RepositoryRef {
            forge: ForgeKind::Bitbucket,
            host: "bitbucket.org".into(),
            owner: "acme".into(),
            name: "shop".into(),
        },
    );
    assert!(lock(&harness.engine.state).opens_pull_requests("t"));

    let opened = open(arguments.clone()).unwrap();

    assert_eq!(opened["number"], 12);
    assert_eq!(opened["state"], "open");
    assert_eq!(opened["base"], "main");
    assert_eq!(opened["pushed"], true);
    assert!(git::branch_tip(&origin_dir, "fix/login").is_some());
    let sent = forge.opened.lock().unwrap()[0].clone();
    assert_eq!(
        (sent.head.as_str(), sent.body.as_str()),
        ("fix/login", "Done.")
    );
    assert_eq!(
        lock(&harness.engine.state)
            .pull_requests
            .get("t")
            .map(|pull_request| pull_request.number),
        Some(12)
    );
    let again = open(arguments).unwrap_err();
    assert!(
        again.contains("already open") && again.contains("pull-requests/12"),
        "{again}"
    );
    let _ = std::fs::remove_dir_all(&origin);
}

#[test]
fn a_worktree_thread_runs_in_its_own_worktree_until_deleted() {
    let repo = git::tests::temp_repository("engine-worktree");
    let store = Store::open_in_memory().unwrap();
    let harness = Harness::new("worktree", repo, store, completed_turn());

    harness
        .create_thread("w1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    let thread = harness.thread("w1234567-rest");
    let worktree = thread.worktree_path.clone().unwrap();
    harness.send("w1234567-rest", "m1", "work");
    harness.wait_for_turn_end();

    assert_eq!(thread.branch.as_deref(), Some("hoshi/w1234567"));
    assert_eq!(
        git::current_branch(&worktree).as_deref(),
        Some("hoshi/w1234567")
    );
    assert_eq!(harness.adapter.requests.lock().unwrap()[0].cwd, worktree);
    harness
        .engine
        .dispatch(Command::ThreadDelete {
            thread_id: "w1234567-rest".into(),
        })
        .unwrap();
    wait_until(|| !Path::new(&worktree).exists());
}

#[test]
fn a_thread_can_start_on_a_branch_only_origin_has() {
    let origin = git::tests::temp_repository("engine-origin");
    let origin_str = origin.to_string_lossy().into_owned();
    git_in(&origin_str, &["switch", "-c", "feature"]);
    git_in(
        &origin_str,
        &["commit", "--allow-empty", "-m", "feature work"],
    );
    let tip = git_in(&origin_str, &["rev-parse", "HEAD"]);
    git_in(&origin_str, &["switch", "main"]);
    let repo = git::tests::temp_repository("engine-from-branch");
    let harness = Harness::new(
        "from-branch",
        repo,
        Store::open_in_memory().unwrap(),
        Vec::new(),
    );
    git_in(&harness.root, &["remote", "add", "origin", &origin_str]);
    git_in(&harness.root, &["branch", "mine"]);

    harness
        .create_thread(
            "f1234567-rest",
            ThreadWorkspace::BranchWorktree {
                branch: "feature".into(),
            },
        )
        .unwrap();
    harness
        .create_thread(
            "g1234567-rest",
            ThreadWorkspace::BranchWorktree {
                branch: "mine".into(),
            },
        )
        .unwrap();

    let feature = harness.thread("f1234567-rest");
    let worktree = feature.worktree_path.unwrap();
    assert_eq!(feature.branch.as_deref(), Some("feature"));
    assert_eq!(git_in(&worktree, &["rev-parse", "HEAD"]), tip);
    assert_eq!(
        git_in(&worktree, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/feature"
    );
    let mine = harness.thread("g1234567-rest");
    assert_eq!(
        git::current_branch(&mine.worktree_path.unwrap()).as_deref(),
        Some("mine")
    );
    assert!(matches!(
        harness.create_thread(
            "h1234567-rest",
            ThreadWorkspace::BranchWorktree {
                branch: "nowhere".into(),
            },
        ),
        Err(CommandError::WorktreeFailed { .. })
    ));
}

fn git_in(dir: &str, args: &[&str]) -> String {
    git::stdout(dir, args).unwrap().trim().to_owned()
}

/// The thread's pull request, as the sync would have read it.
fn set_pull_request(
    harness: &Harness,
    thread_id: &str,
    state: hoshi_core::PullRequestState,
    head_sha: Option<String>,
) {
    let branch = harness.thread(thread_id).branch.unwrap();
    let pull_request = hoshi_core::PullRequest {
        forge: ForgeKind::Github,
        number: 7,
        title: "Work".into(),
        url: "https://github.com/o/r/pull/7".into(),
        author: None,
        head_branch: branch,
        head_sha,
        base_branch: "main".into(),
        state,
        checks: None,
        unresolved_comments: 0,
        open_tasks: 0,
        changes_requested: false,
        turn: None,
        conflicts: false,
        additions: 0,
        deletions: 0,
        updated_at: "2026-11-07T12:00:00Z".into(),
    };
    lock(&harness.engine.state).pull_requests.record(
        thread_id,
        ForgeKind::Github,
        Ok(Some(pull_request)),
        false,
        Instant::now(),
        chrono::Utc::now(),
    );
}

fn commit_in(dir: &str, file: &str) -> String {
    std::fs::write(Path::new(dir).join(file), "work").unwrap();
    git_in(dir, &["add", file]);
    git_in(dir, &["commit", "-m", file]);
    git_in(dir, &["rev-parse", "HEAD"])
}

#[test]
fn a_released_worktree_comes_back_with_the_next_message() {
    let repo = git::tests::temp_repository("engine-release");
    let harness = Harness::new(
        "release",
        repo,
        Store::open_in_memory().unwrap(),
        completed_turn(),
    );
    harness
        .create_thread("r1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    let worktree = harness.thread("r1234567-rest").worktree_path.unwrap();

    harness.engine.release_worktree("r1234567-rest").unwrap();

    let thread = harness.thread("r1234567-rest");
    assert!(!Path::new(&worktree).exists());
    assert!(thread.worktree_released);
    assert_eq!(thread.branch.as_deref(), Some("hoshi/r1234567"));
    assert!(git::branch_tip(&harness.root, "hoshi/r1234567").is_some());

    harness.send("r1234567-rest", "m1", "go on");
    harness.wait_for_turn_end();

    assert!(Path::new(&worktree).is_dir());
    assert!(!harness.thread("r1234567-rest").worktree_released);
    assert_eq!(harness.adapter.requests.lock().unwrap()[0].cwd, worktree);
}

#[test]
fn a_worktree_with_uncommitted_files_is_kept() {
    let repo = git::tests::temp_repository("engine-dirty");
    let harness = Harness::new("dirty", repo, Store::open_in_memory().unwrap(), Vec::new());
    harness
        .create_thread("d1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    let worktree = harness.thread("d1234567-rest").worktree_path.unwrap();
    std::fs::write(Path::new(&worktree).join("notes.txt"), "draft").unwrap();

    let refused = harness.engine.release_worktree("d1234567-rest");

    assert_eq!(
        refused,
        Err(CommandError::WorktreeDirty {
            thread_id: "d1234567-rest".into()
        })
    );
    assert!(Path::new(&worktree).join("notes.txt").exists());
    assert!(!harness.thread("d1234567-rest").worktree_released);
}

#[test]
fn a_lock_left_by_a_finished_agent_is_lifted_and_a_live_one_kept() {
    let repo = git::tests::temp_repository("engine-locked");
    let harness = Harness::new("locked", repo, Store::open_in_memory().unwrap(), Vec::new());
    for id in ["l1234567-rest", "k1234567-rest"] {
        harness
            .create_thread(id, ThreadWorkspace::NewWorktree)
            .unwrap();
    }
    let stale = harness.thread("l1234567-rest").worktree_path.unwrap();
    let kept = harness.thread("k1234567-rest").worktree_path.unwrap();
    // No process has this pid: the agent is gone.
    let gone = "claude session work (pid 4294967294)";
    git_in(
        &harness.root,
        &["worktree", "lock", "--reason", gone, &stale],
    );
    git_in(
        &harness.root,
        &["worktree", "lock", "--reason", "mine", &kept],
    );

    harness.engine.release_worktree("l1234567-rest").unwrap();
    let refused = harness.engine.release_worktree("k1234567-rest");

    assert!(!Path::new(&stale).exists());
    assert_eq!(
        refused,
        Err(CommandError::WorktreeLocked {
            thread_id: "k1234567-rest".into(),
            reason: "mine".into()
        })
    );
    assert!(Path::new(&kept).is_dir());
}

#[test]
fn cleaning_up_waits_for_a_merge_and_keeps_local_only_commits() {
    let repo = git::tests::temp_repository("engine-cleanup-refused");
    let harness = Harness::new(
        "cleanup-refused",
        repo,
        Store::open_in_memory().unwrap(),
        Vec::new(),
    );
    harness
        .create_thread("c1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    let worktree = harness.thread("c1234567-rest").worktree_path.unwrap();
    let pushed = commit_in(&worktree, "a.txt");
    set_pull_request(
        &harness,
        "c1234567-rest",
        hoshi_core::PullRequestState::Open,
        Some(pushed.clone()),
    );
    assert_eq!(
        harness.engine.clean_up_thread("c1234567-rest"),
        Err(CommandError::PullRequestNotMerged {
            thread_id: "c1234567-rest".into()
        })
    );

    commit_in(&worktree, "after-merge.txt");
    set_pull_request(
        &harness,
        "c1234567-rest",
        hoshi_core::PullRequestState::Merged,
        Some(pushed),
    );

    assert_eq!(
        harness.engine.clean_up_thread("c1234567-rest"),
        Err(CommandError::UnpushedCommits {
            thread_id: "c1234567-rest".into()
        })
    );
    assert!(Path::new(&worktree).is_dir());
    assert!(git::branch_tip(&harness.root, "hoshi/c1234567").is_some());
}

#[test]
fn the_sweep_removes_merged_worktrees_and_keeps_their_branches() {
    let repo = git::tests::temp_repository("engine-merged-sweep");
    let harness = Harness::new(
        "merged-sweep",
        repo,
        Store::open_in_memory().unwrap(),
        Vec::new(),
    );
    for id in [
        "s1234567-rest",
        "o1234567-rest",
        "p1234567-rest",
        "d1234567-rest",
    ] {
        harness
            .create_thread(id, ThreadWorkspace::NewWorktree)
            .unwrap();
    }
    let path = |id: &str| harness.thread(id).worktree_path.unwrap();
    let (merged, open, moved_on) = (
        path("s1234567-rest"),
        path("o1234567-rest"),
        path("p1234567-rest"),
    );
    let head = commit_in(&merged, "a.txt");
    set_pull_request(
        &harness,
        "s1234567-rest",
        hoshi_core::PullRequestState::Merged,
        Some(head),
    );
    let head = commit_in(&open, "b.txt");
    set_pull_request(
        &harness,
        "o1234567-rest",
        hoshi_core::PullRequestState::Open,
        Some(head),
    );
    let head = commit_in(&moved_on, "c.txt");
    commit_in(&moved_on, "after-merge.txt");
    set_pull_request(
        &harness,
        "p1234567-rest",
        hoshi_core::PullRequestState::Merged,
        Some(head),
    );
    let dirty = path("d1234567-rest");
    let head = commit_in(&dirty, "d.txt");
    std::fs::write(Path::new(&dirty).join("draft.txt"), "wip").unwrap();
    set_pull_request(
        &harness,
        "d1234567-rest",
        hoshi_core::PullRequestState::Merged,
        Some(head),
    );

    let mut cleanup = harness.engine.delete_merged_worktrees();
    assert_eq!(cleanup.removed, ["s1234567-rest"]);
    assert_eq!(harness.engine.merged_cleanup(), Some(cleanup.clone()));
    cleanup.kept.sort();
    assert_eq!(
        cleanup.kept,
        [
            ("d1234567-rest".to_owned(), KeptReason::Changes),
            ("p1234567-rest".to_owned(), KeptReason::NewCommits),
        ]
    );
    assert!(Path::new(&dirty).is_dir());

    assert!(!Path::new(&merged).exists());
    assert!(harness.thread("s1234567-rest").worktree_released);
    assert!(git::branch_tip(&harness.root, "hoshi/s1234567").is_some());
    assert!(Path::new(&open).is_dir());
    assert!(
        Path::new(&moved_on).is_dir(),
        "commits after the merge stay checked out"
    );
}

#[test]
fn cleaning_up_a_merged_thread_removes_only_local_things() {
    let repo = git::tests::temp_repository("engine-cleanup");
    let harness = Harness::new(
        "cleanup",
        repo,
        Store::open_in_memory().unwrap(),
        Vec::new(),
    );
    harness
        .create_thread("m1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    let worktree = harness.thread("m1234567-rest").worktree_path.unwrap();
    let head = commit_in(&worktree, "a.txt");
    set_pull_request(
        &harness,
        "m1234567-rest",
        hoshi_core::PullRequestState::Merged,
        Some(head),
    );

    harness.engine.clean_up_thread("m1234567-rest").unwrap();

    let thread = harness.thread("m1234567-rest");
    assert!(!Path::new(&worktree).exists());
    assert!(git::branch_tip(&harness.root, "hoshi/m1234567").is_none());
    assert!(thread.archived);
    assert_eq!(thread.worktree_path, None);
    assert_eq!(thread.branch, None);
    assert_eq!(thread.session_id, None);
    assert!(git::branch_tip(&harness.root, "main").is_some());
}

#[test]
fn cleaning_up_a_root_thread_goes_back_to_the_base_branch() {
    let repo = git::tests::temp_repository("engine-cleanup-root");
    let harness = Harness::new(
        "cleanup-root",
        repo,
        Store::open_in_memory().unwrap(),
        Vec::new(),
    );
    harness
        .create_thread("r1234567-rest", ThreadWorkspace::Local)
        .unwrap();
    git_in(&harness.root, &["branch", "feature"]);
    harness
        .engine
        .switch_branch("r1234567-rest", "feature")
        .unwrap();
    let head = commit_in(&harness.root, "a.txt");
    set_pull_request(
        &harness,
        "r1234567-rest",
        hoshi_core::PullRequestState::Merged,
        Some(head),
    );
    std::fs::write(Path::new(&harness.root).join("draft.txt"), "draft").unwrap();

    assert_eq!(
        harness.engine.clean_up_thread("r1234567-rest"),
        Err(CommandError::WorktreeDirty {
            thread_id: "r1234567-rest".into()
        })
    );
    assert_eq!(
        git::current_branch(&harness.root).as_deref(),
        Some("feature")
    );

    std::fs::remove_file(Path::new(&harness.root).join("draft.txt")).unwrap();
    harness.engine.clean_up_thread("r1234567-rest").unwrap();

    let thread = harness.thread("r1234567-rest");
    assert_eq!(git::current_branch(&harness.root).as_deref(), Some("main"));
    assert!(git::branch_tip(&harness.root, "feature").is_none());
    assert!(thread.archived);
    assert_eq!(thread.session_id, None);
}

#[test]
fn a_root_cleanup_that_fails_after_switching_still_tells_the_threads_sharing_the_root() {
    let repo = git::tests::temp_repository("engine-cleanup-root-fails");
    let harness = Harness::new(
        "cleanup-root-fails",
        repo,
        Store::open_in_memory().unwrap(),
        Vec::new(),
    );
    harness
        .create_thread("r1234567-rest", ThreadWorkspace::Local)
        .unwrap();
    harness
        .create_thread("s1234567-rest", ThreadWorkspace::Local)
        .unwrap();
    git_in(&harness.root, &["branch", "feature"]);
    harness
        .engine
        .switch_branch("r1234567-rest", "feature")
        .unwrap();
    let head = commit_in(&harness.root, "a.txt");
    set_pull_request(
        &harness,
        "r1234567-rest",
        hoshi_core::PullRequestState::Merged,
        Some(head),
    );
    let root = harness.root.clone();
    lock(&harness.engine.state).record_branch_read("s1234567-rest", &root, Some("feature".into()));
    assert_eq!(
        harness.thread("s1234567-rest").branch.as_deref(),
        Some("feature")
    );
    let lock_file = Path::new(&harness.root).join(".git/refs/heads/feature.lock");
    std::fs::write(&lock_file, "").unwrap();

    assert!(matches!(
        harness.engine.clean_up_thread("r1234567-rest"),
        Err(CommandError::GitFailed { .. })
    ));

    assert_eq!(git::current_branch(&harness.root).as_deref(), Some("main"));
    assert_eq!(
        harness.thread("s1234567-rest").branch.as_deref(),
        Some("main")
    );
    std::fs::remove_file(lock_file).unwrap();
}

fn bash(command: &str) -> ProviderOutput {
    ProviderOutput::ToolCallStarted {
        item_id: command.into(),
        name: "Bash".into(),
        input: serde_json::json!({ "command": command }).to_string(),
    }
}

fn interrupt(harness: &Harness) {
    harness
        .engine
        .dispatch(Command::TurnInterrupt {
            thread_id: "t".into(),
        })
        .unwrap();
}

fn turn_changes(harness: &Harness) -> Vec<DiffFile> {
    harness.wait_for(|event| match event {
        Event::ItemAdded { item } => match item.content {
            ItemContent::TurnChanges { files: Some(files) } => Some(files),
            _ => None,
        },
        _ => None,
    })
}

fn paths_of(files: &[DiffFile]) -> Vec<&str> {
    files.iter().map(|file| file.path.as_str()).collect()
}

/// A folder beside the project's, named after it as agents name the worktrees they add.
fn sibling(repo: &Path, suffix: &str) -> PathBuf {
    let name = repo.file_name().unwrap().to_string_lossy();
    let path = repo.with_file_name(format!("{name}-{suffix}"));
    let _ = std::fs::remove_dir_all(&path);
    path
}

#[test]
fn a_worktree_the_agent_adds_beside_the_project_and_works_in_becomes_the_thread_folder() {
    let repo = git::tests::temp_repository("engine-agent-worktree");
    commit_in(&repo.to_string_lossy(), "README.md");
    let worktree = sibling(&repo, "1234");
    let name = worktree.file_name().unwrap().to_string_lossy().into_owned();
    let script = vec![
        bash(&format!("git worktree add ../{name} -b fix/1234 main")),
        bash(&format!("cd ../{name} && echo fixed > fix.txt")),
    ];
    let store = Store::open_in_memory().unwrap();
    let harness = Harness::new("agent-worktree", repo.clone(), store, script);
    std::fs::write(repo.join("draft.txt"), "mine\n").unwrap();
    harness.send("t", "m1", "add a worktree for ticket 1234 and fix it there");
    let before = checkpoint::reference("t", "m1", Moment::Before);
    wait_until(|| checkpoint::exists(&harness.root, &before));

    git::add_worktree(&harness.root, &worktree, "fix/1234").unwrap();
    std::fs::write(worktree.join("fix.txt"), "fixed\n").unwrap();
    std::fs::write(repo.join("draft.txt"), "mine, edited by hand\n").unwrap();
    interrupt(&harness);

    let expected = git::normalize(&worktree.to_string_lossy());
    let attached = harness.wait_for(|event| match event {
        Event::ThreadWorktreeAttached {
            worktree_path,
            branch,
            ..
        } => Some((worktree_path, branch)),
        _ => None,
    });
    assert_eq!(attached, (expected.clone(), Some("fix/1234".into())));
    let files = turn_changes(&harness);
    assert_eq!(paths_of(&files), ["fix.txt"]);
    assert_eq!(harness.engine.turn_diff_files("t", "m1").unwrap(), files);
    assert!(harness.thread("t").worktree_adopted);

    harness.engine.rewind_thread("t", "m1", true).unwrap();
    assert!(!worktree.join("fix.txt").exists());
    assert!(repo.join("draft.txt").exists());

    harness.send("t", "m2", "continue");
    assert_eq!(harness.adapter.requests.lock().unwrap()[1].cwd, expected);
    interrupt(&harness);
    harness.wait_for_turn_end();
    harness
        .engine
        .dispatch(Command::ThreadDelete {
            thread_id: "t".into(),
        })
        .unwrap();
    assert!(worktree.is_dir());
    let _ = git::remove_worktree(&harness.root, &expected);
}

#[test]
fn a_worktree_the_agent_only_adds_leaves_the_thread_where_it_was() {
    let repo = git::tests::temp_repository("engine-unused-worktree");
    let worktree = sibling(&repo, "later");
    let name = worktree.file_name().unwrap().to_string_lossy().into_owned();
    let script = vec![bash(&format!("git worktree add ../{name} -b chore/later"))];
    let store = Store::open_in_memory().unwrap();
    let harness = Harness::new("unused-worktree", repo.clone(), store, script);
    harness.send("t", "m1", "prepare a worktree for later");
    let before = checkpoint::reference("t", "m1", Moment::Before);
    wait_until(|| checkpoint::exists(&harness.root, &before));

    git::add_worktree(&harness.root, &worktree, "chore/later").unwrap();
    std::fs::write(repo.join("notes.txt"), "plan\n").unwrap();
    interrupt(&harness);

    assert_eq!(paths_of(&turn_changes(&harness)), ["notes.txt"]);
    assert_eq!(harness.thread("t").worktree_path, None);
    assert!(worktree.is_dir());
    let _ = git::remove_worktree(&harness.root, &worktree.to_string_lossy());
}

#[test]
fn a_turn_that_moves_to_a_worktree_lists_what_it_changed_there() {
    let repo = git::tests::temp_repository("engine-moved-changes");
    commit_in(&repo.to_string_lossy(), "README.md");
    let worktree = sibling(&repo, "moved");
    git::add_worktree(&repo.to_string_lossy(), &worktree, "feat/moved").unwrap();
    let path = git::normalize(&worktree.to_string_lossy());
    let store = Store::open_in_memory().unwrap();
    let harness = Harness::new("moved-changes", repo.clone(), store, Vec::new());
    harness.send("t", "m1", "work in the other worktree");
    let before = checkpoint::reference("t", "m1", Moment::Before);
    wait_until(|| checkpoint::exists(&harness.root, &before));
    std::fs::write(repo.join("stray.txt"), "root\n").unwrap();

    let entered = ProviderOutput::WorkingDirectory { path: path.clone() };
    lock(&harness.engine.state).receive("t", "m1", entered);
    let arrived = checkpoint::reference("t", "m1", Moment::Arrived(1));
    wait_until(|| checkpoint::exists(&path, &arrived));
    std::fs::write(worktree.join("work.txt"), "done\n").unwrap();
    interrupt(&harness);

    let files = turn_changes(&harness);
    assert_eq!(paths_of(&files), ["work.txt"]);
    assert_eq!(
        harness.thread("t").worktree_path.as_deref(),
        Some(path.as_str())
    );
    harness.engine.rewind_thread("t", "m1", true).unwrap();
    assert!(!worktree.join("work.txt").exists());
    assert!(repo.join("stray.txt").exists());
    let _ = git::remove_worktree(&harness.root, &path);
}

#[test]
fn files_are_never_reverted_in_another_folder_than_the_turn_s() {
    let repo = git::tests::temp_repository("engine-rewind-elsewhere");
    let worktree = sibling(&repo, "elsewhere");
    git::add_worktree(&repo.to_string_lossy(), &worktree, "feat/elsewhere").unwrap();
    let path = git::normalize(&worktree.to_string_lossy());
    let store = Store::open_in_memory().unwrap();
    let harness = Harness::new("rewind-elsewhere", repo.clone(), store, Vec::new());
    harness.send("t", "m1", "change the root");
    let before = checkpoint::reference("t", "m1", Moment::Before);
    wait_until(|| checkpoint::exists(&harness.root, &before));
    std::fs::write(repo.join("root.txt"), "root\n").unwrap();
    interrupt(&harness);
    assert_eq!(paths_of(&turn_changes(&harness)), ["root.txt"]);

    harness.send("t", "m2", "now go to the worktree");
    let entered = ProviderOutput::WorkingDirectory { path: path.clone() };
    lock(&harness.engine.state).receive("t", "m2", entered);
    wait_until(|| harness.thread("t").worktree_path.is_some());
    std::fs::write(worktree.join("kept.txt"), "kept\n").unwrap();
    interrupt(&harness);
    harness.wait_for_turn_end();

    assert_eq!(
        harness.engine.rewind_thread("t", "m1", true).unwrap_err(),
        CommandError::NoCheckpoint {
            thread_id: "t".into(),
            message_id: "m1".into()
        }
    );
    assert!(worktree.join("kept.txt").exists());
    assert!(repo.join("root.txt").exists());
    let _ = git::remove_worktree(&harness.root, &path);
}

#[test]
fn a_finished_turn_lists_the_files_it_changed_and_shows_their_diff() {
    let repo = git::tests::temp_repository("engine-turn-changes");
    let store = Store::open_in_memory().unwrap();
    let harness = Harness::new("turn-changes", repo.clone(), store, Vec::new());
    harness.send("t", "m1", "add a file");
    let before = checkpoint::reference("t", "m1", Moment::Before);
    wait_until(|| git::stdout(&harness.root, &["rev-parse", "--verify", &before]).is_ok());

    std::fs::write(repo.join("notes.txt"), "one\ntwo\n").unwrap();
    harness
        .engine
        .dispatch(Command::TurnInterrupt {
            thread_id: "t".into(),
        })
        .unwrap();

    let files = harness.wait_for(|event| match event {
        Event::ItemAdded { item } => match item.content {
            ItemContent::TurnChanges { files: Some(files) } => Some(files),
            _ => None,
        },
        _ => None,
    });
    let summary: Vec<_> = files
        .iter()
        .map(|file| (file.path.as_str(), file.additions))
        .collect();
    assert_eq!(summary, [("notes.txt", Some(2))]);
    assert_eq!(harness.engine.turn_diff_files("t", "m1").unwrap(), files);
    let diff = harness.engine.turn_file_diff("t", "m1", &files[0]).unwrap();
    assert!(diff.text.contains("+two"));
}

#[test]
fn a_rewind_goes_back_to_what_the_turn_before_ran_on() {
    let mut script = completed_turn();
    script.insert(
        2,
        ProviderOutput::ConversationCursor {
            cursor: "u-answer".into(),
        },
    );
    let harness = Harness::in_memory("rewind-model", script);
    let effort = |effort| Command::ThreadModelSet {
        thread_id: "t".into(),
        provider: DEFAULT_PROVIDER.into(),
        model: Some("opus".into()),
        effort: Some(effort),
        fast_mode: false,
        ultracode: false,
    };
    harness.engine.dispatch(effort(Effort::Medium)).unwrap();
    harness.send("t", "m1", "one");
    harness.wait_for_turn_end();
    harness.engine.dispatch(effort(Effort::High)).unwrap();
    harness.send("t", "m2", "two");
    harness.wait_for_turn_end();

    harness.engine.rewind_thread("t", "m2", false).unwrap();
    let last = harness
        .thread("t")
        .last_turn_model
        .map(|model| model.effort);
    assert_eq!(last, Some(Some(Effort::Medium)));

    harness.send("t", "m3", "two again");
    harness.wait_for_turn_end();
    let switch = match harness
        .engine
        .thread_item("t", "m3")
        .unwrap()
        .unwrap()
        .content
    {
        ItemContent::UserMessage { model_switch, .. } => {
            model_switch.map(|switch| (switch.from.effort, switch.to.effort))
        }
        _ => None,
    };
    assert_eq!(switch, Some((Some(Effort::Medium), Some(Effort::High))));
}

#[test]
fn rewinding_drops_the_message_and_resumes_the_conversation_before_it() {
    let mut script = completed_turn();
    script.insert(
        2,
        ProviderOutput::ConversationCursor {
            cursor: "u-answer".into(),
        },
    );
    let harness = Harness::in_memory("rewind", script);
    harness.send("t", "m1", "one");
    harness.wait_for_turn_end();
    harness.send("t", "m2", "two");
    harness.wait_for_turn_end();

    assert_eq!(
        harness.engine.rewind_thread("t", "m2", true).unwrap_err(),
        CommandError::NoCheckpoint {
            thread_id: "t".into(),
            message_id: "m2".into()
        }
    );
    harness.engine.rewind_thread("t", "m2", false).unwrap();

    let ids: Vec<_> = harness
        .engine
        .transcript("t", None)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert!(ids.contains(&"m1".to_owned()));
    assert!(!ids.contains(&"m2".to_owned()));
    harness.send("t", "m3", "two again");
    harness.wait_for_turn_end();
    let requests = harness.adapter.requests.lock().unwrap();
    assert_eq!(requests[1].resume_at, None);
    assert_eq!(requests[2].resume.as_deref(), Some("s1"));
    assert_eq!(requests[2].resume_at.as_deref(), Some("u-answer"));
    drop(requests);
    assert_eq!(harness.thread("t").resume_at, None);
    assert_eq!(
        harness.engine.rewind_thread("t", "m1b", false).unwrap_err(),
        CommandError::CannotRewind {
            thread_id: "t".into(),
            message_id: "m1b".into()
        }
    );
}

fn waiting_adapter() -> Arc<FakeAdapter> {
    Arc::new(FakeAdapter {
        requests: Mutex::new(Vec::new()),
        script: Vec::new(),
        title: None,
        responses: Arc::default(),
        steers: None,
        finishers: Mutex::default(),
    })
}

#[test]
fn files_are_not_reverted_while_another_thread_works_in_the_same_folder() {
    let harness = Harness::in_memory("rewind-shared", completed_turn());
    harness.create_thread("o", ThreadWorkspace::Local).unwrap();
    harness.send("t", "m1", "one");
    harness.wait_for_turn_end();
    harness.engine.set_adapter(Some(waiting_adapter()));
    harness.send("o", "m2", "long task");

    assert_eq!(
        harness.engine.rewind_thread("t", "m1", true).unwrap_err(),
        CommandError::ThreadBusy {
            thread_id: "o".into()
        }
    );
    harness.engine.rewind_thread("t", "m1", false).unwrap();
    harness
        .engine
        .dispatch(Command::TurnInterrupt {
            thread_id: "o".into(),
        })
        .unwrap();
    harness.wait_for_turn_end();
}

#[test]
fn a_message_to_a_thread_git_works_on_waits_for_that_work() {
    let harness = Harness::in_memory("op-held", completed_turn());
    let cwd = lock(&harness.engine.state).cwd("t").unwrap();
    let op = lock(&harness.engine.state).begin_op("t", &[&cwd]).unwrap();
    assert_eq!(
        lock(&harness.engine.state)
            .begin_op("t", &[&cwd])
            .err()
            .unwrap(),
        CommandError::ThreadBusy {
            thread_id: "t".into()
        }
    );

    harness.send("t", "m1", "go");
    assert!(harness.adapter.requests.lock().unwrap().is_empty());
    assert_eq!(harness.thread("t").status, ThreadStatus::Idle);

    lock(&harness.engine.state).end_op(op, None).unwrap();
    harness.wait_for_turn_end();
    assert_eq!(harness.adapter.requests.lock().unwrap().len(), 1);
}

#[test]
fn a_turn_in_a_folder_git_changes_starts_once_git_is_done() {
    let harness = Harness::in_memory("op-parked", completed_turn());
    harness.create_thread("o", ThreadWorkspace::Local).unwrap();
    let cwd = lock(&harness.engine.state).cwd("t").unwrap();
    let op = lock(&harness.engine.state).begin_op("o", &[&cwd]).unwrap();

    harness.send("t", "m1", "go");
    assert!(harness.adapter.requests.lock().unwrap().is_empty());
    assert_eq!(harness.thread("t").status, ThreadStatus::Working);
    let started_on = harness.thread("t").model.clone();
    harness
        .engine
        .dispatch(Command::ThreadModelSet {
            thread_id: "t".into(),
            provider: harness.thread("t").provider.clone(),
            model: Some("chosen-while-waiting".into()),
            effort: None,
            fast_mode: false,
            ultracode: false,
        })
        .unwrap();

    lock(&harness.engine.state).end_op(op, None).unwrap();
    assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
    let requests = harness.adapter.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].model, started_on,
        "it runs on what it started on, as its history says"
    );
}

#[test]
fn a_turn_waiting_for_git_can_be_stopped() {
    let harness = Harness::in_memory("op-parked-stop", completed_turn());
    harness.create_thread("o", ThreadWorkspace::Local).unwrap();
    let cwd = lock(&harness.engine.state).cwd("t").unwrap();
    let op = lock(&harness.engine.state).begin_op("o", &[&cwd]).unwrap();
    harness.send("t", "m1", "go");

    harness
        .engine
        .dispatch(Command::TurnInterrupt {
            thread_id: "t".into(),
        })
        .unwrap();
    assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Interrupted);
    lock(&harness.engine.state).end_op(op, None).unwrap();

    assert!(harness.adapter.requests.lock().unwrap().is_empty());
    assert_eq!(harness.thread("t").status, ThreadStatus::Idle);
}

#[test]
fn creating_a_worktree_leaves_the_engine_free_for_other_threads() {
    let repo = git::tests::temp_repository("engine-slow-checkout");
    let hooks = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("post-checkout"), "#!/bin/sh\nsleep 2\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let hook = hooks.join("post-checkout");
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let harness = Harness::new(
        "slow-checkout",
        repo,
        Store::open_in_memory().unwrap(),
        completed_turn(),
    );

    let engine = &harness.engine;
    let (created_in, snapshot_in) = thread::scope(|scope| {
        let creating = scope.spawn(|| {
            let started = Instant::now();
            engine
                .dispatch(Command::ThreadCreate {
                    thread_id: "s1234567-rest".into(),
                    project_id: Some("p".into()),
                    title: String::new(),
                    workspace: ThreadWorkspace::NewWorktree,
                    provider: hoshi_core::default_provider(),
                    model: None,
                    effort: None,
                    fast_mode: false,
                    ultracode: false,
                    access_mode: AccessMode::AcceptEdits,
                    plan_mode: false,
                })
                .unwrap();
            started.elapsed()
        });
        thread::sleep(Duration::from_millis(500));
        let started = Instant::now();
        engine.snapshot();
        let snapshot_in = started.elapsed();
        (creating.join().unwrap(), snapshot_in)
    });

    assert!(created_in >= Duration::from_millis(1500), "{created_in:?}");
    assert!(snapshot_in < Duration::from_millis(500), "{snapshot_in:?}");
    assert!(harness.thread("s1234567-rest").worktree_path.is_some());
}

#[test]
fn a_task_in_its_own_worktree_is_created_off_the_lock() {
    let repo = git::tests::temp_repository("engine-slow-task");
    let hooks = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("post-checkout"), "#!/bin/sh\nsleep 2\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let hook = hooks.join("post-checkout");
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let harness = Harness::new(
        "slow-task",
        repo,
        Store::open_in_memory().unwrap(),
        Vec::new(),
    );
    let adapter = adapter(None);
    harness.engine.set_adapter(Some(adapter.clone()));
    harness.send("t", "m1", "split the work");
    let state = Arc::downgrade(&harness.engine.state);
    let engine = &harness.engine;

    let (task, snapshot_in) = thread::scope(|scope| {
        let delegating = scope.spawn(|| {
            let arguments =
                serde_json::json!({ "task": "count the files", "isolation": "worktree" });
            delegation::delegate_task(&state, "t", &arguments).unwrap()
        });
        thread::sleep(Duration::from_millis(500));
        let started = Instant::now();
        engine.snapshot();
        let snapshot_in = started.elapsed();
        (delegating.join().unwrap(), snapshot_in)
    });

    assert!(snapshot_in < Duration::from_millis(500), "{snapshot_in:?}");
    let child = harness.thread(task["taskId"].as_str().unwrap());
    assert_eq!(child.parent_thread_id.as_deref(), Some("t"));
    let worktree = child.worktree_path.unwrap();
    assert!(Path::new(&worktree).is_dir());
    let requests = adapter.requests.lock().unwrap();
    assert_eq!(requests[1].cwd, worktree);
    assert_eq!(requests[1].prompt, "count the files");
}

#[test]
fn a_wake_for_a_released_thread_creates_its_worktree_again() {
    let repo = git::tests::temp_repository("engine-wake-released");
    let harness = Harness::new(
        "wake-released",
        repo,
        Store::open_in_memory().unwrap(),
        completed_turn(),
    );
    harness
        .create_thread("r1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    let worktree = harness.thread("r1234567-rest").worktree_path.unwrap();
    harness.engine.release_worktree("r1234567-rest").unwrap();
    assert!(!Path::new(&worktree).exists());
    let pull_request = hoshi_core::PullRequest {
        forge: ForgeKind::Github,
        number: 7,
        title: "Work".into(),
        url: "https://github.com/o/r/pull/7".into(),
        author: Some("me".into()),
        head_branch: "hoshi/r1234567".into(),
        head_sha: Some("abc1234".into()),
        base_branch: "main".into(),
        state: hoshi_core::PullRequestState::Open,
        checks: None,
        unresolved_comments: 0,
        open_tasks: 0,
        changes_requested: false,
        turn: None,
        conflicts: false,
        additions: 0,
        deletions: 0,
        updated_at: "2026-11-07T12:00:00Z".into(),
    };
    lock(&harness.engine.state).pull_requests.record(
        "r1234567-rest",
        ForgeKind::Github,
        Ok(Some(pull_request.clone())),
        false,
        Instant::now(),
        chrono::Utc::now(),
    );
    lock(&harness.engine.state)
        .call_tool(
            "r1234567-rest",
            "watch_pull_request",
            &serde_json::json!({}),
        )
        .unwrap();
    let detail = hoshi_core::PullRequestDetail {
        supports: hoshi_core::PullRequestSupport::all(),
        forge_id: "id".into(),
        repository: "o/r".into(),
        mergeability: hoshi_core::Mergeability::Mergeable,
        merge_methods: Vec::new(),
        can_edit: true,
        body: String::new(),
        changed_files: 1,
        reviewers: Vec::new(),
        labels: Vec::new(),
        checks: vec![hoshi_core::PullRequestCheck {
            name: "lint".into(),
            state: hoshi_core::ChecksState::Failing,
            url: None,
        }],
        review_threads: Vec::new(),
        timeline: Vec::new(),
    };

    lock(&harness.engine.state).observe_watch(
        "r1234567-rest",
        ForgeKind::Github,
        &pull_request,
        &detail,
        Some("me".into()),
    );

    assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
    assert!(Path::new(&worktree).is_dir());
    let requests = harness.adapter.requests.lock().unwrap();
    assert_eq!(requests.last().unwrap().cwd, worktree);
}

fn wait_until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn an_existing_worktree_the_agent_enters_is_adopted_and_never_removed() {
    let repo = git::tests::temp_repository("engine-adopt-worktree");
    let worktree = repo.with_extension("adopt-wt");
    let _ = std::fs::remove_dir_all(&worktree);
    git::add_worktree(&repo.to_string_lossy(), &worktree, "feat/mine").unwrap();
    let path = git::normalize(&worktree.to_string_lossy());
    let script = vec![
        ProviderOutput::WorkingDirectory { path: path.clone() },
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    ];
    let harness = Harness::new(
        "adopt-worktree",
        repo,
        Store::open_in_memory().unwrap(),
        script,
    );

    harness.send("t", "m1", "work in my worktree");
    harness.wait_for_turn_end();
    wait_until(|| harness.thread("t").worktree_path.is_some());

    let thread = harness.thread("t");
    assert_eq!(thread.worktree_path.as_deref(), Some(path.as_str()));
    assert_eq!(thread.branch.as_deref(), Some("feat/mine"));
    assert!(thread.worktree_adopted);
    assert_eq!(
        harness.engine.release_worktree("t").unwrap_err(),
        CommandError::WorktreeAdopted {
            thread_id: "t".into()
        }
    );
    harness
        .engine
        .dispatch(Command::ThreadDelete {
            thread_id: "t".into(),
        })
        .unwrap();
    assert!(worktree.is_dir());
    let _ = git::remove_worktree(&harness.root, &path);
}

#[test]
fn a_branch_read_in_the_folder_a_thread_left_is_not_recorded() {
    let repo = git::tests::temp_repository("engine-stale-branch");
    let worktree = repo.with_extension("stale-wt");
    let _ = std::fs::remove_dir_all(&worktree);
    git::add_worktree(&repo.to_string_lossy(), &worktree, "feat/mine").unwrap();
    let path = git::normalize(&worktree.to_string_lossy());
    let script = vec![
        ProviderOutput::WorkingDirectory { path: path.clone() },
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    ];
    let store = Store::open_in_memory().unwrap();
    let harness = Harness::new("stale-branch", repo, store, script);
    harness.send("t", "m1", "work in my worktree");
    harness.wait_for_turn_end();
    wait_until(|| harness.thread("t").worktree_path.is_some());

    let recorded =
        lock(&harness.engine.state).record_branch_read("t", &harness.root, Some("main".into()));

    assert!(!recorded);
    assert_eq!(harness.thread("t").branch.as_deref(), Some("feat/mine"));
    let _ = git::remove_worktree(&harness.root, &path);
}

#[test]
fn leaving_the_worktree_brings_the_thread_back_to_the_project_root() {
    let repo = git::tests::temp_repository("engine-leave-worktree");
    let worktree = repo.with_extension("leave-wt");
    let _ = std::fs::remove_dir_all(&worktree);
    git::add_worktree(&repo.to_string_lossy(), &worktree, "feat/leave").unwrap();
    let path = git::normalize(&worktree.to_string_lossy());
    let root = git::normalize(&repo.to_string_lossy());
    let script = vec![
        ProviderOutput::WorkingDirectory { path: path.clone() },
        ProviderOutput::WorkingDirectory { path: root },
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    ];
    let harness = Harness::new(
        "leave-worktree",
        repo,
        Store::open_in_memory().unwrap(),
        script,
    );

    harness.send("t", "m1", "go in and out");
    harness.wait_for_turn_end();
    wait_until(|| lock(&harness.engine.state).session_moves.is_empty());
    // The root's branch is read in the background once the turn ends.
    wait_until(|| harness.thread("t").branch.is_some());

    let thread = harness.thread("t");
    assert_eq!(thread.worktree_path, None);
    assert_eq!(thread.branch.as_deref(), Some("main"));
    assert!(!thread.worktree_adopted);
    let _ = git::remove_worktree(&harness.root, &path);
}

#[test]
fn a_worktree_created_mid_turn_is_attached_before_the_turn_ends() {
    let repo = git::tests::temp_repository("engine-live-worktree");
    let worktree = repo.with_extension("live-wt");
    let _ = std::fs::remove_dir_all(&worktree);
    let folder = worktree.to_string_lossy().into_owned();
    let script = vec![
        bash(&format!("git worktree add {folder} -b chore/deps")),
        bash(&format!("cd {folder} && cargo update")),
    ];
    let store = Store::open_in_memory().unwrap();
    let harness = Harness::new("live-worktree", repo, store, script);
    harness.send("t", "m1", "make a worktree and keep going");
    // Lets the watcher register the new project's `.git` folder.
    thread::sleep(std::time::Duration::from_millis(500));

    git::add_worktree(&harness.root, &worktree, "chore/deps").unwrap();

    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    let attached = std::iter::from_fn(|| {
        let left = deadline.saturating_duration_since(Instant::now());
        harness.events.recv_timeout(left).ok()
    })
    .find_map(|event| match event {
        Event::ThreadWorktreeAttached { branch, .. } => Some(branch),
        _ => None,
    });
    assert_eq!(attached, Some(Some("chore/deps".into())));
    assert_eq!(harness.thread("t").status, ThreadStatus::Working);
    let _ = git::remove_worktree(&harness.root, &worktree.to_string_lossy());
}

#[test]
fn a_local_thread_records_the_branch_of_the_project_root() {
    let repo = git::tests::temp_repository("engine-branch");
    let store = Store::open_in_memory().unwrap();
    let harness = Harness::new("branch", repo, store, Vec::new());

    let branch = harness.wait_for(|event| match event {
        Event::ThreadBranchSet { branch, .. } => Some(branch),
        _ => None,
    });

    assert_eq!(branch.as_deref(), Some("main"));
}

#[test]
fn an_idle_local_thread_keeps_its_branch_when_the_root_switches() {
    let repo = git::tests::temp_repository("engine-root-switch");
    let store = Store::open_in_memory().unwrap();
    let harness = Harness::new("root-switch", repo, store, Vec::new());
    harness.wait_for(|event| match event {
        Event::ThreadBranchSet { thread_id, .. } if thread_id == "t" => Some(()),
        _ => None,
    });
    harness
        .create_thread("w1234567-rest", ThreadWorkspace::NewWorktree)
        .unwrap();
    let worktree = harness.thread("w1234567-rest").worktree_path.unwrap();

    source_control::create_branch(&harness.root, "fix").unwrap();
    source_control::create_branch(&worktree, "other").unwrap();
    refresh_projects(&Arc::downgrade(&harness.engine.state), &["p".into()]);

    assert_eq!(harness.thread("t").branch.as_deref(), Some("main"));
    assert_eq!(
        harness.thread("w1234567-rest").branch.as_deref(),
        Some("other")
    );
    assert_eq!(
        harness.engine.snapshot().project_branches,
        vec![hoshi_core::ProjectBranch {
            project_id: "p".into(),
            branch: Some("fix".into()),
        }]
    );
    let _ = git::remove_worktree(&harness.root, &worktree);
}

#[test]
fn a_worktree_needs_a_git_repository() {
    let harness = Harness::in_memory("no-git", Vec::new());

    let error = harness
        .create_thread("w", ThreadWorkspace::NewWorktree)
        .unwrap_err();

    assert!(matches!(error, CommandError::NotAGitRepository { .. }));
}

/// One agent process the test speaks for, kept after its turns while it says so.
#[derive(Default)]
struct KeptProcess {
    sink: Mutex<Option<OutputSink>>,
    spawned: Mutex<usize>,
    lingers: Mutex<bool>,
    awaits_work: Mutex<bool>,
    /// Whether a next turn may run in it.
    takes_turns: Mutex<bool>,
    continued: Mutex<Vec<String>>,
    stopped: Mutex<Vec<String>>,
    closed: Mutex<bool>,
}

impl KeptProcess {
    fn report(&self, output: ProviderOutput) {
        let mut sink = self.sink.lock().unwrap().take().unwrap();
        sink(output);
        *self.sink.lock().unwrap() = Some(sink);
    }
}

struct KeptAdapter(Arc<KeptProcess>);

struct KeptTurn(Arc<KeptProcess>);

impl RunningTurn for KeptTurn {
    fn interrupt(&self) {}

    fn respond(&self, _request_id: &str, _response: &RequestResponse) {}

    fn stop_background_task(&self, task_id: &str) {
        self.0.stopped.lock().unwrap().push(task_id.to_owned());
    }

    fn await_background_tasks(&self, task_ids: &[String], _awaited: bool) -> Option<Vec<String>> {
        Some(
            task_ids
                .iter()
                .filter(|task_id| *task_id == "b1")
                .cloned()
                .collect(),
        )
    }

    fn lingers(&self) -> bool {
        *self.0.lingers.lock().unwrap()
    }

    fn awaits_work(&self) -> bool {
        *self.0.awaits_work.lock().unwrap()
    }

    fn continue_with(&self, request: &TurnRequest) -> bool {
        if !*self.0.takes_turns.lock().unwrap() {
            return false;
        }
        self.0
            .continued
            .lock()
            .unwrap()
            .push(request.prompt.clone());
        true
    }

    fn close(&self) {
        *self.0.closed.lock().unwrap() = true;
    }
}

impl ProviderAdapter for KeptAdapter {
    fn provider(&self) -> &'static str {
        hoshi_core::DEFAULT_PROVIDER
    }

    fn models(&self) -> hoshi_core::ProviderModels {
        hoshi_core::ProviderModels::default()
    }

    fn start_turn(
        &self,
        _request: TurnRequest,
        sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        *self.0.spawned.lock().unwrap() += 1;
        *self.0.sink.lock().unwrap() = Some(sink);
        Ok(Box::new(KeptTurn(self.0.clone())))
    }
}

fn dev_server() -> BackgroundTask {
    BackgroundTask {
        task_id: "b1".into(),
        tool_call_id: Some("toolu_1".into()),
        description: "Start the dev server".into(),
        started_at: 1,
    }
}

/// Thread `t`'s first turn starts the dev server and completes, leaving it running.
fn leave_dev_server_running(name: &str) -> (Harness, Arc<KeptProcess>) {
    let harness = Harness::in_memory(name, Vec::new());
    let process = Arc::new(KeptProcess {
        takes_turns: Mutex::new(true),
        ..KeptProcess::default()
    });
    harness
        .engine
        .set_adapter(Some(Arc::new(KeptAdapter(process.clone()))));
    harness.send("t", "m1", "start the dev server");
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    process.report(ProviderOutput::SessionStarted {
        session_id: "s1".into(),
    });
    process.report(ProviderOutput::BackgroundTasks {
        tasks: vec![dev_server()],
        waiting: false,
    });
    *process.lingers.lock().unwrap() = true;
    process.report(ProviderOutput::TurnCompleted {
        outcome: TurnOutcome::Completed,
    });
    assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
    (harness, process)
}

#[test]
fn a_message_to_a_thread_awaiting_its_commands_runs_in_its_process_no_longer_waiting() {
    let (harness, process) = leave_dev_server_running("awaited-continue");
    lock(&harness.engine.state)
        .lingering
        .get_mut("t")
        .unwrap()
        .background
        .as_mut()
        .unwrap()
        .waiting = true;

    harness.send("t", "m2", "meanwhile, say hi");
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));

    assert_eq!(*process.continued.lock().unwrap(), ["meanwhile, say hi"]);
    let state = lock(&harness.engine.state);
    let background = state.turns["t"].background.as_ref().unwrap();
    assert!(!background.waiting);
    assert_eq!(background.tasks, [dev_server()]);
}

/// Thread `t` on `database` left the dev server running and hoshi quit, `awaited` when
/// its agent waited for it.
fn quit_with_dev_server_running(name: &str, database: &Path, awaited: bool) {
    let dir = database.parent().unwrap().to_owned();
    let harness = Harness::new(name, dir, Store::open(database).unwrap(), Vec::new());
    let process = Arc::new(KeptProcess::default());
    harness
        .engine
        .set_adapter(Some(Arc::new(KeptAdapter(process.clone()))));
    harness.send("t", "m1", "start the dev server");
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    process.report(ProviderOutput::SessionStarted {
        session_id: "s1".into(),
    });
    process.report(ProviderOutput::BackgroundTasks {
        tasks: vec![dev_server()],
        waiting: false,
    });
    if awaited {
        let arguments = serde_json::json!({ "backgroundTaskIds": ["b1"] });
        lock(&harness.engine.state)
            .call_tool("t", background::TOOL, &arguments)
            .unwrap();
    }
    *process.lingers.lock().unwrap() = true;
    process.report(ProviderOutput::TurnCompleted {
        outcome: TurnOutcome::Completed,
    });
    harness.wait_for_turn_end();
    harness.engine.end_agents();
    assert!(*process.closed.lock().unwrap());
}

#[test]
fn a_turn_quitting_cut_short_goes_on_though_its_agent_said_it_stopped() {
    let database = scratch_dir("quit-mid-turn").join("state.sqlite");
    let dir = database.parent().unwrap().to_owned();
    let harness = Harness::new(
        "quit-mid-turn",
        dir,
        Store::open(&database).unwrap(),
        Vec::new(),
    );
    let process = Arc::new(KeptProcess::default());
    harness
        .engine
        .set_adapter(Some(Arc::new(KeptAdapter(process.clone()))));
    harness.send("t", "m1", "refactor it");
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    process.report(ProviderOutput::SessionStarted {
        session_id: "s1".into(),
    });
    session_started(&harness);

    harness.engine.end_agents();
    process.report(ProviderOutput::TurnCompleted {
        outcome: TurnOutcome::Interrupted,
    });
    drop(harness);

    let (engine, adapter, events) = reopen("quit-mid-turn", &database);
    engine.continue_cut_threads();
    let turn_id = wait_for_turn_start(&engine, &events);
    let requests = adapter.requests.lock().unwrap().clone();
    assert_eq!(requests[0].resume.as_deref(), Some("s1"));
    assert_eq!(requests[0].prompt, recovery::CONTINUE_PROMPT);
    assert_eq!(
        resumption(&engine, &turn_id),
        Some(hoshi_core::Resumption::Restart)
    );
}

#[test]
fn a_command_the_agent_waited_for_that_quitting_ended_has_it_go_on() {
    let database = scratch_dir("quit-awaited").join("state.sqlite");
    quit_with_dev_server_running("quit-awaited", &database, true);

    let (engine, adapter, events) = reopen("quit-awaited", &database);
    engine.continue_cut_threads();
    let turn_id = wait_for_turn_start(&engine, &events);
    assert_eq!(
        resumption(&engine, &turn_id),
        Some(hoshi_core::Resumption::RestartEndedWork)
    );

    assert_eq!(
        prompts(&adapter),
        [format!(
            "{}
- shell: Start the dev server (id b1)",
            recovery::CUT_WORK_NOTE
        )]
    );
    assert_eq!(
        adapter.requests.lock().unwrap()[0].resume.as_deref(),
        Some("s1")
    );
}

#[test]
fn quitting_ends_the_commands_left_running_and_the_next_message_says_so() {
    let database = scratch_dir("quit-kept").join("state.sqlite");
    quit_with_dev_server_running("quit-kept", &database, false);

    let (engine, adapter, events) = reopen("quit-kept", &database);
    engine.continue_cut_threads();
    assert!(adapter.requests.lock().unwrap().is_empty());
    engine
        .dispatch(Command::MessageSend {
            thread_id: "t".into(),
            message_id: "m2".into(),
            text: "is it up?".into(),
            attachments: Vec::new(),
        })
        .unwrap();
    wait_for_turn_start(&engine, &events);

    assert_eq!(
        prompts(&adapter),
        [format!(
            "{}
- shell: Start the dev server (id b1)

User message:
is it up?",
            recovery::CUT_WORK_NOTE
        )]
    );
}

fn running_in_background(harness: &Harness) -> Vec<ThreadBackgroundTasks> {
    harness.engine.snapshot().background_tasks
}

#[test]
fn a_retry_clears_once_the_answer_starts_streaming() {
    let harness = Harness::in_memory("retry-stream", Vec::new());
    let process = Arc::new(KeptProcess::default());
    harness
        .engine
        .set_adapter(Some(Arc::new(KeptAdapter(process.clone()))));
    harness.send("t", "m1", "go");
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));

    process.report(ProviderOutput::Retrying {
        attempt: 1,
        max_attempts: Some(10),
        retry_in_ms: Some(500),
        error: Some("overloaded".into()),
    });
    let retry = harness.wait_for(|event| match event {
        Event::TurnRetrying { retry, .. } => Some(retry),
        _ => None,
    });
    assert_eq!(retry.map(|retry| retry.attempt), Some(1));

    process.report(ProviderOutput::TextDelta {
        item_id: "a1".into(),
        delta: "Hi".into(),
    });
    process.report(ProviderOutput::TurnCompleted {
        outcome: TurnOutcome::Completed,
    });
    let cleared = harness.wait_for(|event| match event {
        Event::TurnRetrying { retry, .. } => Some(retry.is_none()),
        Event::TurnCompleted { .. } => Some(false),
        _ => None,
    });
    assert!(cleared, "the retry outlived the streamed answer");
    let recovered = harness.wait_for(|event| match event {
        Event::ItemAdded { item } => match item.content {
            ItemContent::ProviderRecovered {
                attempt,
                max_attempts,
            } => Some(Some((attempt, max_attempts))),
            _ => None,
        },
        Event::TurnCompleted { .. } => Some(None),
        _ => None,
    });
    assert_eq!(recovered, Some((1, Some(10))));
}

#[test]
fn a_turn_that_leaves_a_command_running_ends_and_its_process_takes_the_next_message() {
    let (harness, process) = leave_dev_server_running("kept-process");

    assert_eq!(harness.thread("t").status, ThreadStatus::Idle);
    assert_eq!(
        running_in_background(&harness),
        [ThreadBackgroundTasks {
            thread_id: "t".into(),
            tasks: vec![dev_server()],
            waiting: false,
        }]
    );

    harness.send("t", "m2", "is it up?");
    let started = harness.wait_for(|event| match event {
        Event::TurnStarted { turn_id, .. } => Some(turn_id),
        _ => None,
    });
    assert_eq!(started, "m2");
    assert_eq!(*process.spawned.lock().unwrap(), 1);
    assert_eq!(*process.continued.lock().unwrap(), ["is it up?"]);

    process.report(ProviderOutput::AssistantMessage {
        item_id: "a2".into(),
        text: "It is.".into(),
    });
    process.report(ProviderOutput::TurnCompleted {
        outcome: TurnOutcome::Completed,
    });
    assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);
    assert_eq!(running_in_background(&harness).len(), 1);
}

#[test]
fn a_command_left_running_that_ends_wakes_the_agent_in_a_turn_of_its_own() {
    let (harness, process) = leave_dev_server_running("kept-process-wake");

    process.report(ProviderOutput::BackgroundTasks {
        tasks: Vec::new(),
        waiting: false,
    });
    process.report(ProviderOutput::Woke);
    let woken = harness.wait_for(|event| match event {
        Event::TurnStarted { turn_id, .. } => Some(turn_id),
        _ => None,
    });
    assert_ne!(woken, "m1");
    assert_eq!(harness.thread("t").status, ThreadStatus::Working);
    process.report(ProviderOutput::BackgroundTaskEnded {
        task_id: "b1".into(),
        tool_use_id: Some("toolu_1".into()),
        failed: true,
    });
    *process.lingers.lock().unwrap() = false;
    process.report(ProviderOutput::TurnCompleted {
        outcome: TurnOutcome::Completed,
    });
    assert_eq!(harness.wait_for_turn_end(), TurnOutcome::Completed);

    let items = harness.engine.transcript("t", None).unwrap().items;
    assert!(
        items
            .iter()
            .any(|item| item.id == format!("{woken}:ended:b1"))
    );
    assert!(running_in_background(&harness).is_empty());
    assert!(lock(&harness.engine.state).lingering.is_empty());
}

#[test]
fn a_process_kept_between_turns_is_forgotten_once_it_ends() {
    let (harness, process) = leave_dev_server_running("kept-process-ends");

    process.report(ProviderOutput::ProcessEnded);

    assert!(running_in_background(&harness).is_empty());
    assert!(lock(&harness.engine.state).lingering.is_empty());
    harness.send("t", "m2", "again");
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    assert_eq!(*process.spawned.lock().unwrap(), 2);
}

#[test]
fn a_command_left_running_can_be_stopped_between_turns() {
    let (harness, process) = leave_dev_server_running("kept-process-stop");

    harness
        .engine
        .dispatch(Command::BackgroundTaskStop {
            thread_id: "t".into(),
            task_id: "b1".into(),
        })
        .unwrap();

    assert_eq!(*process.stopped.lock().unwrap(), ["b1"]);
}

#[test]
fn a_next_message_the_kept_process_cannot_take_waits_for_its_commands_to_end() {
    let (harness, process) = leave_dev_server_running("kept-process-refuses");
    *process.takes_turns.lock().unwrap() = false;

    harness.send("t", "m2", "now with another model");
    assert_eq!(
        harness.wait_for_turn_end(),
        TurnOutcome::Failed {
            reason: TurnFailure::BackgroundWorkRunning
        }
    );
    assert!(!*process.closed.lock().unwrap());
    assert_eq!(*process.spawned.lock().unwrap(), 1);
    assert_eq!(running_in_background(&harness).len(), 1);

    process.report(ProviderOutput::BackgroundTasks {
        tasks: Vec::new(),
        waiting: false,
    });
    harness.send("t", "m3", "now with another model");
    harness.wait_for(|event| match event {
        Event::TurnStarted { turn_id, .. } if turn_id == "m3" => Some(()),
        _ => None,
    });

    assert!(*process.closed.lock().unwrap());
    assert_eq!(*process.spawned.lock().unwrap(), 2);
    assert!(running_in_background(&harness).is_empty());
}

#[test]
fn archiving_or_deleting_a_thread_ends_the_process_its_turn_kept() {
    let (harness, process) = leave_dev_server_running("kept-process-archive");
    harness
        .engine
        .dispatch(Command::ThreadArchive {
            thread_id: "t".into(),
            archived: true,
        })
        .unwrap();
    assert!(*process.closed.lock().unwrap());
    assert!(running_in_background(&harness).is_empty());

    let (harness, process) = leave_dev_server_running("kept-process-delete");
    harness
        .engine
        .dispatch(Command::ThreadDelete {
            thread_id: "t".into(),
        })
        .unwrap();
    assert!(*process.closed.lock().unwrap());
    assert!(lock(&harness.engine.state).lingering.is_empty());
}

#[test]
fn a_kept_process_that_stays_quiet_too_long_is_ended() {
    let (harness, process) = leave_dev_server_running("kept-process-quiet");
    let now = SystemTime::now();

    lock(&harness.engine.state).close_idle_lingering(now + Duration::from_secs(60 * 60));
    assert!(!*process.closed.lock().unwrap(), "its dev server runs");

    lock(&harness.engine.state).close_idle_lingering(now + Duration::from_secs(5 * 60 * 60));
    assert!(*process.closed.lock().unwrap());
    assert!(lock(&harness.engine.state).lingering.is_empty());
    assert!(running_in_background(&harness).is_empty());
}

#[test]
fn a_kept_process_whose_commands_ended_is_ended_after_half_an_hour_quiet() {
    let (harness, process) = leave_dev_server_running("kept-process-done");
    process.report(ProviderOutput::BackgroundTasks {
        tasks: Vec::new(),
        waiting: false,
    });
    let now = SystemTime::now();

    lock(&harness.engine.state).close_idle_lingering(now + Duration::from_secs(20 * 60));
    assert!(!*process.closed.lock().unwrap());

    lock(&harness.engine.state).close_idle_lingering(now + Duration::from_secs(31 * 60));
    assert!(*process.closed.lock().unwrap());
}

#[test]
fn a_kept_process_with_a_run_ahead_may_stay_quiet_as_long_as_one_with_commands() {
    let (harness, process) = leave_dev_server_running("kept-process-wakeup");
    process.report(ProviderOutput::BackgroundTasks {
        tasks: Vec::new(),
        waiting: false,
    });
    *process.awaits_work.lock().unwrap() = true;
    let now = SystemTime::now();

    lock(&harness.engine.state).close_idle_lingering(now + Duration::from_secs(60 * 60));
    assert!(!*process.closed.lock().unwrap(), "its wakeup is ahead");

    lock(&harness.engine.state).close_idle_lingering(now + Duration::from_secs(5 * 60 * 60));
    assert!(*process.closed.lock().unwrap());
}

#[test]
fn a_next_message_starts_a_new_process_when_the_kept_one_went_quiet_too_long() {
    let (harness, process) = leave_dev_server_running("kept-process-slept");
    process.report(ProviderOutput::BackgroundTasks {
        tasks: Vec::new(),
        waiting: false,
    });
    lock(&harness.engine.state)
        .lingering
        .get_mut("t")
        .unwrap()
        .heard_at = SystemTime::now() - Duration::from_secs(24 * 60 * 60);

    harness.send("t", "m2", "good morning");
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));

    assert!(*process.closed.lock().unwrap());
    assert!(process.continued.lock().unwrap().is_empty());
    assert_eq!(*process.spawned.lock().unwrap(), 2);
}

#[test]
fn what_a_kept_process_reports_counts_as_heard() {
    let (harness, process) = leave_dev_server_running("kept-process-heard");
    let heard = || lock(&harness.engine.state).lingering["t"].heard_at;
    let before = heard();
    thread::sleep(Duration::from_millis(20));

    process.report(ProviderOutput::BackgroundTasks {
        tasks: vec![dev_server()],
        waiting: true,
    });

    assert!(heard() > before);
}

#[test]
fn ending_the_agents_ends_running_turns_and_kept_processes() {
    let (harness, process) = leave_dev_server_running("kept-process-quit");
    harness.send("t", "m2", "is it up?");
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    assert!(lock(&harness.engine.state).turns.contains_key("t"));

    harness.engine.end_agents();

    assert!(*process.closed.lock().unwrap());
}

#[test]
fn the_agent_awaits_its_background_commands_while_its_turn_runs() {
    let (harness, _process) = leave_dev_server_running("kept-process-await");
    let call = |harness: &Harness| {
        lock(&harness.engine.state).call_tool(
            "t",
            background::TOOL,
            &serde_json::json!({ "backgroundTaskIds": ["b1", "b9"] }),
        )
    };
    assert!(call(&harness).is_err(), "no turn runs");

    harness.send("t", "m2", "run the tests");
    harness.wait_for(|event| matches!(event, Event::TurnStarted { .. }).then_some(()));
    let answer = call(&harness).unwrap();

    assert_eq!(answer["backgroundTaskIds"], serde_json::json!(["b1"]));
    assert_eq!(answer["notRunning"], serde_json::json!(["b9"]));
    assert!(answer["next"].is_string());
}

/// Lists `live` and knows `offline` without running anything.
struct CatalogAdapter {
    provider: &'static str,
    live: Vec<&'static str>,
    offline: Option<Vec<&'static str>>,
}

fn catalog_of(provider: &str, slugs: &[&str]) -> hoshi_core::ProviderModels {
    hoshi_core::ProviderModels {
        provider: provider.into(),
        default_model: slugs.first().map(|slug| (*slug).to_owned()),
        models: slugs
            .iter()
            .map(|slug| hoshi_core::ModelInfo {
                slug: (*slug).into(),
                name: format!("Model {slug}"),
                short_name: (*slug).into(),
                legacy: false,
                is_new: false,
                update_required: false,
                efforts: Vec::new(),
                default_effort: None,
                fast_mode: false,
                ultracode: false,
            })
            .collect(),
        compacts_idle_threads: false,
    }
}

impl ProviderAdapter for CatalogAdapter {
    fn provider(&self) -> &'static str {
        self.provider
    }

    fn models(&self) -> hoshi_core::ProviderModels {
        catalog_of(self.provider, &self.live)
    }

    fn offline_models(&self) -> Option<hoshi_core::ProviderModels> {
        self.offline
            .as_ref()
            .map(|slugs| catalog_of(self.provider, slugs))
    }

    fn compacts_idle_threads(&self) -> bool {
        self.provider == DEFAULT_PROVIDER
    }

    fn start_turn(
        &self,
        _request: TurnRequest,
        _sink: OutputSink,
    ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
        Err(TurnFailure::ProviderDisabled)
    }
}

fn catalog_adapter(
    provider: &'static str,
    live: &[&'static str],
    offline: Option<&[&'static str]>,
) -> Arc<dyn ProviderAdapter> {
    Arc::new(CatalogAdapter {
        provider,
        live: live.to_vec(),
        offline: offline.map(<[_]>::to_vec),
    })
}

fn slugs(catalog: &hoshi_core::ProviderModels) -> Vec<&str> {
    catalog
        .models
        .iter()
        .map(|model| model.slug.as_str())
        .collect()
}

fn catalog_engine(data_dir: &Path) -> Engine {
    Engine::from_store(Store::open_in_memory().unwrap(), data_dir, Arc::new(|_| {})).unwrap()
}

#[test]
fn the_models_listed_last_run_are_known_at_once_after_a_restart() {
    let data_dir = scratch_dir("known-models");
    let engine = catalog_engine(&data_dir);
    engine.set_adapters(vec![
        catalog_adapter("copilot", &["gpt-6"], None),
        catalog_adapter(DEFAULT_PROVIDER, &["claude-haiku-5-5"], None),
    ]);
    assert!(engine.known_provider_models().is_empty());
    engine.provider_models();

    let restarted = catalog_engine(&data_dir);
    restarted.set_adapters(vec![
        catalog_adapter("copilot", &[], None),
        catalog_adapter(DEFAULT_PROVIDER, &[], None),
    ]);
    let known = restarted.known_provider_models();

    let providers: Vec<_> = known
        .iter()
        .map(|catalog| catalog.provider.as_str())
        .collect();
    assert_eq!(providers, [DEFAULT_PROVIDER, "copilot"]);
    assert_eq!(slugs(&known[0]), ["claude-haiku-5-5"]);
    assert_eq!(known[0].models[0].name, "Model claude-haiku-5-5");
    assert!(known[0].compacts_idle_threads);
    assert_eq!(slugs(&known[1]), ["gpt-6"]);
    assert!(
        data_dir
            .join("caches")
            .join("providers")
            .join("copilot.json")
            .exists()
    );
}

#[test]
fn a_listing_that_comes_back_empty_keeps_the_models_known_before() {
    let data_dir = scratch_dir("known-models-empty");
    let engine = catalog_engine(&data_dir);
    engine.set_adapters(vec![catalog_adapter("copilot", &["gpt-6"], None)]);
    engine.provider_models();

    engine.set_adapters(vec![catalog_adapter("copilot", &[], None)]);
    engine.provider_models();

    assert_eq!(slugs(&engine.known_provider_models()[0]), ["gpt-6"]);
}

#[test]
fn a_disabled_provider_s_kept_models_are_left_out() {
    let data_dir = scratch_dir("known-models-disabled");
    let engine = catalog_engine(&data_dir);
    engine.set_adapters(vec![
        catalog_adapter(DEFAULT_PROVIDER, &["opus"], None),
        catalog_adapter("copilot", &["gpt-6"], None),
    ]);
    engine.provider_models();

    engine.set_adapters(vec![catalog_adapter(DEFAULT_PROVIDER, &[], None)]);
    let known = engine.known_provider_models();

    assert_eq!(known.len(), 1);
    assert_eq!(known[0].provider, DEFAULT_PROVIDER);
}

#[test]
fn the_offline_catalog_comes_first_and_kept_models_add_what_it_lacks() {
    let data_dir = scratch_dir("known-models-offline");
    let engine = catalog_engine(&data_dir);
    engine.set_adapters(vec![catalog_adapter(
        DEFAULT_PROVIDER,
        &["haiku", "opus"],
        None,
    )]);
    engine.provider_models();

    engine.set_adapters(vec![catalog_adapter(
        DEFAULT_PROVIDER,
        &[],
        Some(&["opus", "sonnet"]),
    )]);
    let known = engine.known_provider_models();

    assert_eq!(slugs(&known[0]), ["opus", "sonnet", "haiku"]);
    assert_eq!(known[0].default_model.as_deref(), Some("opus"));
}

#[test]
fn an_offline_catalog_is_known_before_any_listing() {
    let engine = catalog_engine(&scratch_dir("known-models-first-run"));
    engine.set_adapters(vec![
        catalog_adapter("copilot", &["gpt-6"], None),
        catalog_adapter(DEFAULT_PROVIDER, &["opus"], Some(&["opus"])),
    ]);

    let known = engine.known_provider_models();

    assert_eq!(known.len(), 1);
    assert_eq!(slugs(&known[0]), ["opus"]);
}

#[test]
fn a_kept_file_holding_another_provider_s_models_is_ignored() {
    let data_dir = scratch_dir("known-models-identity");
    let dir = provider_cache::dir(&data_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("codex.json"),
        serde_json::to_string(&catalog_of("copilot", &["gpt-6"])).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join(format!("{DEFAULT_PROVIDER}.json")), "not json").unwrap();
    let engine = catalog_engine(&data_dir);
    engine.set_adapters(vec![
        catalog_adapter("codex", &[], None),
        catalog_adapter(DEFAULT_PROVIDER, &[], None),
    ]);

    assert!(engine.known_provider_models().is_empty());
}
