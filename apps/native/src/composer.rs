mod attachments;
pub(crate) use attachments::format_size;
pub(crate) use footer::{
    ACCESS_MODES, access_description, access_label, effective_effort, effort_label, fit_choice,
    model_options, workspace_option,
};
pub(crate) use model_picker::{
    ModelTab, PickerHost, PickerList, default_tab, provider_mark, provider_tint,
};
mod chips;
mod context;
mod delegated_request;
mod footer;
mod handoff;
mod held;
mod history;
mod model_picker;
mod notices;
mod planning_note;
mod queue;
mod request_panel;
mod suggestions;
mod trigger;

use crate::m;
use crate::requests::QuestionProgress;
use crate::state::Workspace;
use attachments::DraftAttachment;
use footer::FooterMenu;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::*;
use hoshi_core::{
    ApprovalDecision, Command, CommandError, ItemContent, MessageAttachment, SlashCommand,
    ThreadItem, ThreadStatus,
};
use hoshi_ui::prelude::*;
use hoshi_ui::prelude::{LanguageLabels, RichEditor, RichEditorEvent};
use request_panel::SecretField;
use std::collections::{HashMap, HashSet};
use suggestions::{Scope, Suggestion};
use trigger::Trigger;

/// One line of the docked editor, so the buttons beside it center on it.
const DOCKED_LINE: Pixels = px(32.);

const DROP_GROUP: &str = "composer-drop";

/// A command's failure as the composer says it.
pub(crate) fn command_error_message(error: &CommandError) -> String {
    match error {
        CommandError::ThreadBusy { .. } => m::error_turn_running(),
        CommandError::BackgroundWorkRunning { .. } => m::error_background_work_running(),
        CommandError::QueuedMessageNotFound { .. } => m::queue_message_gone(),
        CommandError::ThreadNotFound { .. } => m::error_thread_not_found(),
        CommandError::ProjectNotFound { .. } => m::error_project_not_found(),
        CommandError::ThreadNotWorking { .. } => m::error_thread_not_working(),
        CommandError::EmptyMessage => m::error_empty_message(),
        CommandError::RequestNotPending { .. } => m::error_request_not_pending(),
        CommandError::RequestResponseInvalid { .. } => m::error_request_response_invalid(),
        CommandError::ProviderLocked { .. } => m::error_provider_locked(),
        CommandError::ThreadArchived { .. } => m::error_thread_archived(),
        CommandError::ThreadWaitingForUser { .. } => m::error_thread_waiting_for_user(),
        CommandError::MessageQueued { .. } => m::error_message_queued(),
        CommandError::NotAGitRepository { .. } => m::error_not_a_git_repository(),
        CommandError::WorktreeFailed { detail } => m::error_worktree_failed(detail),
        CommandError::WorkspaceRootNotFound { path } => m::error_workspace_root_not_found(path),
        CommandError::ChatNotLocal { .. } => m::error_chat_not_local(),
        CommandError::CannotRewind { .. } => m::error_cannot_rewind(),
        CommandError::NoCheckpoint { .. } => m::error_no_checkpoint(),
        CommandError::FolderUnavailable { path } => m::error_folder_unavailable(path),
        CommandError::GitFailed { detail } => m::error_git_failed(detail),
        _ => m::error_generic(),
    }
}

