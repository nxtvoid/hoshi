//! What a turn hands its agent before the message: the turns another provider ran, the
//! history of the thread a fork came from, or a fork merged back. A fork's first turn
//! resumes a copy of its source's session instead when its provider can. Also the fork and
//! merge-back commands, which read their turn from the transcript first.

use crate::{COMPACT_COMMAND, Engine, State, internal, lock, now};
use hoshi_core::{
    Command, CommandError, ContextHandoff, ForkOrigin, ForkPoint, HandoffBudget, HandoffDelivery,
    HandoffSection, HandoffSource, HandoffStrategy, ItemContent, MergeBackSummary,
    MessageAttachment, PendingHandoff, ThreadItem, TranscriptPage, TurnFailure, TurnOutcome,
    TurnPoint, TurnStatus, assemble_handoff, consume_handoffs, fork_without_native,
    handoff_token_cap, history_section, turn_point,
};
use hoshi_providers::ProviderAdapter;
use std::collections::HashSet;

/// Overrides how many bytes a handoff may carry, within bounds.
const TOKEN_CAP_VARIABLE: &str = "HOSHI_CONTEXT_HANDOFF_TOKEN_CAP";

/// What a starting turn carries besides its message, until it is delivered.
pub(crate) struct TurnHandoff {
    /// The transcript item that shows it.
    item: Option<ThreadItem>,
    /// What it takes off the thread once the provider took the context.
    delivers: Vec<PendingHandoff>,
    /// The fork handoff a native fork takes off once its session started.
    native_fork: Option<PendingHandoff>,
    delivered: bool,
}

/// How a starting turn goes on from what its agent did not see.
pub(crate) struct Prepared {
    pub handoff: TurnHandoff,
    pub context: Option<ContextHandoff>,
    /// The source's session and where in it a native fork resumes a copy.
    pub fork_from: Option<(String, String)>,
}

