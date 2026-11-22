use super::*;

/// How long Claude Code may take to start, then to connect hoshi's MCP server.
#[derive(Debug, Clone, Copy)]
pub(super) struct Deadlines {
    pub(super) boot: Duration,
    pub(super) mcp: Duration,
}

pub(super) const STARTUP: Deadlines = Deadlines {
    boot: Duration::from_secs(5 * 60),
    mcp: Duration::from_secs(90),
};

/// What the turn's process said so far that shows it started.
pub(super) enum Milestone {
    /// Its first line: it runs and reads what it is sent.
    Booted,
    /// It answered the request that gives it hoshi's MCP server.
    Registered,
}

/// What a turn's process is started from, to start it again when it doesn't get going.
pub(super) struct Startup {
    pub(super) command: Command,
    pub(super) prompt: String,
    pub(super) attachments: Vec<TurnAttachment>,
    pub(super) earlier_media: Vec<TurnAttachment>,
    pub(super) mcp: Option<McpServer>,
    pub(super) model: Option<String>,
    pub(super) effort: Option<Effort>,
    pub(super) queue: Queue,
    pub(super) requests: Requests,
    pub(super) live_output: LiveOutput,
    pub(super) sink: SharedSink,
    pub(super) process: LiveProcess,
    /// A resumed session gets a second try.
    pub(super) retries: bool,
    pub(super) deadlines: Deadlines,
}

impl Startup {
    /// Starts attempt `attempt` of the turn's process, which opens the turn with a message of
    /// its own.
    pub(super) fn launch(self: &Arc<Self>, attempt: u32) -> Result<(), TurnFailure> {
        let message = {
            let mut state = self.queue.state();
            let mut media = MediaBudget::after(&self.earlier_media);
            let message = user_message(&self.prompt, &self.attachments, &mut media, &mut state);
            state.media = media;
            message
        };
        // Over stdin rather than argv, which keeps the token out of the process list. The CLI
        // connects before it reads the message, so the first turn has the tools.
        let input = match &self.mcp {
            Some(server) => format!("{}\n{message}", mcp_servers(server)),
            None => message,
        };
        let mut parser = ClaudeParser {
            requests: self.requests.clone(),
            live_output: self.live_output.clone(),
            queue: self.queue.clone(),
            ..ClaudeParser::for_turn(self.model.as_deref(), self.effort)
        };
        let (milestones, heard) = mpsc::channel();
        let (mut booted, mut registered) = (false, self.mcp.is_none());
        let (ending, in_turn) = (self.queue.clone(), self.queue.clone());
        let process = spawn_turn(
            rebuilt(&self.command),
            input,
            self.sink.for_attempt(attempt, self.queue.clone()),
            move |line, stdin| {
                if !booted {
                    booted = true;
                    let _ = milestones.send(Milestone::Booted);
                }
                if !registered && line.contains(MCP_REQUEST_ID) {
                    let value: Value = serde_json::from_str(line).unwrap_or_default();
                    if value["type"] == "control_response"
                        && value["response"]["request_id"] == MCP_REQUEST_ID
                    {
                        registered = true;
                        let _ = milestones.send(Milestone::Registered);
                    }
                }
                let outputs = parser.parse_line(line);
                for reply in parser.take_replies() {
                    stdin.write_line(&reply);
                }
                if ending.state().closing {
                    stdin.close();
                }
                outputs
            },
            move || in_turn.state().turn_open,
        )?;
        self.live_output.poll(process.stdin().clone());
        self.process.replace(process);
        let startup = Arc::clone(self);
        thread::spawn(move || startup.watch(attempt, &heard));
        Ok(())
    }

    /// Gives up on attempt `attempt` when it doesn't start in time: tries once more for a
    /// resumed session, then fails the turn.
    pub(super) fn watch(self: &Arc<Self>, attempt: u32, heard: &Receiver<Milestone>) {
        let mut deadline = Instant::now() + self.deadlines.boot;
        let (mut booted, mut registered) = (false, self.mcp.is_none());
        while !(booted && registered) {
            match heard.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Milestone::Booted) => {
                    booted = true;
                    deadline = Instant::now() + self.deadlines.mcp;
                }
                Ok(Milestone::Registered) => registered = true,
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => break,
            }
        }
        if booted && registered {
            return;
        }
        {
            let queue = self.queue.state();
            if queue.interrupted || queue.closing || !queue.turn_open {
                return;
            }
        }
        let detail = if booted {
            format!(
                "hoshi's MCP server was not connected after {}s",
                self.deadlines.mcp.as_secs()
            )
        } else {
            format!(
                "Claude Code printed nothing for {}s",
                self.deadlines.boot.as_secs()
            )
        };
        if self.retries && attempt == 0 {
            self.sink.supersede(attempt + 1);
            self.process.interrupt();
            if let Err(reason) = self.launch(attempt + 1) {
                self.sink.send(ProviderOutput::TurnCompleted {
                    outcome: TurnOutcome::Failed { reason },
                });
            }
            return;
        }
        self.sink.time_out(detail);
        self.process.interrupt();
    }
}

