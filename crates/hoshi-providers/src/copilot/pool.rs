use super::*;

/// A `copilot --acp` process, and the turn it runs while it runs one.
pub(super) struct Live {
    pub(super) agent: Agent,
    connection: Connection,
    turn: Arc<Mutex<Option<Arc<Turn>>>>,
}

impl Live {
    pub(super) fn start(binary: &Path, launch: &Launch) -> Result<Arc<Self>, TurnFailure> {
        let mut agent = Agent::spawn(launch.command(binary))?;
        let stderr = agent.stderr();
        let turn: Arc<Mutex<Option<Arc<Turn>>>> = Arc::default();
        let replies: Arc<OnceLock<Connection>> = Arc::default();
        let (reader, ended, idle) = (turn.clone(), turn.clone(), replies.clone());
        let connection = agent.connect(
            move |incoming| {
                let current = lock(&reader).clone();
                match (current, incoming) {
                    (Some(turn), incoming) => turn.receive(incoming),
                    (None, Incoming::Request { id, method, .. }) => {
                        let Some(connection) = idle.get() else {
                            return;
                        };
                        if method == "session/request_permission" {
                            connection.reply(&id, json!({ "outcome": { "outcome": "cancelled" } }));
                        } else {
                            connection.reply_error(&id, -32601, "not supported by this client");
                        }
                    }
                    (None, Incoming::Notification { .. }) => {}
                }
            },
            move || {
                let stderr = stderr.join().unwrap_or_default();
                if let Some(turn) = lock(&ended).clone() {
                    turn.process_ended(stderr);
                }
            },
        );
        let _ = replies.set(connection.clone());
        Ok(Arc::new(Self {
            agent,
            connection,
            turn,
        }))
    }

    pub(super) fn attach(&self, turn: &Arc<Turn>) {
        *lock(&turn.connection) = Some(self.connection.clone());
        *lock(&self.turn) = Some(turn.clone());
    }

    pub(super) fn detach(&self) {
        lock(&self.turn).take();
    }

    pub(super) fn running(&self) -> bool {
        !self.agent.exited.load(Ordering::SeqCst)
    }

    pub(super) fn end(&self) {
        self.detach();
        self.connection.close();
        self.agent.kill();
    }
}

/// The processes kept after their turn completed, by the session they hold.
#[derive(Default)]
pub(super) struct Pool {
    pub(super) kept: Mutex<HashMap<String, Kept>>,
    sweeping: AtomicBool,
}

pub(super) struct Kept {
    launch: Launch,
    pub(super) live: Arc<Live>,
    since: Instant,
}

impl Pool {
    /// The process kept for `session_id`, when it was started as `launch` asks and still runs.
    /// One that wasn't ends.
    pub(super) fn take(&self, session_id: &str, launch: &Launch) -> Option<Arc<Live>> {
        let kept = lock(&self.kept).remove(session_id)?;
        if kept.launch == *launch && kept.live.running() {
            Some(kept.live)
        } else {
            kept.live.end();
            None
        }
    }

    pub(super) fn keep(self: &Arc<Self>, session_id: String, launch: Launch, live: Arc<Live>) {
        let kept = Kept {
            launch,
            live,
            since: Instant::now(),
        };
        if let Some(previous) = lock(&self.kept).insert(session_id, kept) {
            previous.live.end();
        }
        if self.sweeping.swap(true, Ordering::SeqCst) {
            return;
        }
        let pool = Arc::downgrade(self);
        thread::spawn(move || {
            loop {
                thread::sleep(IDLE_SWEEP);
                let Some(pool) = pool.upgrade() else {
                    return;
                };
                pool.end_idle(IDLE_TIMEOUT);
            }
        });
    }

    /// Ends the processes kept for longer than `timeout`.
    pub(super) fn end_idle(&self, timeout: Duration) {
        let ended: Vec<Kept> = {
            let mut kept = lock(&self.kept);
            let expired: Vec<String> = kept
                .iter()
                .filter(|(_, kept)| kept.since.elapsed() >= timeout || !kept.live.running())
                .map(|(session_id, _)| session_id.clone())
                .collect();
            expired
                .iter()
                .filter_map(|session_id| kept.remove(session_id))
                .collect()
        };
        for kept in ended {
            kept.live.end();
        }
    }