impl State {
    /// What the thread's turn `turn_id` hands its agent first, recorded as its transcript
    /// item. `None` when nothing waits, or a `/compact` turn could not carry it. Fails when
    /// it leaves the message too little room.
    pub(crate) fn prepare_handoff(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        prompt: &str,
        attachments: &[MessageAttachment],
        adapter: &dyn ProviderAdapter,
    ) -> Result<Option<Prepared>, TurnFailure> {
        let Some(thread) = self.model.thread(thread_id).cloned() else {
            return Ok(None);
        };
        if thread.pending_handoffs.is_empty() || prompt.trim() == COMPACT_COMMAND {
            return Ok(None);
        }
        let mut sections = Vec::new();
        let mut merge_backs = Vec::new();
        let mut sources = Vec::new();
        let mut delivers = Vec::new();
        let mut native_fork = None;
        let mut fork_from = None;
        let mut fresh = false;
        let mut own: Option<Vec<ThreadItem>> = None;
        for handoff in &thread.pending_handoffs {
            match handoff {
                PendingHandoff::Fork { native } => {
                    let Some(origin) = &thread.forked_from else {
                        delivers.push(handoff.clone());
                        continue;
                    };
                    if *native && let Some(resume) = self.native_fork(&thread, origin, adapter) {
                        fork_from = Some(resume);
                        native_fork = Some(handoff.clone());
                        continue;
                    }
                    fresh = true;
                    sections.extend(self.fork_sections(origin));
                    sources.push(HandoffSource::Fork {
                        thread_id: origin.thread_id.clone(),
                    });
                    delivers.push(handoff.clone());
                }
                PendingHandoff::ProviderSwitch {
                    from,
                    to,
                    strategy,
                    since,
                    before,
                } => {
                    fresh = true;
                    let items = own.get_or_insert_with(|| {
                        self.store.thread_items(thread_id).unwrap_or_default()
                    });
                    let position = |id: &str| items.iter().position(|item| item.id == id);
                    let start = since.as_deref().and_then(position).unwrap_or(0);
                    let end = position(before).unwrap_or(items.len());
                    let forking = thread
                        .pending_handoffs
                        .iter()
                        .any(|handoff| matches!(handoff, PendingHandoff::Fork { .. }));
                    if *strategy == HandoffStrategy::FullThreadSummary
                        && !forking
                        && let Some(origin) = &thread.forked_from
                    {
                        sections.extend(self.fork_sections(origin));
                    }
                    sections.extend(history_section(*strategy, thread_id, items, start..end));
                    sources.push(HandoffSource::ProviderSwitch {
                        from: from.clone(),
                        to: to.clone(),
                        strategy: *strategy,
                    });
                    delivers.push(handoff.clone());
                }
                PendingHandoff::MergeBack {
                    fork_thread_id,
                    turn,
                } => {
                    let items = self.store.thread_items(fork_thread_id).unwrap_or_default();
                    let end = items
                        .iter()
                        .position(|item| item.id == turn.through_item_id)
                        .map_or(items.len(), |position| position + 1);
                    merge_backs.push(MergeBackSummary::of(
                        fork_thread_id,
                        thread_id,
                        turn.ordinal,
                        &items[..end],
                    ));
                    sources.push(HandoffSource::MergeBack {
                        fork_thread_id: fork_thread_id.clone(),
                    });
                    delivers.push(handoff.clone());
                }
            }
        }
        let usage = if fresh {
            None
        } else {
            self.context_usages.get(thread_id).copied()
        };
        let configured = std::env::var(TOKEN_CAP_VARIABLE)
            .ok()
            .and_then(|cap| cap.trim().parse().ok());
        let budget = HandoffBudget {
            token_cap: handoff_token_cap(configured),
            model_window: adapter.context_window(thread.model.as_deref()),
            usage,
            user_text: prompt,
            attachments,
        }
        .bytes();
        let item = |context: Option<&ContextHandoff>, delivery| ThreadItem {
            id: format!("{turn_id}:handoff"),
            thread_id: thread_id.to_owned(),
            created_at: now(),
            content: ItemContent::ContextHandoff {
                sources: sources.clone(),
                delivery,
                included_items: context.map_or(0, |context| context.messages.len() as u32),
                omitted_items: context.map_or(0, |context| context.omitted_items),
            },
        };
        let context = match assemble_handoff(sections, merge_backs, budget) {
            Ok(context) => context,
            Err(failure) => {
                self.record_handoff_item(item(None, HandoffDelivery::Failed));
                return Err(failure);
            }
        };
        if context.is_none() && !delivers.is_empty() {
            let events = consume_handoffs(&self.model, thread_id, &delivers);
            if let Err(error) = self.commit(&events, &now()) {
                eprintln!("hoshi: failed to record an empty handoff: {error}");
            }
            delivers.clear();
        }
        if context.is_none() && native_fork.is_none() {
            return Ok(None);
        }
        let shown = context
            .as_ref()
            .map(|context| item(Some(context), HandoffDelivery::Pending));
        if let Some(shown) = &shown {
            self.record_handoff_item(shown.clone());
        }
        Ok(Some(Prepared {
            handoff: TurnHandoff {
                item: shown,
                delivers,
                native_fork,
                delivered: false,
            },
            context,
            fork_from,
        }))
    }

    /// The source's session and the point a fork's first turn resumes a copy of, when its
    /// provider can fork natively from that turn.
    fn native_fork(
        &self,
        thread: &hoshi_core::Thread,
        origin: &ForkOrigin,
        adapter: &dyn ProviderAdapter,
    ) -> Option<(String, String)> {
        let turn = &origin.turn;
        let busy = self
            .model
            .thread(&origin.thread_id)
            .is_some_and(|source| source.active_turn_id.is_some());
        let possible = turn.status == TurnStatus::Completed
            && turn.provider == thread.provider
            && adapter.forks_sessions()
            && !busy;
        possible.then(|| Some((turn.session_id.clone()?, turn.cursor.clone()?)))?
    }