pub struct Composer {
    workspace: Entity<Workspace>,
    editor: Entity<RichEditor>,
    trigger: Option<Trigger>,
    /// The trigger Escape closed, kept closed until the token changes.
    dismissed: Option<Trigger>,
    suggestions: Vec<Suggestion>,
    /// Shown under the suggestions, or alone while there are none.
    status: Option<SharedString>,
    suggestion_scroll: ScrollHandle,
    editor_scroll: ScrollHandle,
    /// Where the caret was within the scrolled text at the last frame.
    caret_at: Option<Point<Pixels>>,
    reveal_caret: bool,
    active: usize,
    search: u64,
    commands: HashMap<Scope, Vec<SlashCommand>>,
    commands_loading: HashSet<Scope>,
    /// What the pull request and skill chips stand for, by chip id.
    mentions: HashMap<String, MessageAttachment>,
    attachments: Vec<DraftAttachment>,
    next_attachment: u64,
    error: Option<SharedString>,
    /// A passing note, such as where a large paste went.
    notice: Option<SharedString>,
    _notice_timer: Option<Task<()>>,
    placeholder: SharedString,
    /// How far the user got through each request's questions, by request id.
    progress: HashMap<String, QuestionProgress>,
    secret: Option<SecretField>,
    sending: bool,
    menu: Option<FooterMenu>,
    show_legacy: bool,
    model_search: Entity<InputState>,
    /// `None` until a rail button is pressed: the picker's default tab.
    model_tab: Option<ModelTab>,
    /// The docked editor outgrew one line; it stays wide until it is emptied.
    tall: bool,
    /// Past prompts, oldest first, for ↑.
    history: Vec<String>,
    recalled: Option<history::Recalled>,
    /// Where the text on screen is kept as a draft.
    draft_key: Option<String>,
    /// Images and files still being saved when the composer left their draft, by its key.
    parked: HashMap<String, Vec<DraftAttachment>>,
    /// Threads whose user chose to keep their full history on the next send.
    kept_history: HashSet<String>,
    notices: notices::Notices,
    _ticker: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Composer {
    pub fn new(workspace: Entity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder: SharedString = m::composer_placeholder().into();
        let pasting = cx.weak_entity();
        let editor = cx.new(|cx| {
            let mut editor = RichEditor::new(window, cx);
            editor.set_placeholder(placeholder.clone(), window, cx);
            editor.set_language_labels(
                LanguageLabels {
                    plain: m::code_language_plain().into(),
                    none: m::code_language_none().into(),
                },
                m::code_language_search(),
                window,
                cx,
            );
            let opening = pasting.clone();
            editor.set_paste_handler(move |item, window, cx| {
                pasting
                    .update(cx, |this, cx| this.paste_attachments(item, window, cx))
                    .unwrap_or(false)
            });
            editor.set_token_click_handler(move |token, window, cx| {
                if let Some(key) = chips::attachment_key(token.id()) {
                    opening
                        .update(cx, |this, cx| this.open_attachment(key, window, cx))
                        .ok();
                }
            });
            editor
        });
        let this = cx.weak_entity();
        let menu_keys = cx.intercept_keystrokes(move |event, window, cx| {
            let handled = this
                .update(cx, |composer, cx| {
                    composer.menu_key(&event.keystroke, window, cx)
                })
                .unwrap_or(false);
            if handled {
                cx.stop_propagation();
            }
        });
        let model_search = cx.new(|cx| InputState::new(window, cx).placeholder(m::search_models()));
        let subscriptions = vec![
            cx.subscribe_in(&editor, window, Self::on_editor),
            cx.subscribe_in(
                &model_search,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => {
                        this.pick_shortcut(0, window, cx);
                    }
                    _ => {}
                },
            ),
            menu_keys,
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.follow_draft(window, cx);
                cx.notify();
            }),
        ];
        cx.defer_in(window, |this: &mut Self, window, cx| {
            this.follow_draft(window, cx)
        });
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(context::CHECK_EVERY).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let mut composer = Self {
            workspace,
            editor,
            trigger: None,
            dismissed: None,
            suggestions: Vec::new(),
            status: None,
            suggestion_scroll: ScrollHandle::new(),
            editor_scroll: ScrollHandle::new(),
            caret_at: None,
            reveal_caret: false,
            active: 0,
            search: 0,
            commands: HashMap::new(),
            commands_loading: HashSet::new(),
            mentions: HashMap::new(),
            attachments: Vec::new(),
            next_attachment: 0,
            error: None,
            notice: None,
            _notice_timer: None,
            placeholder,
            progress: HashMap::new(),
            secret: None,
            sending: false,
            menu: FooterMenu::from_env(),
            show_legacy: false,
            model_search,
            model_tab: None,
            tall: false,
            history: Vec::new(),
            recalled: None,
            draft_key: None,
            parked: HashMap::new(),
            kept_history: HashSet::new(),
            notices: Default::default(),
            _ticker: ticker,
            _subscriptions: subscriptions,
        };
        composer.load_history(cx);
        composer
    }

    /// Keeps what is written as the draft of where it goes, then shows the draft of where the
    /// composer writes now: its text with its chips, and its saved images and files.
    fn follow_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.workspace.read(cx).draft_key(cx);
        if key == self.draft_key {
            return;
        }
        self.keep_draft(cx);
        let saving: Vec<DraftAttachment> = self
            .attachments
            .iter()
            .filter(|kept| kept.saved.is_none() && !kept.failed)
            .cloned()
            .collect();
        if let Some(left) = self.draft_key.clone()
            && !saving.is_empty()
        {
            self.parked.entry(left).or_default().extend(saving);
        }
        self.draft_key = key;
        let workspace = self.workspace.read(cx);
        let draft = self
            .draft_key
            .as_ref()
            .and_then(|key| workspace.drafts.get(key).cloned())
            .unwrap_or_default();
        let engine = workspace.engine.clone();
        self.attachments = draft
            .attachments
            .iter()
            .filter_map(|kept| {
                let path = engine.attachment_path(&kept.attachment)?;
                attachments::restored(kept, path.into())
            })
            .collect();
        if let Some(parked) = self
            .draft_key
            .as_ref()
            .and_then(|key| self.parked.remove(key))
        {
            self.attachments.extend(parked);
        }
        let highest = self.attachments.iter().map(|kept| kept.key).max();
        self.next_attachment = self.next_attachment.max(highest.unwrap_or(0));
        self.mentions = draft.mentions.iter().cloned().collect();
        self.error = None;
        let blocks = draft.snapshot();
        self.editor
            .update(cx, |editor, cx| editor.restore(blocks, window, cx));
    }

    /// Keeps what is written, as it is now, as the draft of where it goes.
    fn keep_draft(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.draft_key.clone() else {
            return;
        };
        let editor = self.editor.read(cx);
        let chips: HashSet<String> = editor
            .tokens(cx)
            .iter()
            .map(|span| span.token().id().to_string())
            .collect();
        let mut draft = crate::drafts::Draft::from_snapshot(editor.snapshot(cx));
        draft.attachments = self
            .attachments
            .iter()
            .filter_map(|kept| {
                Some(crate::drafts::Attachment {
                    key: kept.key,
                    chipped: kept.chipped,
                    attachment: kept.saved.clone()?,
                })
            })
            .collect();
        draft.mentions = self
            .mentions
            .iter()
            .filter(|(id, _)| chips.contains(*id))
            .map(|(id, mention)| (id.clone(), mention.clone()))
            .collect();
        draft.mentions.sort_by(|a, b| a.0.cmp(&b.0));
        self.workspace
            .update(cx, |workspace, cx| workspace.set_draft(&key, draft, cx));
    }

    /// Writes `draft` in place of what is written, as a message the thread went back before
    /// returns to be edited.
    pub fn restore_message(
        &mut self,
        draft: crate::drafts::Draft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = self.draft_key.take() {
            let draft = chips::chip_attachment_labels(draft);
            self.workspace
                .update(cx, |workspace, cx| workspace.set_draft(&key, draft, cx));
        }
        self.follow_draft(window, cx);
        self.focus(window, cx);
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.focus(window, cx));
    }

    /// Scrolls the text to the caret after it moves or the text changes, once laid out.
    fn caret_follower(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let editor = self.editor.clone();
        let scroll = self.editor_scroll.clone();
        canvas(
            move |_, _, cx| {
                let Some(caret) = editor.read(cx).caret_bounds(cx) else {
                    return;
                };
                let offset = scroll.offset();
                let at = point(caret.left(), caret.top() - offset.y);
                this.update(cx, |this, cx| {
                    let moved = this.caret_at != Some(at);
                    this.caret_at = Some(at);
                    let requested = std::mem::take(&mut this.reveal_caret);
                    if !(moved || requested) {
                        return;
                    }
                    let view = scroll.bounds();
                    let margin = px(4.);
                    let mut y = offset.y;
                    if caret.bottom() + margin > view.bottom() {
                        y -= caret.bottom() + margin - view.bottom();
                    } else if caret.top() - margin < view.top() {
                        y += view.top() - (caret.top() - margin);
                    }
                    let y = y.clamp(-scroll.max_offset().y, px(0.));
                    if y != offset.y {
                        scroll.set_offset(point(offset.x, y));
                        cx.notify();
                    }
                })
                .ok();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }

    /// Shows a note for a few seconds where errors show.
    pub(crate) fn show_notice(&mut self, notice: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.notice = Some(notice.into());
        self._notice_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(6))
                .await;
            this.update(cx, |this, cx| {
                this.notice = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The block the caret is in: where triggers are typed and chips go.
    fn input(&self, cx: &App) -> Option<Entity<TextareaState>> {
        self.editor.read(cx).active_input().cloned()
    }

    fn text(&self, cx: &App) -> String {
        self.editor.read(cx).markdown(cx)
    }

    fn set_text(&self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.set_markdown(text, window, cx));
    }

    #[cfg(test)]
    pub fn editor(&self) -> &Entity<RichEditor> {
        &self.editor
    }

    #[cfg(test)]
    pub fn suggestions(&self) -> &[Suggestion] {
        &self.suggestions
    }

    fn on_editor(
        &mut self,
        _: &Entity<RichEditor>,
        event: &RichEditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RichEditorEvent::Submit => self.send(window, cx),
            RichEditorEvent::Change => {
                if self.editor.read(cx).is_empty(cx) {
                    self.tall = false;
                }
                self.reveal_caret = true;
                self.keep_draft(cx);
                self.update_trigger(cx);
                cx.notify();
            }
        }
    }

    /// Keys that steer the open menus, read before the editor sees them.
    fn menu_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.menu == Some(FooterMenu::Model) {
            return self.model_key(keystroke, window, cx);
        }
        if !self.editor.read(cx).is_focused(window, cx) {
            return false;
        }
        let modifiers = &keystroke.modifiers;
        if keystroke.key == "v"
            && (modifiers.control || modifiers.platform)
            && modifiers.shift
            && !modifiers.alt
        {
            return self.paste_as_text(window, cx);
        }
        let plain = !(modifiers.control || modifiers.alt || modifiers.platform);
        if plain && (self.suggestion_key(keystroke, window, cx) || self.request_key(keystroke, cx))
        {
            return true;
        }
        if self.history_key(keystroke, window, cx) {
            return true;
        }
        self.editor
            .update(cx, |editor, cx| editor.handle_key(keystroke, window, cx))
    }

    /// A digit picks an option of the question on screen while nothing is typed.
    fn request_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        if keystroke.modifiers.shift || !self.editor.read(cx).is_empty(cx) {
            return false;
        }
        let Some(request) = self.pending_request(cx) else {
            return false;
        };
        let Some((label, multi_select)) = self.option_for_digit(&request, &keystroke.key) else {
            return false;
        };
        self.pick_option(&request, &label, multi_select, cx);
        true
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx).trim().to_owned();
        if let Some(request) = self.pending_request(cx) {
            self.answer_typed(&request, text, window, cx);
            return;
        }
        if self.attachments.is_empty()
            && let Some(action) = suggestions::standalone_action(&text, &self.actions(cx))
        {
            self.set_text("", window, cx);
            self.run_action(action, window, cx);
            return;
        }
        let ordered = self.ordered_attachments(cx);
        let failed = ordered.iter().any(|attachment| attachment.failed);
        if (text.is_empty() && ordered.is_empty()) || self.attachments_saving(cx) || failed {
            return;
        }
        let labels = chips::attachment_labels(
            &ordered
                .iter()
                .map(|attachment| (attachment.key, attachment.kind))
                .collect::<Vec<_>>(),
        );
        let raw = text;
        let text = chips::number_attachment_chips(&raw, &labels);
        let taken = std::mem::take(&mut self.attachments);
        let left_out: Vec<_> = taken
            .iter()
            .filter(|attachment| !ordered.iter().any(|sent| sent.key == attachment.key))
            .cloned()
            .collect();
        let attachments: Vec<MessageAttachment> = ordered
            .iter()
            .filter_map(|attachment| attachment.saved.clone())
            .chain(self.mentioned(cx))
            .collect();
        let compact_first = self.compacts_first(&raw, cx);
        let workspace = self.workspace.read(cx);
        let sending = match workspace.open_thread_id(cx) {
            Some(thread_id) => {
                let engine = workspace.engine.clone();
                let message_id = uuid::Uuid::new_v4().to_string();
                let text = text.clone();
                cx.background_spawn(async move {
                    if compact_first {
                        engine.compact_then_send(&thread_id, &message_id, text, attachments)
                    } else {
                        engine.dispatch(Command::MessageSend {
                            thread_id,
                            message_id,
                            text,
                            attachments,
                        })
                    }
                })
            }
            None if workspace.draft.is_some() => {
                let draft = text.clone();
                self.workspace.update(cx, |workspace, cx| {
                    workspace.send_draft(draft, attachments, cx)
                })
            }
            None => {
                self.attachments = taken;
                return;
            }
        };
        self.discard_all(&left_out, cx);
        self.set_text("", window, cx);
        self.recalled = None;
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = sending.await;
            if result.is_ok() {
                this.update(cx, |this, cx| this.remember_prompt(&text, cx))
                    .ok();
            }
            if let Err(error) = result {
                this.update_in(cx, |this, window, cx| {
                    this.error = Some(command_error_message(&error).into());
                    this.set_text(&raw, window, cx);
                    let added = std::mem::take(&mut this.attachments);
                    this.attachments = ordered
                        .into_iter()
                        .map(chips::unchipped)
                        .chain(added)
                        .collect();
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }
}

impl Composer {
    /// Typed text answers the pending request: feedback that declines, or an own answer. A
    /// secret is typed in its own card, so it never reaches the chat.
    fn answer_typed(
        &mut self,
        request: &ThreadItem,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if text.is_empty() {
            return;
        }
        let answered = match &request.content {
            ItemContent::Questions { .. } => {
                let from = self.progress.get(&request.id).cloned().unwrap_or_default();
                self.advance_from(request, &from, &text, cx)
            }
            ItemContent::Approval { .. } | ItemContent::ProposedPlan { .. } => {
                self.decide(request, ApprovalDecision::Decline, Some(text), cx);
                true
            }
            _ => false,
        };
        if answered {
            self.set_text("", window, cx);
        }
    }

    fn interrupt(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace.read(cx);
        let Some(thread_id) = workspace.open_thread_id(cx) else {
            return;
        };
        if workspace.stopping_since(&thread_id).is_some() {
            return;
        }
        let engine = workspace.engine.clone();
        self.workspace
            .update(cx, |workspace, cx| workspace.mark_stopping(&thread_id, cx));
        cx.spawn(async move |this, cx| {
            let stopped = thread_id.clone();
            let result = cx
                .background_spawn(async move {
                    engine.dispatch(Command::TurnInterrupt { thread_id: stopped })
                })
                .await;
            if let Err(error) = result {
                this.update(cx, |this, cx| {
                    this.workspace.update(cx, |workspace, cx| {
                        workspace.unmark_stopping(&thread_id, cx)
                    });
                    this.error = Some(command_error_message(&error).into());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn stopping(&self, cx: &App) -> bool {
        let workspace = self.workspace.read(cx);
        workspace
            .open_thread_id(cx)
            .is_some_and(|id| workspace.stopping_since(&id).is_some())
    }

    fn working(&self, cx: &App) -> bool {
        let workspace = self.workspace.read(cx);
        workspace
            .open_thread_id(cx)
            .and_then(|id| workspace.model.thread(&id))
            .is_some_and(|thread| {
                matches!(
                    thread.status,
                    ThreadStatus::Working | ThreadStatus::WaitingForInput
                )
            })
    }

    fn sync_placeholder(
        &mut self,
        request: Option<&ThreadItem>,
        working: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder: SharedString = match request.map(|request| &request.content) {
            Some(ItemContent::SecretRequest { .. }) => m::secret_request_composer_placeholder(),
            Some(ItemContent::Approval { .. }) => m::request_approval_placeholder(),
            Some(ItemContent::Questions { .. }) => m::request_question_placeholder(),
            Some(ItemContent::ProposedPlan { .. }) => m::request_plan_placeholder(),
            _ if working => m::composer_steer_placeholder(),
            _ => m::composer_placeholder(),
        }
        .into();
        if placeholder != self.placeholder {
            self.placeholder = placeholder.clone();
            self.editor.update(cx, |editor, cx| {
                editor.set_placeholder(placeholder, window, cx)
            });
        }
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let request = self.pending_request(cx);
        let delegated = request
            .is_none()
            .then(|| self.delegated_request(cx))
            .flatten();
        let working = self.working(cx);
        self.sync_placeholder(request.as_ref(), working, window, cx);
        let shown = request
            .as_ref()
            .or(delegated.as_ref().map(|(request, _)| request));
        if self
            .secret
            .as_ref()
            .is_some_and(|field| shown.is_none_or(|request| request.id != field.request_id()))
        {
            self.secret = None;
        }
        let panel = match (&request, &delegated) {
            (Some(request), _) => Some(self.render_request(request, None, window, cx)),
            (None, delegated) => {
                let delegated = delegated
                    .as_ref()
                    .map(|(request, task)| self.render_delegated(request, task, window, cx));
                let note = planning_note::planning_note(&self.workspace, cx);
                match (delegated, note) {
                    (Some(delegated), Some(note)) => Some(
                        div()
                            .flex()
                            .flex_col()
                            .child(delegated)
                            .child(note)
                            .into_any_element(),
                    ),
                    (delegated, note) => delegated.or(note),
                }
            }
        };
        let written = !self.text(cx).trim().is_empty();
        let secret_pending = matches!(
            request.as_ref().map(|request| &request.content),
            Some(ItemContent::SecretRequest { .. })
        );
        let attached = !self.attachments.is_empty();
        let blocked = self.attachments_saving(cx)
            || self.attached(cx).iter().any(|attachment| attachment.failed);
        let can_send = (written || (attached && request.is_none())) && !secret_pending && !blocked;
        let resumable =
            !working && request.is_none() && self.queue(cx).is_some_and(|queue| queue.can_resume);
        let (kind, action) = if working && !written && !attached {
            let stopping = self.stopping(cx);
            let stop = Button::new("stop")
                .icon(IconName::Square)
                .size(ButtonSize::IconSm)
                .variant(ButtonVariant::Secondary)
                .rounded_full()
                .disabled(stopping)
                .tooltip(
                    Tooltip::new(if stopping {
                        m::stopping()
                    } else {
                        m::composer_stop()
                    })
                    .side(Side::Top),
                )
                .on_click(cx.listener(|this, _, _, cx| this.interrupt(cx)));
            ("stop", stop)
        } else if resumable && !written && !attached {
            let resume = Button::new("resume")
                .icon(IconName::Play)
                .label(m::queue_resume())
                .size(ButtonSize::Sm)
                .rounded_full()
                .tooltip(Tooltip::new(m::queue_resume_hint()).side(Side::Top))
                .on_click(cx.listener(|this, _, _, cx| this.resume(cx)));
            ("resume", resume)
        } else {
            let send = Button::new("send")
                .icon(IconName::ArrowUp)
                .size(ButtonSize::IconSm)
                .rounded_full()
                .disabled(!can_send)
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)));
            ("send", send)
        };
        let action = div()
            .debug_selector(move || format!("composer-{kind}"))
            .child(action);
        let footer = self.render_footer(cx);
        let notices = self.render_notices(cx);
        let held = self.render_held(cx);
        let queue = self.render_queue(cx);
        let handoff = self.render_handoff(cx);
        let suggestions = self.render_suggestions(cx);
        let tiles = self.render_attachments(cx);
        let attach = self.attach_button(cx);

        let hero = {
            let workspace = self.workspace.read(cx);
            workspace.open.is_none() && workspace.draft.is_some()
        };
        let infos = self.chip_infos();
        let colors = cx.hoshi().clone();
        self.editor.update(cx, |editor, _| {
            editor.set_token_renderer(move |context, _, _| {
                chips::render_chip(context, &infos, &colors)
            });
        });
        let stacked = hero || self.tall;
        let (inset_y, min_height) = if hero {
            (px(8.), px(64.))
        } else {
            (px(6.), DOCKED_LINE)
        };
        self.editor
            .update(cx, |editor, _| editor.set_inset(px(10.), inset_y));
        let measured = cx.weak_entity();
        let editor = div()
            .id("composer-editor")
            .relative()
            .flex_1()
            .min_w_0()
            .cursor_text()
            .on_click(cx.listener(|this, _, window, cx| this.focus(window, cx)))
            .child(
                div()
                    .id("composer-scroll")
                    .min_h(min_height)
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .track_scroll(&self.editor_scroll)
                    .child(self.editor.clone()),
            )
            .child(self.caret_follower(cx))
            .when(!stacked, |this| {
                this.child(
                    canvas(
                        move |bounds, _, cx| {
                            if bounds.size.height > DOCKED_LINE + px(8.) {
                                measured
                                    .update(cx, |this, cx| {
                                        this.tall = true;
                                        cx.notify();
                                    })
                                    .ok();
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
            });
        let body = if stacked {
            div().flex().flex_col().gap_1().child(editor).child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(attach)
                    .child(action),
            )
        } else {
            div()
                .flex()
                .items_end()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .h(DOCKED_LINE)
                        .child(attach),
                )
                .child(editor)
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .h(DOCKED_LINE)
                        .child(action),
                )
        };

        div()
            .id("composer")
            .group(DROP_GROUP)
            .pb_4()
            .pt_1()
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.attach_paths(paths.paths().to_vec(), window, cx)
            }))
            .child(
                Lane::new()
                    .children(held)
                    .children(suggestions)
                    .children(notices)
                    .children(panel)
                    .children(queue)
                    .child(
                        InputFrame::new()
                            .roomy(hero)
                            .file_drop_group(DROP_GROUP)
                            .children(tiles)
                            .child(body)
                            .children(handoff),
                    )
                    .child(footer),
            )
    }
}

#[cfg(test)]
mod ui_tests;
