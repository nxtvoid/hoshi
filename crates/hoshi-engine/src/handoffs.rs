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
    /// it leaves the message too little room in the window of `model`, the one the turn runs on.
    pub(crate) fn prepare_handoff(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        prompt: &str,
        attachments: &[MessageAttachment],
        model: Option<&str>,
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
            model_window: adapter.context_window(model),
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
mod tests;