    /// The history a fork inherits, as handoff sections: its source's items through the fork
    /// point, after those the source inherited in turn.
    fn fork_sections(&self, origin: &ForkOrigin) -> Vec<HandoffSection> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut next = Some(Box::new(origin.clone()));
        while let Some(origin) = next.take() {
            if !seen.insert(origin.thread_id.clone()) {
                break;
            }
            let items = self
                .store
                .thread_items(&origin.thread_id)
                .unwrap_or_default();
            if let Some(end) = items
                .iter()
                .position(|item| item.id == origin.turn.through_item_id)
            {
                chain.push((origin.thread_id.clone(), items, end + 1));
            }
            next = self
                .model
                .thread(&origin.thread_id)
                .and_then(|source| source.forked_from.clone());
        }
        chain
            .into_iter()
            .rev()
            .filter_map(|(thread_id, items, end)| {
                history_section(
                    HandoffStrategy::FullThreadSummary,
                    &thread_id,
                    &items,
                    0..end,
                )
            })
            .collect()
    }

    fn record_handoff_item(&mut self, item: ThreadItem) {
        if let Err(error) = self.commit(&[hoshi_core::Event::ItemAdded { item }], &now()) {
            eprintln!("hoshi: failed to record a context handoff: {error}");
        }
    }

    /// The running turn's provider took its context handoff.
    pub(crate) fn handoff_delivered(&mut self, thread_id: &str, turn_id: &str, injected: bool) {
        let Some(handoff) = self
            .turns
            .get_mut(thread_id)
            .filter(|turn| turn.id == turn_id)
            .and_then(|turn| turn.handoff.as_mut())
            .filter(|handoff| !handoff.delivered)
        else {
            return;
        };
        handoff.delivered = true;
        let delivery = if injected {
            HandoffDelivery::Injected
        } else {
            HandoffDelivery::Inline
        };
        let item = handoff
            .item
            .clone()
            .map(|item| with_delivery(item, delivery));
        let delivers = handoff.delivers.clone();
        if let Some(item) = item {
            self.record_handoff_item(item);
        }
        let events = consume_handoffs(&self.model, thread_id, &delivers);
        if let Err(error) = self.commit(&events, &now()) {
            eprintln!("hoshi: failed to record a delivered handoff: {error}");
        }
    }

    /// The running turn's native fork started its session: the fork has its source's
    /// conversation.
    pub(crate) fn native_fork_started(&mut self, thread_id: &str, turn_id: &str) {
        let Some(fork) = self
            .turns
            .get_mut(thread_id)
            .filter(|turn| turn.id == turn_id)
            .and_then(|turn| turn.handoff.as_mut())
            .and_then(|handoff| handoff.native_fork.take())
        else {
            return;
        };
        let events = consume_handoffs(&self.model, thread_id, &[fork]);
        if let Err(error) = self.commit(&events, &now()) {
            eprintln!("hoshi: failed to record a fork's session: {error}");
        }
    }

    /// A turn that could not start takes nothing off the thread; its handoff shows as failed.
    pub(crate) fn abandon_handoff(&mut self, handoff: Option<TurnHandoff>) {
        if let Some(item) = handoff.and_then(|handoff| handoff.item) {
            self.record_handoff_item(with_delivery(item, HandoffDelivery::Failed));
        }
    }

    /// The running turn ends: a handoff it never delivered shows as failed, and waits for the
    /// next turn; a native fork its provider could not make gives way to a transcript.
    pub(crate) fn handoff_turn_ended(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        outcome: &TurnOutcome,
    ) {
        let Some(handoff) = self
            .turns
            .get_mut(thread_id)
            .filter(|turn| turn.id == turn_id)
            .and_then(|turn| turn.handoff.take())
        else {
            return;
        };
        if !handoff.delivered
            && let Some(item) = handoff.item
        {
            self.record_handoff_item(with_delivery(item, HandoffDelivery::Failed));
        }
        let resume_failed = matches!(
            outcome,
            TurnOutcome::Failed {
                reason: TurnFailure::ResumeFailed { .. }
            }
        );
        if handoff.native_fork.is_some() && resume_failed {
            let events = fork_without_native(&self.model, thread_id);
            if let Err(error) = self.commit(&events, &now()) {
                eprintln!("hoshi: failed to record a fork's fallback: {error}");
            }
        }
    }
}