/// The same command again: a `Command` spawns once per call, but only its parts can be
/// copied.
pub(super) fn rebuilt(command: &Command) -> Command {
    let mut copy = Command::new(command.get_program());
    copy.args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => copy.env(key, value),
            None => copy.env_remove(key),
        };
    }
    if let Some(dir) = command.get_current_dir() {
        copy.current_dir(dir);
    }
    copy
}

/// The turn's process, which a retried start replaces.
#[derive(Clone, Default)]
pub(super) struct LiveProcess(Arc<Mutex<Option<ProcessTurn>>>);

impl LiveProcess {
    pub(super) fn current(&self) -> Option<ProcessTurn> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(super) fn replace(&self, process: ProcessTurn) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(process);
    }

    pub(super) fn stdin(&self) -> Stdin {
        self.current()
            .map(|process| process.stdin().clone())
            .unwrap_or_default()
    }

    /// Kills the process, if it still runs.
    pub(super) fn interrupt(&self) {
        if let Some(process) = self.current() {
            process.interrupt();
        }
    }
}

/// The turn's sink, shared by the attempts at starting its process: only the latest one
/// reports, and how the turn ends is read from what the process left behind.
#[derive(Clone)]
pub(super) struct SharedSink(Arc<Mutex<SinkState>>);

pub(super) struct SinkState {
    pub(super) sink: OutputSink,
    pub(super) attempt: u32,
    /// The process was killed for not starting in time, as `detail` says.
    pub(super) timed_out: Option<String>,
}

impl SharedSink {
    pub(super) fn new(sink: OutputSink) -> Self {
        Self(Arc::new(Mutex::new(SinkState {
            sink,
            attempt: 0,
            timed_out: None,
        })))
    }

    pub(super) fn state(&self) -> MutexGuard<'_, SinkState> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn for_attempt(&self, attempt: u32, queue: Queue) -> OutputSink {
        let shared = self.clone();
        Box::new(move |output| {
            let mut state = shared.state();
            if state.attempt != attempt {
                return;
            }
            let ProviderOutput::TurnCompleted { outcome } = output else {
                (state.sink)(output);
                return;
            };
            let outcome = ended_process(outcome, state.timed_out.take());
            if outcome != TurnOutcome::Completed {
                let dropped = queue.state().dropped(true);
                if !dropped.is_empty() {
                    (state.sink)(ProviderOutput::MessagesDropped {
                        message_ids: dropped,
                    });
                }
            }
            (state.sink)(ProviderOutput::TurnCompleted { outcome });
        })
    }

    /// Attempt `attempt` takes over: what the earlier ones still report is dropped.
    pub(super) fn supersede(&self, attempt: u32) {
        self.state().attempt = attempt;
    }

    pub(super) fn time_out(&self, detail: String) {
        self.state().timed_out = Some(detail);
    }

    pub(super) fn send(&self, output: ProviderOutput) {
        (self.state().sink)(output);
    }
}

/// How a turn ends whose process ended first: killed for not starting in time, or unable
/// to find the session it was to resume.
pub(super) fn ended_process(outcome: TurnOutcome, timed_out: Option<String>) -> TurnOutcome {
    match (outcome, timed_out) {
        (
            TurnOutcome::Interrupted
            | TurnOutcome::Failed {
                reason: TurnFailure::ProcessExited { .. },
            },
            Some(detail),
        ) => TurnOutcome::Failed {
            reason: TurnFailure::StartupTimedOut { detail },
        },
        (
            TurnOutcome::Failed {
                reason: TurnFailure::ProcessExited { detail },
            },
            None,
        ) if outcome::missing_session(&detail) => TurnOutcome::Failed {
            reason: TurnFailure::ResumeFailed { detail },
        },
        (outcome, _) => outcome,
    }
}

/// Asks the process to stop, and kills it after `grace` if it still runs then.
pub(super) fn ask_to_stop(process: &LiveProcess, grace: Duration) {
    let request = json!({
        "type": "control_request",
        "request_id": INTERRUPT_REQUEST_ID,
        "request": { "subtype": "interrupt" },
    });
    kill_after(process.clone(), grace);
    if !process.stdin().write_line(&request.to_string()) {
        process.interrupt();
    }
}

/// Kills `process` after `grace` if it still runs then, stdin closed or not.
pub(super) fn kill_after(process: LiveProcess, grace: Duration) {
    thread::spawn(move || {
        thread::sleep(grace);
        process.interrupt();
    });
}