    pub(super) fn end_all(&self) {
        let ended: Vec<Kept> = lock(&self.kept).drain().map(|(_, kept)| kept).collect();
        for kept in ended {
            kept.live.end();
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.end_all();
    }
}

/// Runs a turn in its process, and keeps the process for the session's next turn once it
/// completes.
pub(super) struct Work {
    pub(super) turn: Arc<Turn>,
    pub(super) live: Arc<Mutex<Arc<Live>>>,
    pub(super) idle: Weak<Pool>,
    pub(super) binary: PathBuf,
    pub(super) launch: Launch,
}

impl Work {
    pub(super) fn run(self, request: &TurnRequest, mut reused: bool) {
        loop {
            let live = lock(&self.live).clone();
            live.attach(&self.turn);
            self.turn.reused.store(reused, Ordering::SeqCst);
            let attempt = self.turn.run(&live.connection, request, reused);
            live.detach();
            let (outcome, reusable) = match attempt {
                Attempt::Retry => {
                    live.end();
                    match Live::start(&self.binary, &self.launch) {
                        Ok(fresh) => {
                            *lock(&self.live) = fresh;
                            reused = false;
                            continue;
                        }
                        Err(reason) => (TurnOutcome::Failed { reason }, false),
                    }
                }
                Attempt::Done { outcome, reusable } => (outcome, reusable),
            };
            let session_id = lock(&self.turn.session_id).clone();
            let keep = reusable
                && live.running()
                && !self.turn.closed.load(Ordering::SeqCst)
                && lock(&self.turn.subagents).is_empty();
            match (session_id, self.idle.upgrade()) {
                (Some(session_id), Some(idle)) if keep => idle.keep(session_id, self.launch, live),
                _ => live.end(),
            }
            self.turn.finish(outcome);
            return;
        }
    }
}

/// How one try at a turn ended.
pub(super) enum Attempt {
    Done {
        outcome: TurnOutcome,
        /// Whether its process is idle and can take the session's next turn.
        reusable: bool,
    },
    /// The kept process had lost the session before it did anything: the turn goes on in a
    /// new one.
    Retry,
}

/// Whether a kept process failed `error` for having lost its session, or by ending.
pub(super) fn lost(error: &RpcError) -> bool {
    let local = error.code == 0 && error.data.is_null();
    let detail = error.detail();
    local || LOST_SESSION.iter().any(|marker| detail.contains(marker))
}

/// What a probe session offered.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Probe {
    /// Slug and name.
    pub(super) models: Vec<(String, Option<String>)>,
    pub(super) current_model: Option<String>,
    pub(super) efforts: Vec<Effort>,
    pub(super) commands: Vec<SlashCommand>,
}

impl Probe {
    pub(super) fn read(&mut self, update: &Value) {
        match update["sessionUpdate"].as_str() {
            Some("config_option_update") => self.read_options(&update["configOptions"]),
            Some("available_commands_update") => {
                self.commands = update["availableCommands"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|command| {
                        Some(SlashCommand {
                            name: command["name"].as_str()?.to_owned(),
                            description: command["description"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                            argument_hint: command["input"]["hint"].as_str().map(str::to_owned),
                            aliases: Vec::new(),
                            custom: false,
                        })
                    })
                    .collect();
            }
            _ => {}
        }
    }

    /// The model and effort choices among a session's options, when it offers them.
    pub(super) fn read_options(&mut self, options: &Value) {
        for option in options.as_array().into_iter().flatten() {
            let choices = || {
                option["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|choice| {
                        Some((
                            choice["value"].as_str()?.to_owned(),
                            choice["name"].as_str().map(str::to_owned),
                        ))
                    })
                    .collect::<Vec<_>>()
            };
            match option["category"].as_str() {
                Some("model") => {
                    self.models = choices();
                    self.current_model = option["currentValue"].as_str().map(str::to_owned);
                }
                Some("thought_level") => {
                    self.efforts = choices()
                        .into_iter()
                        .filter_map(|(value, _)| effort(&value))
                        .collect();
                }
                _ => {}
            }
        }
    }
}