fn with_delivery(mut item: ThreadItem, wanted: HandoffDelivery) -> ThreadItem {
    if let ItemContent::ContextHandoff { delivery, .. } = &mut item.content {
        *delivery = wanted;
    }
    item.created_at = now();
    item
}

impl Engine {
    /// The turn of `thread_id` that `point` names, as a fork or a merge-back would start
    /// from it.
    pub fn turn_point(
        &self,
        thread_id: &str,
        point: &ForkPoint,
    ) -> Result<TurnPoint, CommandError> {
        let state = lock(&self.state);
        state.turn_point(thread_id, point)
    }

    /// Forks `thread_id` from the turn `point` names into a new thread `fork_thread_id`,
    /// which goes on from there with its first message.
    pub fn fork_thread(
        &self,
        thread_id: &str,
        fork_thread_id: &str,
        point: &ForkPoint,
    ) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        let turn = state.turn_point(thread_id, point)?;
        state.decide_and_commit(Command::ThreadFork {
            thread_id: thread_id.to_owned(),
            fork_thread_id: fork_thread_id.to_owned(),
            turn,
        })
    }

    /// Hands fork `thread_id`'s turns through the one `point` names to the thread it came
    /// from, on that thread's next turn. Refused while that thread has work running or
    /// waiting.
    pub fn merge_back(&self, thread_id: &str, point: &ForkPoint) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        let turn = state.turn_point(thread_id, point)?;
        let parent = state
            .model
            .thread(thread_id)
            .and_then(|thread| thread.forked_from.as_ref())
            .map(|origin| origin.thread_id.clone());
        if let Some(parent) = parent
            && state
                .queued
                .get(&parent)
                .is_some_and(|queued| !queued.is_empty())
        {
            return Err(CommandError::MergeBackTargetBusy { thread_id: parent });
        }
        state.decide_and_commit(Command::ThreadMergeBack {
            thread_id: thread_id.to_owned(),
            turn,
        })
    }

    /// What fork `thread_id` inherits from the thread it came from: that thread's latest turns
    /// before item `before` (the fork point when `None`), as [`Engine::transcript`] pages
    /// them. Empty when the thread is no fork or its source is gone; a fork of a fork
    /// inherits its source's inherited history too, which this call on the source returns.
    pub fn fork_history(
        &self,
        thread_id: &str,
        before: Option<&str>,
    ) -> Result<TranscriptPage, CommandError> {
        let state = lock(&self.state);
        let Some(origin) = state
            .model
            .thread(thread_id)
            .and_then(|thread| thread.forked_from.clone())
        else {
            return Ok(TranscriptPage {
                items: Vec::new(),
                has_older: false,
            });
        };
        let (items, has_older) = state
            .store
            .transcript_page_through(
                &origin.thread_id,
                Some(&origin.turn.through_item_id),
                before,
                crate::PAGE_TURNS,
                crate::PAGE_MAX_ITEMS,
            )
            .map_err(internal)?;
        let items = items
            .into_iter()
            .map(|item| ThreadItem {
                content: item.content.summary(),
                ..item
            })
            .collect();
        Ok(TranscriptPage { items, has_older })
    }
}

impl State {
    fn turn_point(&self, thread_id: &str, point: &ForkPoint) -> Result<TurnPoint, CommandError> {
        let thread = self
            .model
            .thread(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        let items = self.store.thread_items(thread_id).map_err(internal)?;
        turn_point(thread, &items, point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Notification, Notify};
    use hoshi_core::{
        AccessMode, BackgroundTask, Event, ProviderModels, ProviderOutput, RequestResponse,
        ThreadWorkspace,
    };
    use hoshi_providers::{OutputSink, RunningTurn, TurnRequest};
    use hoshi_store::Store;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    const CLAUDE: &str = hoshi_core::DEFAULT_PROVIDER;
    const CODEX: &str = "codex";

    /// Answers every turn at once: a session, `Done.` as `<provider>-<turn>:answer`, and a
    /// cursor after it.
    struct Scripted {
        name: &'static str,
        forks: bool,
        injects: bool,
        window: Option<u64>,
        /// Leaves a command running in the background, in a process kept after the turn.
        background: bool,
        requests: Mutex<Vec<TurnRequest>>,
    }

    impl Scripted {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                forks: true,
                injects: name == CODEX,
                window: None,
                background: false,
                requests: Mutex::default(),
            }
        }

        fn request(&self, turn: usize) -> TurnRequest {
            self.requests.lock().unwrap()[turn].clone()
        }
    }

    struct Done {
        lingers: bool,
    }

    impl RunningTurn for Done {
        fn interrupt(&self) {}

        fn respond(&self, _request_id: &str, _response: &RequestResponse) {}

        fn lingers(&self) -> bool {
            self.lingers
        }
    }

    impl ProviderAdapter for Scripted {
        fn provider(&self) -> &'static str {
            self.name
        }

        fn models(&self) -> ProviderModels {
            ProviderModels::default()
        }

        fn forks_sessions(&self) -> bool {
            self.forks
        }

        fn injects_context(&self) -> bool {
            self.injects
        }

        fn context_window(&self, _model: Option<&str>) -> Option<u64> {
            self.window
        }

        fn start_turn(
            &self,
            request: TurnRequest,
            mut sink: OutputSink,
        ) -> Result<Box<dyn RunningTurn>, TurnFailure> {
            let mut requests = self.requests.lock().unwrap();
            let turn = requests.len() + 1;
            let name = self.name;
            let session = match (&request.resume_session_id, request.fork_session) {
                (Some(session), false) => session.clone(),
                _ => format!("{name}-{turn}"),
            };
            let injects = self.injects && request.handoff.is_some();
            let background = self.background;
            requests.push(request);
            std::thread::spawn(move || {
                sink(ProviderOutput::SessionStarted {
                    session_id: session,
                });
                if injects {
                    sink(ProviderOutput::ContextHandoffDelivered { injected: true });
                }
                sink(ProviderOutput::AssistantMessage {
                    item_id: format!("{name}-{turn}:answer"),
                    text: "Done.".into(),
                });
                sink(ProviderOutput::ConversationCursor {
                    cursor: format!("{name}-{turn}:cursor"),
                });
                if background {
                    sink(ProviderOutput::BackgroundTasks {
                        tasks: vec![BackgroundTask {
                            task_id: "server".into(),
                            tool_call_id: None,
                            description: "dev server".into(),
                            started_at: 0,
                        }],
                        waiting: false,
                    });
                }
                sink(ProviderOutput::TurnCompleted {
                    outcome: TurnOutcome::Completed,
                });
            });
            Ok(Box::new(Done {
                lingers: background,
            }))
        }
    }

    struct Setup {
        engine: Engine,
        events: mpsc::Receiver<Event>,
        claude: Arc<Scripted>,
        codex: Arc<Scripted>,
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hoshi-handoffs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Project `p` on `root`, with thread `t` on Claude Code in `workspace`.
    fn setup(name: &str, root: &Path, workspace: ThreadWorkspace, claude: Scripted) -> Setup {
        let (sender, events) = mpsc::channel();
        let notify: Notify = Arc::new(move |notification| {
            if let Notification::Event(envelope) = notification {
                let _ = sender.send(envelope.event);
            }
        });
        let data = scratch(&format!("{name}-data"));
        let engine = Engine::from_store(Store::open_in_memory().unwrap(), &data, notify).unwrap();
        let (claude, codex) = (Arc::new(claude), Arc::new(Scripted::new(CODEX)));
        engine.set_adapters(vec![claude.clone(), codex.clone()]);
        let root = dunce::canonicalize(root).unwrap();
        engine
            .dispatch(Command::ProjectCreate {
                project_id: "p".into(),
                title: "Repo".into(),
                workspace_root: root.to_string_lossy().into_owned(),
            })
            .unwrap();
        engine
            .dispatch(Command::ThreadCreate {
                thread_id: "t".into(),
                project_id: Some("p".into()),
                title: "Login".into(),
                workspace,
                provider: CLAUDE.into(),
                model: None,
                effort: None,
                fast_mode: false,
                ultracode: false,
                access_mode: AccessMode::AcceptEdits,
                plan_mode: false,
            })
            .unwrap();
        Setup {
            engine,
            events,
            claude,
            codex,
        }
    }

    impl Setup {
        fn send(&self, thread_id: &str, message_id: &str, text: &str) {
            self.engine
                .dispatch(Command::MessageSend {
                    thread_id: thread_id.into(),
                    message_id: message_id.into(),
                    text: text.into(),
                    attachments: Vec::new(),
                })
                .unwrap();
            self.events
                .iter()
                .find(|event| {
                    matches!(event, Event::TurnCompleted { thread_id: done, .. } if done == thread_id)
                })
                .unwrap();
        }

        fn pick(&self, thread_id: &str, provider: &str) -> Result<(), CommandError> {
            self.engine.dispatch(Command::ThreadModelSet {
                thread_id: thread_id.into(),
                provider: provider.into(),
                model: None,
                effort: None,
                fast_mode: false,
                ultracode: false,
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

        fn handoff_item(&self, thread_id: &str, turn_id: &str) -> ItemContent {
            self.engine
                .thread_item(thread_id, &format!("{turn_id}:handoff"))
                .unwrap()
                .unwrap()
                .content
        }
    }

    fn local(name: &str, claude: Scripted) -> Setup {
        setup(name, &scratch(name), ThreadWorkspace::Local, claude)
    }

    fn delivery(content: &ItemContent) -> HandoffDelivery {
        match content {
            ItemContent::ContextHandoff { delivery, .. } => *delivery,
            other => panic!("not a handoff: {other:?}"),
        }
    }

    fn handoff_text(request: &TurnRequest) -> String {
        request.handoff.as_ref().unwrap().render()
    }

    #[test]
    fn another_provider_gets_the_conversation_and_a_switch_back_only_what_it_missed() {
        let setup = local("switch", Scripted::new(CLAUDE));
        setup.send("t", "m1", "Remember APPLE.");

        setup.pick("t", CODEX).unwrap();
        assert_eq!(setup.thread("t").provider, CLAUDE);
        setup.send("t", "m2", "Go on.");

        let to_codex = setup.codex.request(0);
        assert_eq!(to_codex.resume_session_id, None);
        assert_eq!(to_codex.prompt, "Go on.");
        let text = handoff_text(&to_codex);
        assert!(
            text.starts_with("Context handoff (full_thread_summary):"),
            "{text}"
        );
        assert!(text.contains("item=m1;") && text.contains("Remember APPLE."));
        assert!(text.contains("item=claudeCode-1:answer;"));
        assert_eq!(
            delivery(&setup.handoff_item("t", "m2")),
            HandoffDelivery::Injected
        );
        assert!(setup.thread("t").pending_handoffs.is_empty());

        setup.pick("t", CLAUDE).unwrap();
        setup.send("t", "m3", "And now?");

        let back = setup.claude.request(1);
        assert_eq!(back.resume_session_id.as_deref(), Some("claudeCode-1"));
        let text = handoff_text(&back);
        assert!(
            text.starts_with("Context handoff (delta_since_target_last_seen):"),
            "{text}"
        );
        assert!(text.contains("item=m2;") && text.contains("item=codex-1:answer;"));
        assert!(!text.contains("item=m1;"));
        assert_eq!(
            delivery(&setup.handoff_item("t", "m3")),
            HandoffDelivery::Inline
        );
        let thread = setup.thread("t");
        assert_eq!(thread.provider, CLAUDE);
        assert_eq!(thread.provider_sessions[0].session_id, "codex-1");
    }

    #[test]
    fn a_handoff_that_leaves_no_room_fails_the_turn_and_waits_for_the_next() {
        let setup = local("too-large", Scripted::new(CLAUDE));
        setup.send("t", "m1", "Remember APPLE.");
        setup.pick("t", CODEX).unwrap();
        setup.engine.set_adapters(vec![
            setup.claude.clone(),
            Arc::new(Scripted {
                window: Some(16_100),
                ..Scripted::new(CODEX)
            }),
        ]);

        setup.send("t", "m2", "Go on.");

        assert_eq!(
            delivery(&setup.handoff_item("t", "m2")),
            HandoffDelivery::Failed
        );
        let failed = setup.engine.thread_item("t", "m2:failed").unwrap().unwrap();
        assert_eq!(
            failed.content,
            ItemContent::TurnFailed {
                reason: TurnFailure::ContextHandoffTooLarge
            }
        );
        assert_eq!(setup.thread("t").pending_handoffs.len(), 1);
    }

    #[test]
    fn a_kept_process_with_background_work_holds_off_a_switch() {
        let setup = local(
            "background",
            Scripted {
                background: true,
                ..Scripted::new(CLAUDE)
            },
        );
        setup.send("t", "m1", "Start the server.");

        assert_eq!(
            setup.pick("t", CODEX),
            Err(CommandError::BackgroundWorkRunning {
                thread_id: "t".into()
            })
        );
        assert_eq!(setup.thread("t").pending_model, None);
    }

    #[test]
    fn a_fork_resumes_a_copy_of_its_source_session_from_its_turn() {
        let setup = local("native-fork", Scripted::new(CLAUDE));
        setup.send("t", "m1", "Remember APPLE.");
        setup.send("t", "m2", "Remember BANANA.");
        let point = ForkPoint::Turn {
            item_id: "claudeCode-1:answer".into(),
        };

        setup.engine.fork_thread("t", "f", &point).unwrap();
        let history = setup.engine.fork_history("f", None).unwrap();
        setup.send("f", "f1", "Which words?");

        let ids: Vec<&str> = history.items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["m1", "claudeCode-1:answer"]);
        let forked = setup.claude.request(2);
        assert_eq!(forked.resume_session_id.as_deref(), Some("claudeCode-1"));
        assert_eq!(
            forked.resume_session_at.as_deref(),
            Some("claudeCode-1:cursor")
        );
        assert!(forked.fork_session);
        assert!(forked.handoff.is_none());
        let fork = setup.thread("f");
        assert_eq!(fork.title, "Login fork");
        assert_eq!(fork.session_id.as_deref(), Some("claudeCode-3"));
        assert!(fork.pending_handoffs.is_empty());
        assert_eq!(
            setup.thread("t").session_id.as_deref(),
            Some("claudeCode-1")
        );
    }

    #[test]
    fn a_fork_knows_what_its_turn_ran_on() {
        let setup = local("fork-model", Scripted::new(CLAUDE));
        let effort_of = |thread_id: &str, effort| hoshi_core::Command::ThreadModelSet {
            thread_id: thread_id.into(),
            provider: CLAUDE.into(),
            model: Some("opus".into()),
            effort: Some(effort),
            fast_mode: false,
            ultracode: false,
        };
        let effort = |effort| effort_of("t", effort);
        setup
            .engine
            .dispatch(effort(hoshi_core::Effort::Medium))
            .unwrap();
        setup.send("t", "m1", "Remember APPLE.");
        setup
            .engine
            .dispatch(effort(hoshi_core::Effort::High))
            .unwrap();
        setup.send("t", "m2", "Remember BANANA.");

        setup
            .engine
            .fork_thread(
                "t",
                "f",
                &ForkPoint::Turn {
                    item_id: "m1".into(),
                },
            )
            .unwrap();

        let last = |setup: &Setup| setup.thread("f").last_turn_model.map(|model| model.effort);
        assert_eq!(last(&setup), Some(Some(hoshi_core::Effort::Medium)));

        let fork_effort = effort_of("f", hoshi_core::Effort::High);
        setup.engine.dispatch(fork_effort).unwrap();
        setup.send("f", "f1", "Which words?");
        assert_eq!(last(&setup), Some(Some(hoshi_core::Effort::High)));
        setup.engine.rewind_thread("f", "f1", false).unwrap();
        assert_eq!(
            last(&setup),
            Some(Some(hoshi_core::Effort::Medium)),
            "before its first own turn, a fork ran what its source did"
        );
    }

    #[test]
    fn a_fork_on_another_provider_gets_its_source_history_as_a_transcript() {
        let setup = local("portable-fork", Scripted::new(CLAUDE));
        setup.send("t", "m1", "Remember APPLE.");
        setup.send("t", "m2", "Remember BANANA.");
        setup
            .engine
            .fork_thread(
                "t",
                "f",
                &ForkPoint::Turn {
                    item_id: "m1".into(),
                },
            )
            .unwrap();

        setup.pick("f", CODEX).unwrap();
        setup.send("f", "f1", "Which words?");

        let request = setup.codex.request(0);
        assert_eq!(request.resume_session_id, None);
        assert!(!request.fork_session);
        let text = handoff_text(&request);
        assert!(text.contains("Thread: t. Covered turns: 1-1."), "{text}");
        assert!(text.contains("Remember APPLE.") && !text.contains("BANANA"));
        assert!(setup.thread("f").pending_handoffs.is_empty());
    }

    #[test]
    fn a_merge_back_reaches_the_parent_s_next_turn_as_the_fork_s_delta() {
        let setup = local("merge-back", Scripted::new(CLAUDE));
        setup.send("t", "m1", "Plan the fix.");
        setup
            .engine
            .fork_thread("t", "f", &ForkPoint::LatestStable)
            .unwrap();
        setup.send("f", "f1", "Try the other approach.");

        setup
            .engine
            .merge_back("f", &ForkPoint::LatestStable)
            .unwrap();
        setup.send("t", "m2", "What did the fork find?");

        let request = setup.claude.request(2);
        assert_eq!(request.resume_session_id.as_deref(), Some("claudeCode-1"));
        let text = handoff_text(&request);
        let expected = [
            "Context handoff (merge_back / fork_delta_summary):",
            "Merge-back context from forked conversation.",
            "Source thread: f",
            "Target thread: t",
            "Covered fork turns: 1-1",
            "",
            "Fork delta:",
            "- User: Try the other approach.",
            "- Assistant: Done.",
        ]
        .join("\n");
        assert!(text.starts_with(&expected), "{text}");
        assert!(setup.thread("t").pending_handoffs.is_empty());
    }

    fn wait_until(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_worktree_a_fork_shares_stays_until_neither_thread_uses_it() {
        let repo = crate::git::tests::temp_repository("handoffs-shared-worktree");
        let setup = setup(
            "shared-worktree",
            &repo,
            ThreadWorkspace::NewWorktree,
            Scripted::new(CLAUDE),
        );
        setup.send("t", "m1", "Work.");
        let worktree = setup.thread("t").worktree_path.unwrap();
        setup
            .engine
            .fork_thread("t", "f", &ForkPoint::LatestStable)
            .unwrap();
        assert_eq!(
            setup.thread("f").worktree_path.as_deref(),
            Some(worktree.as_str())
        );

        assert_eq!(
            setup.engine.release_worktree("t"),
            Err(CommandError::WorktreeShared {
                thread_id: "t".into(),
                other_thread_id: "f".into()
            })
        );
        let delete = |thread_id: &str| Command::ThreadDelete {
            thread_id: thread_id.into(),
        };
        assert!(
            lock(&setup.engine.state)
                .worktrees_freed_by(&delete("t"))
                .is_empty()
        );
        setup.engine.dispatch(delete("t")).unwrap();
        let freed = lock(&setup.engine.state).worktrees_freed_by(&delete("f"));
        assert_eq!(freed.len(), 1);
        assert_eq!(freed[0].1, worktree);

        setup
            .engine
            .dispatch(Command::ThreadDelete {
                thread_id: "f".into(),
            })
            .unwrap();
        wait_until(|| !Path::new(&worktree).exists());
    }
}
